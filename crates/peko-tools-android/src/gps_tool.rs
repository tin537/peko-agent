//! Phase 23 — GPS tool. Bridges to LocationBridgeService in the
//! PekoOverlay priv-app via the shared file-RPC pattern.

use crate::bridge_client::{pick_timeout, send, BridgeRequest};
use peko_core::tool::{Tool, ToolResult};
use serde_json::{json, Value};
use std::future::Future;
use std::pin::Pin;

pub struct GpsTool;
impl GpsTool { pub fn new() -> Self { Self } }
impl Default for GpsTool { fn default() -> Self { Self::new() } }

impl Tool for GpsTool {
    fn name(&self) -> &str { "gps" }

    fn description(&self) -> &str {
        "Get device location via the Android LocationManager (GPS / \
         network / fused). Three actions: \
         fix { provider?:\"gps\"|\"network\"|\"fused\", \
              max_age_ms?:30000, timeout_ms?:30000 } — one-shot, \
              tries last-known first then a single live update; returns \
              { lat, lon, alt_m, accuracy_m, speed_mps, bearing_deg, \
                provider, location_ts, from_cache }. \
         start_stream { stream_id?, interval_ms?:5000, \
              min_distance_m?:0, provider?:\"gps\" } — begin continuous \
              updates; samples flow into the shared events store as \
              type=\"location\". Returns { stream_id }. \
         stop_stream { stream_id } — unregister listener. \
         \
         For continuous samples, poll via the `events` tool with \
         type=\"location\". Lat/lon are signed decimal degrees."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": { "type": "string", "enum": ["fix", "start_stream", "stop_stream"] },
                "provider": { "type": "string" },
                "timeout_ms": { "type": "integer" },
                "max_age_ms": { "type": "integer" },
                "stream_id": { "type": "string" },
                "interval_ms": { "type": "integer" },
                "min_distance_m": { "type": "number" },
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
            let timeout = pick_timeout(&args, match action.as_str() {
                "fix" => args["timeout_ms"].as_u64().unwrap_or(30_000) / 1000 + 5,
                _ => 10,
            });
            let resp = send(BridgeRequest {
                topic: "location",
                body: args.clone(),
                input_asset: None,
                input_asset_ext: "bin",
                timeout,
            }).await?;
            if !resp.json["ok"].as_bool().unwrap_or(false) {
                return Ok(ToolResult::error(format!(
                    "gps {action} failed: {}",
                    resp.json["error"].as_str().unwrap_or("(no error)"),
                )));
            }
            let pretty = match action.as_str() {
                "fix" => format!(
                    "📍 lat {:.6} lon {:.6} ±{}m via {} ({})",
                    resp.json["lat"].as_f64().unwrap_or(0.0),
                    resp.json["lon"].as_f64().unwrap_or(0.0),
                    resp.json["accuracy_m"].as_f64().map(|v| format!("{:.1}", v)).unwrap_or("?".into()),
                    resp.json["provider"].as_str().unwrap_or("?"),
                    if resp.json["from_cache"].as_bool().unwrap_or(false) { "cached" } else { "fresh" },
                ),
                "start_stream" => format!(
                    "📡 GPS stream `{}` started (provider={}, interval {}ms). \
                     Poll via events tool: type=\"location\".",
                    resp.json["stream_id"].as_str().unwrap_or("?"),
                    resp.json["provider"].as_str().unwrap_or("?"),
                    resp.json["interval_ms"].as_u64().unwrap_or(0),
                ),
                "stop_stream" => format!("🛑 GPS stream `{}` stopped.",
                    resp.json["stream_id"].as_str().unwrap_or("?")),
                _ => resp.json.to_string(),
            };
            Ok(ToolResult::success(format!("{pretty}\n\n{}", resp.json)))
        })
    }
}
