use peko_core::tool::{Tool, ToolResult};
use peko_hal::InputDevice;
use serde_json::json;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use tokio::sync::Mutex;

use crate::screen_state::ensure_awake;

pub struct TouchTool {
    device: Arc<Mutex<InputDevice>>,
}

impl TouchTool {
    pub fn new(device: InputDevice) -> Self {
        Self { device: Arc::new(Mutex::new(device)) }
    }
}

impl Tool for TouchTool {
    fn name(&self) -> &str { "touch" }

    fn description(&self) -> &str {
        "Inject a touch event on the screen. Supports tap, long_press, and swipe actions."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["tap", "long_press", "swipe"],
                    "description": "The touch action to perform"
                },
                "x": {
                    "type": "integer",
                    "description": "X coordinate"
                },
                "y": {
                    "type": "integer",
                    "description": "Y coordinate"
                },
                "x2": {
                    "type": "integer",
                    "description": "End X coordinate (for swipe)"
                },
                "y2": {
                    "type": "integer",
                    "description": "End Y coordinate (for swipe)"
                },
                "duration_ms": {
                    "type": "integer",
                    "description": "Duration in milliseconds (for swipe/long_press, default 500)"
                }
            },
            "required": ["action", "x", "y"]
        })
    }

    fn execute(
        &self,
        args: serde_json::Value,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<ToolResult>> + Send + '_>> {
        let device = self.device.clone();
        Box::pin(async move {
            // A tap on a dozing phone fires into the dimmed overlay, not
            // the app below. Wake + swipe-dismiss before injecting.
            ensure_awake();
            let action = args["action"].as_str().unwrap_or("tap");
            let x = args["x"].as_i64().ok_or_else(|| anyhow::anyhow!("missing x"))? as i32;
            let y = args["y"].as_i64().ok_or_else(|| anyhow::anyhow!("missing y"))? as i32;

            let mut dev = device.lock().await;

            match action {
                "tap" => {
                    dev.inject_tap(x, y)?;
                    Ok(ToolResult::success(format!("Tapped at ({}, {})", x, y)))
                }
                "long_press" => {
                    let duration = args["duration_ms"].as_u64().unwrap_or(500);
                    // long press = tap down, wait, tap up (reuse swipe to self)
                    dev.inject_swipe(x, y, x, y, duration)?;
                    Ok(ToolResult::success(format!("Long pressed at ({}, {}) for {}ms", x, y, duration)))
                }
                "swipe" => {
                    let x2 = args["x2"].as_i64().ok_or_else(|| anyhow::anyhow!("missing x2"))? as i32;
                    let y2 = args["y2"].as_i64().ok_or_else(|| anyhow::anyhow!("missing y2"))? as i32;
                    let duration = args["duration_ms"].as_u64().unwrap_or(300);
                    dev.inject_swipe(x, y, x2, y2, duration)?;
                    Ok(ToolResult::success(format!("Swiped from ({},{}) to ({},{}) in {}ms", x, y, x2, y2, duration)))
                }
                _ => Ok(ToolResult::error(format!("unknown touch action: {}", action))),
            }
        })
    }
}
