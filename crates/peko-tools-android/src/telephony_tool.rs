//! Phase 23 — read-only telephony tool. Bridges to TelephonyBridgeService.

use crate::bridge_client::{pick_timeout, send, BridgeRequest};
use peko_core::tool::{Tool, ToolResult};
use serde_json::{json, Value};
use std::future::Future;
use std::pin::Pin;

pub struct TelephonyTool;
impl TelephonyTool { pub fn new() -> Self { Self } }
impl Default for TelephonyTool { fn default() -> Self { Self::new() } }

impl Tool for TelephonyTool {
    fn name(&self) -> &str { "telephony" }

    fn description(&self) -> &str {
        "Read-only telephony info. Three actions: \
         info — sim state, carrier, country, phone type, network type, \
            data state, optional phone number. \
         signal — current registered cell signal level + dBm + network \
            type. \
         cells — full neighbour-cell list with per-cell signal stats \
            (LTE/5G/GSM/WCDMA/CDMA fields differ; a `type` field tells \
            you which schema to read)."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": { "type": "string", "enum": ["info", "signal", "cells"] },
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
            let timeout = pick_timeout(&args, 10);
            let resp = send(BridgeRequest {
                topic: "telephony",
                body: args,
                input_asset: None,
                input_asset_ext: "bin",
                timeout,
            }).await?;
            if !resp.json["ok"].as_bool().unwrap_or(false) {
                return Ok(ToolResult::error(format!(
                    "telephony failed: {}",
                    resp.json["error"].as_str().unwrap_or("(no error)"),
                )));
            }
            Ok(ToolResult::success(format!(
                "📶 telephony\n\n{}",
                serde_json::to_string_pretty(&resp.json).unwrap_or_default()
            )))
        })
    }
}
