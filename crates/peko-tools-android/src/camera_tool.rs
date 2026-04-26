//! Phase 23 — Camera tool. One-shot capture + low-FPS streaming via
//! CameraBridgeService in the priv-app.

use crate::bridge_client::{persist_asset, pick_timeout, send, BridgeRequest};
use peko_core::tool::{Tool, ToolResult};
use serde_json::{json, Value};
use std::future::Future;
use std::pin::Pin;

pub struct CameraTool;
impl CameraTool { pub fn new() -> Self { Self } }
impl Default for CameraTool { fn default() -> Self { Self::new() } }

impl Tool for CameraTool {
    fn name(&self) -> &str { "camera" }

    fn description(&self) -> &str {
        "Capture from the device camera via Camera2. Three actions: \
         capture { lens?:\"back\"|\"front\", resolution?:\"720p\"|\
             \"1080p\"|\"max\" } — take one JPEG, returns its path. \
         start_stream { stream_id?, lens?:\"back\", fps?:1, \
             resolution?:\"720p\", max_frames?:0 } — begin continuous \
             capture. Each frame lands as type=\"frame\" in the events \
             store with a per-frame asset_path. fps capped at 5 to \
             keep storage sane. \
         stop_stream { stream_id } — release the camera. \
         \
         For continuous frames, poll the `events` tool with \
         type=\"frame\". Frames are 16:9 JPEGs at the requested \
         resolution; pass them to the vision LLM or OCR tool for \
         downstream interpretation. Single camera at a time — capture \
         while a stream is running returns an error."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": { "type": "string", "enum": ["capture", "start_stream", "stop_stream"] },
                "lens": { "type": "string" },
                "resolution": { "type": "string" },
                "stream_id": { "type": "string" },
                "fps": { "type": "number" },
                "max_frames": { "type": "integer" },
                "timeout_secs": { "type": "integer" }
            },
            "required": ["action"]
        })
    }

    fn execute(
        &self,
        args: Value,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<ToolResult>> + Send + '_>> {
        Box::pin(async move {
            let action = args["action"].as_str().unwrap_or("").to_string();
            let timeout = pick_timeout(&args, match action.as_str() { "capture" => 20, _ => 10 });
            let resp = send(BridgeRequest {
                topic: "camera",
                body: args.clone(),
                input_asset: None,
                input_asset_ext: "bin",
                timeout,
            }).await?;
            if !resp.json["ok"].as_bool().unwrap_or(false) {
                return Ok(ToolResult::error(format!(
                    "camera {action} failed: {}",
                    resp.json["error"].as_str().unwrap_or("(no error)"),
                )));
            }
            match action.as_str() {
                "capture" => {
                    let Some(asset) = resp.asset else {
                        return Ok(ToolResult::error("capture ok but no JPEG produced".to_string()));
                    };
                    let dest = persist_asset(&asset, "camera").await?;
                    Ok(ToolResult::success(format!(
                        "📷 captured {}×{} ({} bytes) → {}",
                        resp.json["width"], resp.json["height"],
                        resp.json["size_bytes"], dest.display(),
                    )))
                }
                "start_stream" => Ok(ToolResult::success(format!(
                    "📹 camera stream `{}` started ({}×{} @ {} fps). \
                     Poll via events tool: type=\"frame\".",
                    resp.json["stream_id"].as_str().unwrap_or("?"),
                    resp.json["width"], resp.json["height"],
                    resp.json["fps"].as_f64().unwrap_or(0.0),
                ))),
                "stop_stream" => Ok(ToolResult::success(format!(
                    "🛑 camera stream `{}` stopped.",
                    resp.json["stream_id"].as_str().unwrap_or("?")))),
                _ => Ok(ToolResult::success(resp.json.to_string())),
            }
        })
    }
}
