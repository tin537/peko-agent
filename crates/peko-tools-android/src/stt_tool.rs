//! Phase 25 — offline STT agent tool. Wraps `peko_stt::Engine` so the
//! agent can transcribe a WAV file (typically produced by `audio_pcm
//! record`) without any cloud round-trip.
//!
//! Model loading is lazy + memoised: first transcribe call opens the
//! model from `/data/peko/models/whisper.bin` and holds it in RAM.
//! Subsequent calls reuse the warm context. If the model file is
//! missing, the tool returns a clear "push the model" error rather
//! than crashing the registry.

use peko_core::tool::{Tool, ToolResult};
use peko_stt::{Engine, TranscribeOpts, DEFAULT_BIN_PATH, DEFAULT_MODEL_PATH};
use rusqlite::{params, Connection};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use tokio::sync::Mutex;
use tokio::task::JoinHandle;

/// Active streaming-STT pipelines keyed by stream_id. Each pipeline
/// loops record→transcribe and writes events into the priv-app's
/// events.db with type="transcript".
type StreamRegistry = Arc<Mutex<HashMap<String, StreamHandle>>>;

struct StreamHandle {
    /// Cooperative cancel flag. Worker loop checks it between iters.
    running: Arc<AtomicBool>,
    /// JoinHandle so stop_streaming can wait for clean shutdown.
    task: JoinHandle<()>,
    /// Total transcripts emitted so far; surfaced via streaming_status.
    transcripts_emitted: Arc<AtomicU64>,
    started_at_ms: i64,
}

pub struct SttTool {
    model_path: PathBuf,
    bin_path: PathBuf,
    streams: StreamRegistry,
}

impl SttTool {
    pub fn new() -> Self {
        Self {
            model_path: PathBuf::from(DEFAULT_MODEL_PATH),
            bin_path: PathBuf::from(DEFAULT_BIN_PATH),
            streams: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}


impl Default for SttTool {
    fn default() -> Self { Self::new() }
}

impl Tool for SttTool {
    fn name(&self) -> &str { "stt" }

    fn description(&self) -> &str {
        "Offline speech-to-text via whisper.cpp. Multilingual model \
         handles Thai + English natively, including code-switching. \
         No cloud round-trip — runs entirely on the device. \
         \
         Actions: \
         transcribe { wav_path, lang?:\"auto\"|\"th\"|\"en\"|..., \
             threads?, translate?, initial_prompt?, timeout_secs? } \
             — read a 16-bit PCM WAV (any sample rate; we resample) \
             and return { text, language, duration_ms, segments }. \
         start_streaming { stream_id?, chunk_secs?:5, lang?, max_chunks?:0 } \
             — kick off a background loop that records `chunk_secs` \
             at a time and pushes each transcript into the shared \
             events.db as type=\"transcript\". Returns { stream_id }; \
             agent polls via the `events` tool with type=\"transcript\". \
             max_chunks=0 = unbounded. \
         stop_streaming { stream_id } — cooperative cancel; finishes \
             current chunk then exits. \
         streaming_status — list active streams (id, started, count). \
         info — model path + presence + size, available threads. \
         \
         Model lives at /data/peko/models/whisper.bin. If absent, \
         push one via scripts/download-whisper-model.sh."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": { "type": "string", "enum": [
                    "transcribe", "info",
                    "start_streaming", "stop_streaming", "streaming_status"
                ] },
                "wav_path": { "type": "string" },
                "lang": { "type": "string" },
                "threads": { "type": "integer" },
                "translate": { "type": "boolean" },
                "initial_prompt": { "type": "string" },
                "timeout_secs": { "type": "integer" },
                "stream_id": { "type": "string" },
                "chunk_secs": { "type": "integer" },
                "max_chunks": { "type": "integer" },
            },
            "required": ["action"]
        })
    }

    fn execute(
        &self,
        args: Value,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<ToolResult>> + Send + '_>> {
        let model_path = self.model_path.clone();
        let bin_path = self.bin_path.clone();
        let streams = self.streams.clone();
        Box::pin(async move {
            let action = args["action"].as_str().unwrap_or("").to_string();
            match action.as_str() {
                "info" => Ok(info_resp(&model_path, &bin_path)),
                "transcribe" => transcribe(&model_path, &bin_path, &args).await,
                "start_streaming" => start_streaming(streams, model_path, &args).await,
                "stop_streaming" => stop_streaming(streams, &args).await,
                "streaming_status" => streaming_status(streams).await,
                "" => Ok(ToolResult::error("missing 'action'".to_string())),
                other => Ok(ToolResult::error(format!(
                    "unknown action '{other}'. valid: transcribe, start_streaming, \
                     stop_streaming, streaming_status, info"
                ))),
            }
        })
    }
}

fn info_resp(model_path: &std::path::Path, _bin_hint: &std::path::Path) -> ToolResult {
    let model_present = model_path.exists();
    let resolved_bin = peko_stt::discover_bin();
    let bin_present = resolved_bin.is_some();
    let bin_path_str = resolved_bin
        .as_ref()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| format!("(none of {:?})", peko_stt::BIN_SEARCH_PATHS));
    let body = json!({
        "engine": "whisper.cpp (whisper-cli shell-out)",
        "model_path": model_path.display().to_string(),
        "model_present": model_present,
        "model_size_bytes": std::fs::metadata(model_path).map(|m| m.len()).unwrap_or(0),
        "bin_path": bin_path_str,
        "bin_present": bin_present,
        "available_threads": std::thread::available_parallelism()
            .map(|n| n.get()).unwrap_or(4),
        "supported_langs": [
            "auto", "en", "th", "ja", "zh", "es", "fr", "de", "ru", "ko", "vi"
        ],
    });
    ToolResult::success(format!(
        "🎙 STT info\n\n{}", serde_json::to_string_pretty(&body).unwrap_or_default()
    ))
}

async fn transcribe(
    model_path: &std::path::Path,
    bin_path: &std::path::Path,
    args: &Value,
) -> anyhow::Result<ToolResult> {
    let Some(wav_path) = args["wav_path"].as_str() else {
        return Ok(ToolResult::error("missing 'wav_path'".to_string()));
    };
    let wav_path_buf = PathBuf::from(wav_path);
    if !wav_path_buf.exists() {
        return Ok(ToolResult::error(format!("WAV not found: {wav_path}")));
    }

    let mut opts = TranscribeOpts::default();
    if let Some(l) = args["lang"].as_str() { opts.language = l.to_string(); }
    if let Some(t) = args["threads"].as_u64() { opts.threads = (t as usize).clamp(1, 16); }
    if let Some(b) = args["translate"].as_bool() { opts.translate = b; }
    if let Some(p) = args["initial_prompt"].as_str() {
        opts.initial_prompt = Some(p.to_string());
    }
    if let Some(t) = args["timeout_secs"].as_u64() { opts.timeout_secs = t.clamp(5, 600); }

    // Pass None for bin_path so peko-stt's discover_bin() walks the
    // search list (covers /system/bin, Magisk module dir, /data/local/tmp).
    let _ = bin_path; // SttTool's stored bin_path is informational only.
    let engine = match Engine::open(Some(model_path), None) {
        Ok(e) => e,
        Err(e) => return Ok(ToolResult::error(format!("STT engine: {e}"))),
    };
    let transcript = match engine.transcribe(&wav_path_buf, &opts).await {
        Ok(t) => t,
        Err(e) => return Ok(ToolResult::error(format!("STT failed: {e}"))),
    };

    let preview: String = transcript.text.chars().take(160).collect();
    let summary = format!(
        "🎙 transcribed in {}ms (lang={}, {} segments)\n\n{preview}{}",
        transcript.duration_ms,
        transcript.language,
        transcript.segments.len(),
        if transcript.text.chars().count() > 160 { "…" } else { "" },
    );
    let json_body = serde_json::to_string_pretty(&transcript).unwrap_or_default();
    Ok(ToolResult::success(format!("{summary}\n\n{json_body}")))
}

// ────── streaming actions ─────────────────────────────────────────

async fn start_streaming(
    streams: StreamRegistry,
    model_path: PathBuf,
    args: &Value,
) -> anyhow::Result<ToolResult> {
    use crate::bridge_client::{pick_timeout, send, BridgeRequest};
    let stream_id = args["stream_id"].as_str()
        .filter(|s| valid_stream_id(s))
        .map(String::from)
        .unwrap_or_else(|| format!("stt-{}", std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos()).unwrap_or(0)));
    let chunk_secs = args["chunk_secs"].as_u64().unwrap_or(5).clamp(2, 60);
    let max_chunks = args["max_chunks"].as_u64().unwrap_or(0);
    let lang = args["lang"].as_str().unwrap_or("auto").to_string();

    {
        let map = streams.lock().await;
        if map.contains_key(&stream_id) {
            return Ok(ToolResult::error(format!(
                "streaming-stt id '{stream_id}' already running; stop_streaming first"
            )));
        }
    }

    let running = Arc::new(AtomicBool::new(true));
    let counter = Arc::new(AtomicU64::new(0));

    // Worker: record → transcribe → write event → repeat. The agent's
    // existing `audio_pcm record` action is reachable through the same
    // bridge_client we use elsewhere; we don't go through ToolRegistry
    // since the streaming task lives outside any ReAct loop.
    let running_w = running.clone();
    let counter_w = counter.clone();
    let stream_id_w = stream_id.clone();
    let model_path_w = model_path.clone();
    let lang_for_msg = lang.clone();
    let task = tokio::spawn(async move {
        let mut chunk_n: u64 = 0;
        // Phase 25c min-viable streaming: carry the tail of the prior
        // transcript as `initial_prompt` for the next chunk. Whisper
        // uses it as decoder context, which acts like a soft overlap
        // and dramatically improves accuracy on word boundaries
        // straddling a chunk split (e.g. "I want to" / "go home" → no
        // longer split into hallucinated halves). Capped at ~200 chars
        // because whisper's prompt token limit is small.
        let mut last_tail: Option<String> = None;
        while running_w.load(Ordering::Relaxed) {
            chunk_n += 1;
            if max_chunks > 0 && chunk_n > max_chunks {
                tracing::info!(stream_id = %stream_id_w, "stt stream hit max_chunks, exiting");
                break;
            }
            // 1) Record a chunk via the audio bridge.
            let rec_resp = match send(BridgeRequest {
                topic: "audio",
                body: json!({
                    "action": "record",
                    "duration_ms": chunk_secs * 1000,
                    "sample_rate": 16_000,
                    "channels": 1,
                }),
                input_asset: None,
                input_asset_ext: "bin",
                timeout: pick_timeout(&Value::Null, chunk_secs + 10),
            }).await {
                Ok(r) => r,
                Err(e) => {
                    tracing::warn!(stream_id = %stream_id_w, error = %e, "stt stream: record failed");
                    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                    continue;
                }
            };
            if !rec_resp.json["ok"].as_bool().unwrap_or(false) {
                tracing::warn!(
                    stream_id = %stream_id_w,
                    err = ?rec_resp.json["error"],
                    "stt stream: record reported failure"
                );
                tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                continue;
            }
            let Some(wav_src) = rec_resp.asset.clone() else {
                tracing::warn!(stream_id = %stream_id_w, "stt stream: record returned no asset");
                continue;
            };

            // 2) Transcribe.
            let engine = match Engine::open(Some(&model_path_w), None) {
                Ok(e) => e,
                Err(e) => {
                    tracing::error!(error = %e, "stt stream: engine open; aborting");
                    break;
                }
            };
            let mut opts = TranscribeOpts::default();
            opts.language = lang.clone();
            opts.initial_prompt = last_tail.clone();
            let transcript = match engine.transcribe(&wav_src, &opts).await {
                Ok(t) => t,
                Err(e) => {
                    tracing::warn!(stream_id = %stream_id_w, error = %e, "stt stream: transcribe failed");
                    continue;
                }
            };

            // 3) Skip empty / silence-only transcripts so the agent's
            // event poller isn't flooded with noise.
            if transcript.text.trim().is_empty() {
                continue;
            }

            // 4) Append to events.db as type="transcript".
            if let Err(e) = append_transcript_event(&stream_id_w, &transcript, chunk_secs).await {
                tracing::warn!(stream_id = %stream_id_w, error = %e, "stt stream: events.db write failed");
            }
            counter_w.fetch_add(1, Ordering::Relaxed);

            // Carry tail forward (last ~200 chars) for the next chunk's
            // initial_prompt — see comment at start of loop.
            let count = transcript.text.chars().count();
            let skip = count.saturating_sub(200);
            last_tail = Some(transcript.text.chars().skip(skip).collect::<String>());
        }
        tracing::info!(stream_id = %stream_id_w, "stt stream exited");
    });

    let started_at_ms = chrono::Utc::now().timestamp_millis();
    streams.lock().await.insert(stream_id.clone(), StreamHandle {
        running, task, transcripts_emitted: counter, started_at_ms,
    });

    Ok(ToolResult::success(format!(
        "🎙▶ stt streaming started — id `{stream_id}` (chunk {chunk_secs}s, lang={lang_for_msg}). \
         Poll via events tool: type=\"transcript\". \
         Stop with stt {{action:\"stop_streaming\",stream_id:\"{stream_id}\"}}."
    )))
}

async fn stop_streaming(streams: StreamRegistry, args: &Value) -> anyhow::Result<ToolResult> {
    let Some(stream_id) = args["stream_id"].as_str() else {
        return Ok(ToolResult::error("missing 'stream_id'".to_string()));
    };
    let handle_opt = streams.lock().await.remove(stream_id);
    let Some(handle) = handle_opt else {
        return Ok(ToolResult::error(format!("no active stt stream '{stream_id}'")));
    };
    handle.running.store(false, Ordering::Relaxed);
    // Don't await join — current chunk could be 60s; agent shouldn't block.
    handle.task.abort();
    let n = handle.transcripts_emitted.load(Ordering::Relaxed);
    let elapsed_s = ((chrono::Utc::now().timestamp_millis() - handle.started_at_ms) / 1000).max(0);
    Ok(ToolResult::success(format!(
        "🛑 stt stream `{stream_id}` stopped — emitted {n} transcript(s) over {elapsed_s}s"
    )))
}

async fn streaming_status(streams: StreamRegistry) -> anyhow::Result<ToolResult> {
    let map = streams.lock().await;
    if map.is_empty() {
        return Ok(ToolResult::success("No active stt streams.".to_string()));
    }
    let now_ms = chrono::Utc::now().timestamp_millis();
    let mut out = format!("{} active stt stream(s):\n", map.len());
    for (id, h) in map.iter() {
        let n = h.transcripts_emitted.load(Ordering::Relaxed);
        let elapsed_s = ((now_ms - h.started_at_ms) / 1000).max(0);
        out.push_str(&format!("  • {id} — {n} transcript(s), {elapsed_s}s elapsed\n"));
    }
    Ok(ToolResult::success(out))
}

fn valid_stream_id(s: &str) -> bool {
    !s.is_empty() && s.len() <= 64
        && s.chars().all(|c| c.is_alphanumeric() || c == '-' || c == '_')
}

/// Append a transcript event into the priv-app's shared events.db so
/// the agent's `events` tool can poll it. peko-agent runs as root so
/// it can open the priv-app's database directly.
async fn append_transcript_event(
    stream_id: &str,
    transcript: &peko_stt::Transcript,
    chunk_secs: u64,
) -> anyhow::Result<()> {
    let db_path = crate::bridge_client::events_db_path();
    let stream_id = stream_id.to_string();
    let data = json!({
        "text": transcript.text,
        "language": transcript.language,
        "duration_ms": transcript.duration_ms,
        "segments": transcript.segments.len(),
        "chunk_secs": chunk_secs,
        "model_path": transcript.model_path,
    });
    // SQLite calls are synchronous; run on a blocking pool so the
    // worker loop's tokio runtime isn't stalled.
    tokio::task::spawn_blocking(move || -> anyhow::Result<()> {
        let conn = Connection::open(&db_path)?;
        let now_ms = chrono::Utc::now().timestamp_millis();
        conn.execute(
            "INSERT INTO events (ts, type, source, data_json, asset_path) \
             VALUES (?1, 'transcript', ?2, ?3, NULL)",
            params![now_ms, format!("stt_stream:{stream_id}"), data.to_string()],
        )?;
        Ok(())
    })
    .await
    .map_err(|e| anyhow::anyhow!("spawn_blocking: {e}"))??;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_lists_actions() {
        let t = SttTool::new();
        let s = t.parameters_schema();
        let actions: Vec<&str> = s["properties"]["action"]["enum"]
            .as_array().unwrap().iter().filter_map(|v| v.as_str()).collect();
        assert!(actions.contains(&"transcribe"));
        assert!(actions.contains(&"info"));
    }

    #[tokio::test]
    async fn info_action_works_without_model() {
        let t = SttTool::new();
        let r = t.execute(json!({"action":"info"})).await.unwrap();
        assert!(!r.is_error);
        assert!(r.content.contains("whisper.cpp"));
    }

    #[tokio::test]
    async fn transcribe_missing_wav_errors_clearly() {
        let t = SttTool::new();
        let r = t.execute(json!({
            "action":"transcribe","wav_path":"/nonexistent/file.wav"
        })).await.unwrap();
        assert!(r.is_error);
        assert!(r.content.contains("WAV not found"));
    }
}
