use peko_core::tool::{Tool, ToolResult};
use peko_hal::InputDevice;
use serde_json::json;
use std::future::Future;
use std::pin::Pin;
use std::process::{Command, Stdio};
use std::sync::Arc;
use tokio::sync::Mutex;

use crate::screen_state::ensure_awake;

pub struct TouchTool {
    device: Arc<Mutex<InputDevice>>,
    // Display size is detected once, the first time we tap. We cache a
    // sentinel here so we don't re-shell `wm size` on every call.
    display_probed: Arc<Mutex<bool>>,
}

impl TouchTool {
    pub fn new(device: InputDevice) -> Self {
        Self {
            device: Arc::new(Mutex::new(device)),
            display_probed: Arc::new(Mutex::new(false)),
        }
    }
}

/// Parse `wm size` output. Accepts either plain `1080x2340` or
/// `Physical size: 1080x2340` / `Override size: 1080x2340` — when an
/// Override is present Android dispatches touches in override coords, so
/// that's what we want to scale from.
fn parse_wm_size(stdout: &str) -> Option<(i32, i32)> {
    let mut physical: Option<(i32, i32)> = None;
    let mut override_size: Option<(i32, i32)> = None;

    for line in stdout.lines() {
        let line = line.trim();
        let (label, value) = match line.split_once(':') {
            Some((l, v)) => (l.trim().to_lowercase(), v.trim()),
            None => (String::new(), line),
        };
        let wh = value
            .split_once('x')
            .and_then(|(w, h)| Some((w.trim().parse().ok()?, h.trim().parse().ok()?)));
        if let Some(dims) = wh {
            if label.contains("override") {
                override_size = Some(dims);
            } else {
                physical = Some(dims);
            }
        }
    }

    override_size.or(physical)
}

fn detect_display_size() -> Option<(i32, i32)> {
    let out = Command::new("sh")
        .arg("-c").arg("wm size")
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    parse_wm_size(&String::from_utf8_lossy(&out.stdout))
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
        let probed = self.display_probed.clone();
        Box::pin(async move {
            ensure_awake();
            let action = args["action"].as_str().unwrap_or("tap");
            let x = args["x"].as_i64().ok_or_else(|| anyhow::anyhow!("missing x"))? as i32;
            let y = args["y"].as_i64().ok_or_else(|| anyhow::anyhow!("missing y"))? as i32;

            // First call on this tool instance: ask `wm size` for the
            // display resolution and pipe it into the InputDevice so
            // subsequent taps scale from display → panel ABS coords.
            // Without this, a screenshot in 1080x2340 display pixels sent
            // to a 1440x3120 native-resolution panel lands in the wrong
            // quadrant.
            {
                let mut probed = probed.lock().await;
                if !*probed {
                    *probed = true;
                    if let Some((dw, dh)) = detect_display_size() {
                        let mut dev = device.lock().await;
                        if dev.has_abs_calibration() {
                            dev.set_display_size(dw, dh);
                        }
                    }
                }
            }

            let mut dev = device.lock().await;

            match action {
                "tap" => {
                    dev.inject_tap(x, y)?;
                    Ok(ToolResult::success(format!("Tapped at ({}, {})", x, y)))
                }
                "long_press" => {
                    let duration = args["duration_ms"].as_u64().unwrap_or(500);
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

#[cfg(test)]
mod tests {
    use super::parse_wm_size;

    #[test]
    fn parses_physical_only() {
        let out = "Physical size: 1080x2340\n";
        assert_eq!(parse_wm_size(out), Some((1080, 2340)));
    }

    #[test]
    fn prefers_override_when_present() {
        let out = "Physical size: 1440x3120\nOverride size: 1080x2340\n";
        assert_eq!(parse_wm_size(out), Some((1080, 2340)));
    }

    #[test]
    fn parses_bare_dimensions() {
        assert_eq!(parse_wm_size("1080x2340\n"), Some((1080, 2340)));
    }

    #[test]
    fn returns_none_on_garbage() {
        assert_eq!(parse_wm_size("error: command not found\n"), None);
    }
}
