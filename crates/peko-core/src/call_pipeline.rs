//! Voice-call pipeline.
//!
//! Bridges the Android shim's CallRecorderService to peko's memory
//! store. The shim writes `<id>.m4a`, `<id>.json`, and a `<id>.done`
//! sentinel into its private `files/calls/` dir whenever a phone call
//! completes; this module polls the dir, uploads the audio to an
//! OpenAI-compatible `/audio/transcriptions` endpoint, asks the
//! existing cloud brain to summarise the transcript, and persists
//! the result in both:
//!
//!   - a `calls` table (detailed per-call record in `calls.db`)
//!   - the main memory store (one `Observation` entry per call, so
//!     the agent "remembers" what was discussed and can bring it up
//!     in later conversations — this is the user's ask: "peko can
//!     receive a call phone voice for summary call to user after
//!     call end")
//!
//! Opt-in via `[calls].enabled=true` in config. The shim side stops
//! producing recordings if the user hasn't held `RECORD_AUDIO` at the
//! Android level — see magisk/peko-module/service.sh.
//!
//! ## Why not use the agent's tool framework?
//!
//! Tools are invoked *by the model* from inside a session. This
//! pipeline runs independently of any session — a call coming in
//! while peko-agent is idle should still be captured. Better modelled
//! as a dedicated daemon task. The summary it produces eventually
//! reaches the model via the memory store anyway.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use futures::StreamExt;
use reqwest::multipart::{Form, Part};
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;
use tokio::task::JoinHandle;
use tracing::{info, warn};

use peko_config::CallsConfig;
use peko_transport::LlmProvider;
use peko_transport::provider::{Message as TransportMessage, MessageContent};
use peko_transport::StreamEvent;

use crate::memory::{MemoryCategory, MemoryStore};

/// Polling interval for the recordings dir. We don't use inotify —
/// the dir lives inside the shim's sandboxed /data/data/ tree, and
/// running inotify_init1() as root across that namespace is fragile
/// (selinux labels + filesystem mounts differ). A 10-second poll is
/// plenty — a call takes 30s+ typically, and there's no latency
/// requirement on "when does the summary show up" anyway.
const POLL_INTERVAL_SECS: u64 = 10;

/// Per-call metadata, shape-compatible with what CallRecorderService
/// writes in `<id>.json`. Extra fields are tolerated by serde.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CallMetadata {
    pub id: String,
    pub direction: String,
    #[serde(default)]
    pub number: String,
    pub started_at_ms: i64,
    pub duration_ms: i64,
    #[serde(default)]
    pub audio_src: String,
    pub audio_path: String,
    #[serde(default)]
    pub audio_bytes: i64,
    #[serde(default)]
    pub partial: bool,
    #[serde(default)]
    pub error: Option<String>,
}

/// Full call record as stored in the calls table and returned by
/// /api/calls. Builds on CallMetadata and adds what the pipeline
/// produced (transcript, summary, state).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CallRecord {
    pub id: String,
    pub direction: String,
    pub number: String,
    pub started_at: String,
    pub duration_ms: i64,
    pub audio_src: String,
    pub transcript: Option<String>,
    pub summary: Option<String>,
    /// "recorded" → waiting for STT
    /// "transcribed" → transcript available, awaiting summary
    /// "summarised" → summary stored in memory
    /// "skipped" → too short or STT unavailable
    /// "error" → see `error` field for details
    pub state: String,
    pub error: Option<String>,
    pub created_at: String,
}

/// SQLite-backed store for call records. Kept separate from the
/// shared memory.db so the CLI/UI can list calls chronologically
/// without having to FTS-scan the memory table.
pub struct CallStore {
    conn: Connection,
}

impl CallStore {
    pub fn open(path: &Path) -> anyhow::Result<Self> {
        let conn = Connection::open(path)?;
        let s = Self { conn };
        s.init_schema()?;
        Ok(s)
    }

    fn init_schema(&self) -> anyhow::Result<()> {
        self.conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS calls (
                id TEXT PRIMARY KEY,
                direction TEXT NOT NULL,
                number TEXT NOT NULL DEFAULT '',
                started_at TEXT NOT NULL,
                duration_ms INTEGER NOT NULL DEFAULT 0,
                audio_src TEXT NOT NULL DEFAULT '',
                transcript TEXT,
                summary TEXT,
                state TEXT NOT NULL DEFAULT 'recorded',
                error TEXT,
                created_at TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_calls_started ON calls(started_at DESC);
            CREATE INDEX IF NOT EXISTS idx_calls_number  ON calls(number);"
        )?;
        Ok(())
    }

    pub fn exists(&self, id: &str) -> anyhow::Result<bool> {
        let c: i64 = self.conn.query_row(
            "SELECT COUNT(1) FROM calls WHERE id = ?1",
            params![id],
            |row| row.get(0),
        )?;
        Ok(c > 0)
    }

    pub fn insert(&self, rec: &CallRecord) -> anyhow::Result<()> {
        self.conn.execute(
            "INSERT OR REPLACE INTO calls
               (id, direction, number, started_at, duration_ms, audio_src,
                transcript, summary, state, error, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            params![
                rec.id, rec.direction, rec.number, rec.started_at, rec.duration_ms,
                rec.audio_src, rec.transcript, rec.summary, rec.state, rec.error,
                rec.created_at,
            ],
        )?;
        Ok(())
    }

    pub fn update_transcript(&self, id: &str, transcript: &str) -> anyhow::Result<()> {
        self.conn.execute(
            "UPDATE calls SET transcript = ?1, state = 'transcribed' WHERE id = ?2",
            params![transcript, id],
        )?;
        Ok(())
    }

    pub fn update_summary(&self, id: &str, summary: &str) -> anyhow::Result<()> {
        self.conn.execute(
            "UPDATE calls SET summary = ?1, state = 'summarised' WHERE id = ?2",
            params![summary, id],
        )?;
        Ok(())
    }

    pub fn update_error(&self, id: &str, state: &str, err: &str) -> anyhow::Result<()> {
        self.conn.execute(
            "UPDATE calls SET state = ?1, error = ?2 WHERE id = ?3",
            params![state, err, id],
        )?;
        Ok(())
    }

    pub fn recent(&self, limit: usize) -> anyhow::Result<Vec<CallRecord>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, direction, number, started_at, duration_ms, audio_src,
                    transcript, summary, state, error, created_at
             FROM calls ORDER BY started_at DESC LIMIT ?1"
        )?;
        let rows = stmt.query_map(params![limit as i64], |row| {
            Ok(CallRecord {
                id:          row.get(0)?,
                direction:   row.get(1)?,
                number:      row.get(2)?,
                started_at:  row.get(3)?,
                duration_ms: row.get(4)?,
                audio_src:   row.get(5)?,
                transcript:  row.get(6)?,
                summary:     row.get(7)?,
                state:       row.get(8)?,
                error:       row.get(9)?,
                created_at:  row.get(10)?,
            })
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }
}

/// Spawn the watcher loop. Returns the handle so main.rs can
/// `.await` or drop it at shutdown. Pattern mirrors
/// `gardener::spawn`.
pub fn spawn(
    cfg: CallsConfig,
    call_store: Arc<Mutex<CallStore>>,
    memory_store: Arc<Mutex<MemoryStore>>,
    provider: Option<Arc<dyn LlmProvider>>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        info!(
            recordings_dir = %cfg.recordings_dir,
            stt_base_url   = %cfg.stt_base_url,
            stt_model      = %cfg.stt_model,
            "call pipeline started"
        );
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(300))
            .connect_timeout(Duration::from_secs(30))
            .build()
            .expect("failed to build reqwest Client");

        loop {
            if let Err(e) = tick(&cfg, &http, &call_store, &memory_store, provider.as_deref()).await {
                warn!(error = %e, "call pipeline tick failed");
            }
            tokio::time::sleep(Duration::from_secs(POLL_INTERVAL_SECS)).await;
        }
    })
}

async fn tick(
    cfg: &CallsConfig,
    http: &reqwest::Client,
    call_store: &Arc<Mutex<CallStore>>,
    memory_store: &Arc<Mutex<MemoryStore>>,
    provider: Option<&dyn LlmProvider>,
) -> anyhow::Result<()> {
    let dir = PathBuf::from(&cfg.recordings_dir);
    if !dir.is_dir() {
        // Recordings dir only exists after the shim has seen its
        // first call — don't spam warnings during idle periods.
        return Ok(());
    }

    let entries = std::fs::read_dir(&dir)?;
    for entry in entries.flatten() {
        let path = entry.path();
        // Only act on the sentinel — guarantees the .m4a and .json
        // pair finished flushing. CallRecorderService writes .done
        // atomically after both are closed.
        if path.extension().and_then(|e| e.to_str()) != Some("done") {
            continue;
        }
        let Some(stem) = path.file_stem().and_then(|s| s.to_str()).map(String::from) else { continue };
        let meta_path = dir.join(format!("{stem}.json"));
        let audio_path = dir.join(format!("{stem}.m4a"));
        let done_path = path;

        if let Err(e) = process_one(
            cfg, http, call_store, memory_store, provider,
            &stem, &meta_path, &audio_path, &done_path,
        ).await {
            warn!(id = %stem, error = %e, "call pipeline: processing failed");
            if let Ok(cs) = call_store.try_lock() {
                let _ = cs.update_error(&stem, "error", &format!("{e}"));
            }
            // Remove the sentinel anyway so the error is terminal —
            // leaving it would retry forever on permanent failures
            // (e.g. corrupt metadata).
            let _ = std::fs::remove_file(&done_path);
        }
    }

    Ok(())
}

async fn process_one(
    cfg: &CallsConfig,
    http: &reqwest::Client,
    call_store: &Arc<Mutex<CallStore>>,
    memory_store: &Arc<Mutex<MemoryStore>>,
    provider: Option<&dyn LlmProvider>,
    id: &str,
    meta_path: &Path,
    audio_path: &Path,
    done_path: &Path,
) -> anyhow::Result<()> {
    // Short-circuit if we've already processed this id (defence-in-
    // depth; the .done file is usually gone after success, but a
    // crash mid-process could leave a stray sentinel).
    {
        let cs = call_store.lock().await;
        if cs.exists(id)? {
            let _ = std::fs::remove_file(done_path);
            return Ok(());
        }
    }

    let meta_bytes = match std::fs::read(meta_path) {
        Ok(b) => b,
        Err(e) => {
            // Metadata missing = nothing to persist. Drop sentinel.
            let _ = std::fs::remove_file(done_path);
            return Err(anyhow::anyhow!("metadata read failed: {e}"));
        }
    };
    let meta: CallMetadata = serde_json::from_slice(&meta_bytes)
        .map_err(|e| anyhow::anyhow!("metadata parse: {e}"))?;

    let started_at = chrono::DateTime::from_timestamp_millis(meta.started_at_ms)
        .map(|d| d.to_rfc3339())
        .unwrap_or_else(|| chrono::Utc::now().to_rfc3339());

    let mut rec = CallRecord {
        id: meta.id.clone(),
        direction: meta.direction.clone(),
        number: meta.number.clone(),
        started_at,
        duration_ms: meta.duration_ms,
        audio_src: meta.audio_src.clone(),
        transcript: None,
        summary: None,
        state: "recorded".into(),
        error: meta.error.clone(),
        created_at: chrono::Utc::now().to_rfc3339(),
    };

    // Short / failed recordings: log and skip STT.
    let too_short = (meta.duration_ms as u64) < cfg.min_duration_ms;
    let bad_audio = meta.audio_bytes == 0 || !audio_path.is_file();
    if too_short || bad_audio || meta.error.is_some() {
        rec.state = "skipped".into();
        if rec.error.is_none() {
            rec.error = Some(if too_short {
                format!("too short ({} ms < {} ms)", meta.duration_ms, cfg.min_duration_ms)
            } else if bad_audio {
                "no audio captured".into()
            } else {
                "recording error".into()
            });
        }
        call_store.lock().await.insert(&rec)?;
        cleanup(audio_path, meta_path, done_path);
        info!(id = %id, state = %rec.state, "call skipped");
        return Ok(());
    }

    call_store.lock().await.insert(&rec)?;

    // ── STT ─────────────────────────────────────────────────
    let api_key = resolve_stt_key(cfg);
    if api_key.is_empty() {
        rec.state = "skipped".into();
        rec.error = Some("no STT API key configured".into());
        call_store.lock().await.update_error(id, "skipped", rec.error.as_deref().unwrap_or(""))?;
        cleanup(audio_path, meta_path, done_path);
        return Ok(());
    }

    let transcript = match transcribe(http, cfg, &api_key, audio_path).await {
        Ok(t) => t,
        Err(e) => {
            call_store.lock().await.update_error(id, "error", &format!("stt: {e}"))?;
            cleanup_keep_audio(meta_path, done_path);
            return Err(e);
        }
    };
    call_store.lock().await.update_transcript(id, &transcript)?;
    rec.transcript = Some(transcript.clone());

    info!(id = %id, len = transcript.len(), "transcript captured");

    // ── Summary ─────────────────────────────────────────────
    // Only attempt if a provider is available (DualBrain hands us
    // the cheap side by default; see main.rs). If not, we still
    // have the transcript in the DB for later.
    if let Some(provider) = provider {
        match summarise(provider, &rec, &transcript).await {
            Ok(summary) => {
                call_store.lock().await.update_summary(id, &summary)?;
                persist_to_memory(memory_store, &rec, &summary).await;
                post_notification(&rec, &summary);
                rec.summary = Some(summary);
            }
            Err(e) => {
                warn!(id = %id, error = %e, "summary failed (transcript kept)");
                call_store.lock().await.update_error(id, "transcribed", &format!("summary: {e}"))?;
            }
        }
    } else {
        warn!(id = %id, "no LLM provider available; transcript stored without summary");
    }

    cleanup(audio_path, meta_path, done_path);
    Ok(())
}

fn resolve_stt_key(cfg: &CallsConfig) -> String {
    let k = cfg.stt_api_key.clone().unwrap_or_default();
    if !k.is_empty() { return k; }
    std::env::var("OPENAI_API_KEY").unwrap_or_default()
}

async fn transcribe(
    http: &reqwest::Client,
    cfg: &CallsConfig,
    api_key: &str,
    audio_path: &Path,
) -> anyhow::Result<String> {
    let bytes = std::fs::read(audio_path)?;
    let file_name = audio_path.file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("audio.m4a")
        .to_string();

    let mut form = Form::new()
        .part("file",
              Part::bytes(bytes)
                  .file_name(file_name)
                  .mime_str("audio/mp4")?)
        .text("model", cfg.stt_model.clone())
        .text("response_format", "json".to_string());
    if let Some(lang) = cfg.stt_language.as_deref() {
        if !lang.is_empty() {
            form = form.text("language", lang.to_string());
        }
    }

    let url = format!("{}/audio/transcriptions", cfg.stt_base_url.trim_end_matches('/'));
    let resp = http.post(&url)
        .header("authorization", format!("Bearer {api_key}"))
        .multipart(form)
        .send()
        .await?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        anyhow::bail!("STT HTTP {status}: {}", truncate(&body, 400));
    }

    let body: serde_json::Value = resp.json().await?;
    let text = body.get("text").and_then(|v| v.as_str()).unwrap_or("").trim().to_string();
    if text.is_empty() {
        anyhow::bail!("STT returned empty transcript");
    }
    Ok(text)
}

async fn summarise(
    provider: &dyn LlmProvider,
    rec: &CallRecord,
    transcript: &str,
) -> anyhow::Result<String> {
    // Keep the prompt tight — the transcript dominates the token
    // budget and there's nothing clever to do other than ask for
    // the gist. Single paragraph, because the UI notification
    // reuses this text.
    let system = "You summarise phone-call transcripts for the user's memory. \
                  Output a single compact paragraph (≤80 words). Include: who \
                  called (if named), what was discussed, any action items or \
                  dates mentioned. No preamble. No markdown.";
    let user = format!(
        "Direction: {}\nOther party: {}\nDuration: {}s\n\nTranscript:\n{}",
        rec.direction,
        if rec.number.is_empty() { "unknown" } else { &rec.number },
        rec.duration_ms / 1000,
        truncate(transcript, 8000),
    );

    let mut stream = provider.stream_completion(
        system,
        &[TransportMessage { role: "user".into(), content: MessageContent::Text(user) }],
        &[],
    ).await?;

    let mut out = String::new();
    while let Some(event) = stream.next().await {
        if let Ok(StreamEvent::TextDelta(t)) = event {
            out.push_str(&t);
        }
    }
    let out = out.trim().to_string();
    if out.is_empty() {
        anyhow::bail!("LLM returned empty summary");
    }
    Ok(out)
}

async fn persist_to_memory(store: &Arc<Mutex<MemoryStore>>, rec: &CallRecord, summary: &str) {
    let caller = if rec.number.is_empty() { "unknown".to_string() } else { rec.number.clone() };
    let key = format!("call:{}:{}", caller, rec.started_at);
    let content = format!(
        "{} call with {} on {} ({}s): {}",
        rec.direction,
        caller,
        rec.started_at,
        rec.duration_ms / 1000,
        summary,
    );
    let res = {
        let s = store.lock().await;
        s.save(&key, &content, &MemoryCategory::Observation, 0.6, None)
    };
    if let Err(e) = res {
        warn!(error = %e, "memory save failed for call");
    }
}

/// Best-effort Android system notification with the summary gist.
/// We use `cmd notification post` — the shim's POST_NOTIFICATIONS
/// permission is already held via the SMS role, and running as root
/// under Magisk lets us call this from outside any app context.
fn post_notification(rec: &CallRecord, summary: &str) {
    use std::process::{Command, Stdio};
    let title = format!(
        "Call {} ({}s)",
        if rec.number.is_empty() { rec.direction.as_str() } else { rec.number.as_str() },
        rec.duration_ms / 1000,
    );
    let body = truncate(summary, 240);
    // `cmd notification post` wants a tag, icon, title, body layout
    // that's a little awkward — this is the minimal incantation that
    // actually produces a shade entry on LineageOS 20. Non-fatal on
    // failure; the summary is already in memory + the web UI.
    let res = Command::new("cmd")
        .arg("notification")
        .arg("post")
        .arg("-t").arg(&title)
        .arg("peko-call")
        .arg(&body)
        .stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null())
        .status();
    if let Err(e) = res {
        warn!(error = %e, "notification post failed");
    }
}

fn cleanup(audio: &Path, meta: &Path, done: &Path) {
    let _ = std::fs::remove_file(audio);
    let _ = std::fs::remove_file(meta);
    let _ = std::fs::remove_file(done);
}

fn cleanup_keep_audio(meta: &Path, done: &Path) {
    let _ = std::fs::remove_file(meta);
    let _ = std::fs::remove_file(done);
}

fn truncate(s: &str, n: usize) -> String {
    if s.len() <= n { return s.to_string(); }
    let mut end = n;
    while end > 0 && !s.is_char_boundary(end) { end -= 1; }
    format!("{}…", &s[..end])
}
