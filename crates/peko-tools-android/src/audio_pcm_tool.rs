//! Phase 5 — PCM record / playback / TTS tool.
//!
//! Bridges peko-agent (root) ↔ AudioRecord / AudioTrack / TextToSpeech
//! (Java APIs in the PekoOverlay priv-app, see `AudioBridgeService.kt`).
//! Talks via files in the priv-app's private storage:
//!
//!   /data/data/com.peko.overlay/files/audio/in/<id>.{json,wav,start}
//!   /data/data/com.peko.overlay/files/audio/out/<id>.{json,wav,done}
//!
//! Why files: stock Android's audioserver owns `/dev/snd/pcmC*D*c`, so
//! direct ALSA from Rust gets EBUSY. AudioRecord / AudioTrack speak to
//! audioserver via binder — only an Android app context can. The
//! overlay APK is already a priv-app and already running; piggybacking
//! a file-based RPC on its private files dir is the simplest path that
//! survives SELinux without policy edits.
//!
//! peko-agent runs as root and can read/write anywhere on the
//! filesystem, so the priv-app private dir is fully accessible from the
//! Rust side.

use peko_core::tool::{Tool, ToolResult};
use serde_json::json;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::time::Duration;

const APP_FILES_DIR: &str = "/data/data/com.peko.overlay/files/audio";
const DEFAULT_TIMEOUT_SECS: u64 = 30;
const MAX_TIMEOUT_SECS: u64 = 130; // > max record duration cap (120s)

pub struct AudioPcmTool {
    bridge_root: PathBuf,
}

impl AudioPcmTool {
    pub fn new() -> Self {
        Self { bridge_root: PathBuf::from(APP_FILES_DIR) }
    }

    #[cfg(test)]
    pub fn with_root(root: PathBuf) -> Self {
        Self { bridge_root: root }
    }
}

impl Default for AudioPcmTool {
    fn default() -> Self { Self::new() }
}

impl Tool for AudioPcmTool {
    fn name(&self) -> &str { "audio_pcm" }

    fn description(&self) -> &str {
        "Record from the microphone, play a WAV through the speaker, or \
         render text-to-speech via the on-device Android TextToSpeech \
         engine. Bridges to AudioRecord / AudioTrack / TextToSpeech in \
         the PekoOverlay priv-app (audioserver owns the kernel ALSA \
         nodes, so we can't open them directly). \
         \
         Actions: \
         record { duration_ms, sample_rate?:16000, channels?:1, source? } \
             — one-shot record, returns wav_path + size. \
         play_wav { wav_path } — plays a 16-bit PCM WAV. \
         tts { text, lang?:\"en\", rate?, pitch? } — synthesises speech \
             to a WAV, then plays it. \
         route_get — returns { mode, speaker, bluetooth_sco, \
             wired_headset_on, music_active, volume_music, volume_voice_call }. \
         route_set { mode?:\"normal\"|\"in_call\"|\"in_communication\"|\"ringtone\", \
             speaker?:bool, bluetooth_sco?:bool } — switches the audio \
             route at the AudioManager level. \
         start_ambient { stream_id?, sample_rate?:16000, window_ms?:1000, \
             min_rms?:0 } — begin continuous ambient capture; per-window \
             features (rms, peak, zero-crossing rate) flow into the \
             events store as type=\"ambient\". Poll via the `events` tool. \
         stop_ambient { stream_id }. \
         \
         Output WAV files land at /data/peko/audio/<id>.wav for record + tts."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "action": { "type": "string", "enum": ["record", "play_wav", "tts", "route_get", "route_set", "start_ambient", "stop_ambient"] },
                "duration_ms": { "type": "integer" },
                "sample_rate": { "type": "integer" },
                "channels": { "type": "integer" },
                "source": { "type": "string" },
                "wav_path": { "type": "string" },
                "text": { "type": "string" },
                "lang": { "type": "string" },
                "rate": { "type": "number" },
                "pitch": { "type": "number" },
                "timeout_secs": { "type": "integer" }
            },
            "required": ["action"]
        })
    }

    fn execute(
        &self,
        args: serde_json::Value,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<ToolResult>> + Send + '_>> {
        let root = self.bridge_root.clone();
        Box::pin(async move {
            let action = args["action"].as_str().unwrap_or("").to_string();
            match action.as_str() {
                "record" => record(&root, &args).await,
                "play_wav" => play_wav(&root, &args).await,
                "tts" => tts(&root, &args).await,
                "route_get" | "route_set" | "start_ambient" | "stop_ambient" =>
                    forward_simple(&args).await,
                "" => Ok(ToolResult::error("missing 'action'".to_string())),
                other => Ok(ToolResult::error(format!(
                    "unknown action '{other}'."
                ))),
            }
        })
    }
}

// ───── actions ─────────────────────────────────────────────────────

/// Phase 23 — forward route + ambient actions through the shared
/// bridge client. These don't return assets; just JSON.
async fn forward_simple(args: &serde_json::Value) -> anyhow::Result<ToolResult> {
    use crate::bridge_client::{pick_timeout, send, BridgeRequest};
    let timeout = pick_timeout(args, 10);
    let resp = send(BridgeRequest {
        topic: "audio",
        body: args.clone(),
        input_asset: None,
        input_asset_ext: "bin",
        timeout,
    }).await?;
    if !resp.json["ok"].as_bool().unwrap_or(false) {
        return Ok(ToolResult::error(format!(
            "audio_pcm {} failed: {}",
            args["action"].as_str().unwrap_or("?"),
            resp.json["error"].as_str().unwrap_or("(no error)")
        )));
    }
    Ok(ToolResult::success(format!(
        "🎚 {}\n\n{}", args["action"].as_str().unwrap_or("?"),
        serde_json::to_string_pretty(&resp.json).unwrap_or_default()
    )))
}

async fn record(root: &Path, args: &serde_json::Value) -> anyhow::Result<ToolResult> {
    let duration_ms = args["duration_ms"].as_u64().unwrap_or(5000).clamp(100, 120_000);
    let mut req = json!({
        "action": "record",
        "duration_ms": duration_ms,
        "sample_rate": args["sample_rate"].as_u64().unwrap_or(16_000),
        "channels": args["channels"].as_u64().unwrap_or(1),
    });
    if let Some(src) = args["source"].as_str() {
        req["source"] = json!(src);
    }
    let timeout = pick_timeout(args, duration_ms / 1000 + 10);
    let resp = run_request(root, req, /*has_input_wav=*/ false, timeout).await?;
    if !resp.json["ok"].as_bool().unwrap_or(false) {
        return Ok(ToolResult::error(format!(
            "record failed: {}",
            resp.json["error"].as_str().unwrap_or("(no error message)")
        )));
    }
    let Some(out_wav) = resp.out_wav else {
        return Ok(ToolResult::error("record returned ok but no WAV file".to_string()));
    };
    let dest = persist_wav(&out_wav).await?;
    Ok(ToolResult::success(format!(
        "🎤 recorded {}ms ({}Hz, {}ch) → {}\nsize: {} bytes",
        resp.json["duration_ms"], resp.json["sample_rate"], resp.json["channels"],
        dest.display(), resp.json["size_bytes"],
    )))
}

async fn play_wav(root: &Path, args: &serde_json::Value) -> anyhow::Result<ToolResult> {
    let Some(path) = args["wav_path"].as_str() else {
        return Ok(ToolResult::error("missing 'wav_path'".to_string()));
    };
    let path = PathBuf::from(path);
    if !path.exists() {
        return Ok(ToolResult::error(format!("WAV not found: {}", path.display())));
    }
    let req = json!({ "action": "play_wav" });
    let timeout = pick_timeout(args, 60);
    let resp = run_request_with_input(root, req, &path, timeout).await?;
    if !resp.json["ok"].as_bool().unwrap_or(false) {
        return Ok(ToolResult::error(format!(
            "play_wav failed: {}",
            resp.json["error"].as_str().unwrap_or("(no error message)")
        )));
    }
    Ok(ToolResult::success(format!(
        "🔊 played {} ({}ms)",
        path.display(), resp.json["duration_ms"]
    )))
}

async fn tts(root: &Path, args: &serde_json::Value) -> anyhow::Result<ToolResult> {
    let text = args["text"].as_str().unwrap_or("").to_string();
    if text.trim().is_empty() {
        return Ok(ToolResult::error("missing 'text'".to_string()));
    }
    let mut req = json!({
        "action": "tts",
        "text": text,
        "lang": args["lang"].as_str().unwrap_or("en"),
        "rate": args["rate"].as_f64().unwrap_or(1.0),
        "pitch": args["pitch"].as_f64().unwrap_or(1.0),
    });
    if let Some(extra) = args["voice"].as_str() { req["voice"] = json!(extra); }
    let timeout = pick_timeout(args, 30);
    let resp = run_request(root, req, false, timeout).await?;
    if !resp.json["ok"].as_bool().unwrap_or(false) {
        return Ok(ToolResult::error(format!(
            "tts failed: {}",
            resp.json["error"].as_str().unwrap_or("(no error message)")
        )));
    }
    let Some(out_wav) = resp.out_wav else {
        return Ok(ToolResult::error("tts returned ok but no WAV file".to_string()));
    };
    let dest = persist_wav(&out_wav).await?;
    // Auto-play the synthesised speech — the typical caller wants the
    // speech audible, not a path to a file.
    let play_req = json!({ "action": "play_wav" });
    let play_resp = run_request_with_input(root, play_req, &dest, timeout).await?;
    let played_ms = play_resp.json["duration_ms"].as_u64().unwrap_or(0);
    Ok(ToolResult::success(format!(
        "🗣  spoke ({} chars) → {}\nplayed {}ms",
        text.chars().count(), dest.display(), played_ms,
    )))
}

// ───── transport ────────────────────────────────────────────────────
//
// Phase 25 follow-up: the file-RPC plumbing (sentinel write, poll for
// .done, parse out json + asset, cleanup guard) used to live here as
// `run_request_inner`. It was a 100-line near-duplicate of
// bridge_client.rs::send. We now delegate to bridge_client and keep
// only thin shape-converting helpers + persist_wav. Single source of
// truth means future protocol tweaks don't drift between record/tts
// and gps/camera/etc.

struct BridgeResponse {
    json: serde_json::Value,
    out_wav: Option<PathBuf>,
}

async fn run_request(
    _root: &Path,
    req: serde_json::Value,
    _has_input_wav: bool,
    timeout: Duration,
) -> anyhow::Result<BridgeResponse> {
    let r = crate::bridge_client::send(crate::bridge_client::BridgeRequest {
        topic: "audio",
        body: req,
        input_asset: None,
        input_asset_ext: "wav",
        timeout,
    }).await?;
    Ok(BridgeResponse { json: r.json, out_wav: r.asset })
}

async fn run_request_with_input(
    _root: &Path,
    req: serde_json::Value,
    input_wav: &Path,
    timeout: Duration,
) -> anyhow::Result<BridgeResponse> {
    let r = crate::bridge_client::send(crate::bridge_client::BridgeRequest {
        topic: "audio",
        body: req,
        input_asset: Some(input_wav),
        input_asset_ext: "wav",
        timeout,
    }).await?;
    Ok(BridgeResponse { json: r.json, out_wav: r.asset })
}

fn pick_timeout(args: &serde_json::Value, default_secs: u64) -> Duration {
    let secs = args["timeout_secs"]
        .as_u64()
        .unwrap_or(default_secs.max(DEFAULT_TIMEOUT_SECS))
        .min(MAX_TIMEOUT_SECS);
    Duration::from_secs(secs)
}

async fn persist_wav(src: &Path) -> anyhow::Result<PathBuf> {
    crate::bridge_client::persist_asset(src, "audio").await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_includes_three_actions() {
        let t = AudioPcmTool::new();
        let s = t.parameters_schema();
        let actions: Vec<&str> = s["properties"]["action"]["enum"]
            .as_array().unwrap().iter().filter_map(|v| v.as_str()).collect();
        for a in ["record", "play_wav", "tts"] {
            assert!(actions.contains(&a), "missing action {a}");
        }
    }

    #[tokio::test]
    async fn bridge_handles_timeout_when_no_service() {
        let dir = std::env::temp_dir().join(format!("audio-bridge-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let tool = AudioPcmTool::with_root(dir.clone());
        let res = tool.execute(json!({
            "action": "tts",
            "text": "hello",
            "timeout_secs": 1,
        })).await;
        assert!(res.is_err() || res.as_ref().unwrap().is_error,
            "should error when no service present");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
