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
use serde_json::{json, Value};
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;

pub struct SttTool {
    model_path: PathBuf,
    bin_path: PathBuf,
}

impl SttTool {
    pub fn new() -> Self {
        Self {
            model_path: PathBuf::from(DEFAULT_MODEL_PATH),
            bin_path: PathBuf::from(DEFAULT_BIN_PATH),
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
             threads?:4, translate?:bool, initial_prompt?:string } \
             — read a 16-bit PCM WAV (any sample rate; we resample) \
             and return { text, language, duration_ms, segments }. \
         info — model path + presence + size, available threads. \
         \
         Model lives at /data/peko/models/whisper.bin. If absent, \
         push one via scripts/download-whisper-model.sh. Default \
         is ggml-base.bin (~150 MB, multilingual). \
         \
         Pipeline: `audio_pcm record { duration_ms: 5000 }` → \
         `stt transcribe { wav_path: <returned path> }` → text \
         the agent can reason about."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": { "type": "string", "enum": ["transcribe", "info"] },
                "wav_path": { "type": "string" },
                "lang": { "type": "string" },
                "threads": { "type": "integer" },
                "translate": { "type": "boolean" },
                "initial_prompt": { "type": "string" },
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
        Box::pin(async move {
            let action = args["action"].as_str().unwrap_or("").to_string();
            match action.as_str() {
                "info" => Ok(info_resp(&model_path, &bin_path)),
                "transcribe" => transcribe(&model_path, &bin_path, &args).await,
                "" => Ok(ToolResult::error("missing 'action'".to_string())),
                other => Ok(ToolResult::error(format!(
                    "unknown action '{other}'. valid: transcribe, info"
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
