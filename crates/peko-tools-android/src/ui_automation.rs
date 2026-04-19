use peko_core::tool::{Tool, ToolResult};
use peko_hal::{SurfaceFlingerCapture, UiHierarchy};
use serde_json::json;
use std::future::Future;
use std::pin::Pin;

use crate::screen_state::ensure_awake;

pub struct UiAutomationTool;

impl UiAutomationTool {
    pub fn new() -> Self { Self }
}

impl Tool for UiAutomationTool {
    fn name(&self) -> &str { "ui_inspect" }

    fn description(&self) -> &str {
        "Inspect the current UI. Actions: dump_hierarchy (get all UI elements with bounds and text), \
         find_text (find elements containing text), find_id (find element by resource ID), \
         screenshot_sf (take screenshot via SurfaceFlinger). \
         Returns element bounds as [left,top][right,bottom] — use center coordinates for tapping."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["dump_hierarchy", "find_text", "find_id", "screenshot_sf"],
                    "description": "Inspection action to perform"
                },
                "query": {
                    "type": "string",
                    "description": "Search text or resource ID (for find_text/find_id)"
                }
            },
            "required": ["action"]
        })
    }

    fn is_available(&self) -> bool {
        UiHierarchy::is_available() || SurfaceFlingerCapture::is_available()
    }

    fn execute(
        &self,
        args: serde_json::Value,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<ToolResult>> + Send + '_>> {
        Box::pin(async move {
            // uiautomator dump and SurfaceFlinger capture both return stale
            // / blank results against a dozing display; wake first.
            ensure_awake();
            let action = args["action"].as_str()
                .ok_or_else(|| anyhow::anyhow!("missing 'action'"))?;

            match action {
                "dump_hierarchy" => {
                    match UiHierarchy::dump_flat() {
                        Ok(nodes) => {
                            let mut output = format!("Found {} UI elements:\n\n", nodes.len());
                            for node in &nodes {
                                let bounds_str = node.bounds
                                    .map(|b| format!("[{},{} {}x{}] center:({},{})",
                                        b.left, b.top, b.width(), b.height(),
                                        b.center().0, b.center().1))
                                    .unwrap_or_else(|| "no-bounds".to_string());

                                let mut desc_parts = Vec::new();
                                if !node.text.is_empty() {
                                    desc_parts.push(format!("text=\"{}\"", node.text));
                                }
                                if !node.content_desc.is_empty() {
                                    desc_parts.push(format!("desc=\"{}\"", node.content_desc));
                                }
                                if !node.resource_id.is_empty() {
                                    desc_parts.push(format!("id=\"{}\"", node.resource_id));
                                }
                                if node.clickable { desc_parts.push("clickable".to_string()); }
                                if node.scrollable { desc_parts.push("scrollable".to_string()); }
                                if node.focused { desc_parts.push("FOCUSED".to_string()); }

                                let class_short = node.class.rsplit('.').next().unwrap_or(&node.class);
                                output.push_str(&format!("  {} {} {}\n",
                                    class_short, bounds_str, desc_parts.join(" ")));
                            }
                            Ok(ToolResult::success(output))
                        }
                        Err(e) => Ok(ToolResult::error(format!("UI dump failed: {}", e)))
                    }
                }

                "find_text" => {
                    let query = args["query"].as_str()
                        .ok_or_else(|| anyhow::anyhow!("missing 'query' for find_text"))?;

                    match UiHierarchy::find_by_text(query) {
                        Ok(nodes) => {
                            if nodes.is_empty() {
                                Ok(ToolResult::success(format!("No elements found matching '{}'", query)))
                            } else {
                                let mut output = format!("Found {} matches for '{}':\n", nodes.len(), query);
                                for node in &nodes {
                                    if let Some(b) = node.bounds {
                                        output.push_str(&format!(
                                            "  text=\"{}\" center=({},{}) clickable={}\n",
                                            if !node.text.is_empty() { &node.text } else { &node.content_desc },
                                            b.center().0, b.center().1, node.clickable));
                                    }
                                }
                                Ok(ToolResult::success(output))
                            }
                        }
                        Err(e) => Ok(ToolResult::error(format!("find_text failed: {}", e)))
                    }
                }

                "find_id" => {
                    let query = args["query"].as_str()
                        .ok_or_else(|| anyhow::anyhow!("missing 'query' for find_id"))?;

                    match UiHierarchy::find_by_id(query) {
                        Ok(Some(node)) => {
                            let bounds_info = node.bounds
                                .map(|b| format!("center=({},{}) size={}x{}",
                                    b.center().0, b.center().1, b.width(), b.height()))
                                .unwrap_or_else(|| "no bounds".to_string());
                            Ok(ToolResult::success(format!(
                                "Found: id=\"{}\" text=\"{}\" {} clickable={}",
                                node.resource_id, node.text, bounds_info, node.clickable)))
                        }
                        Ok(None) => Ok(ToolResult::success(format!("No element with id '{}'", query))),
                        Err(e) => Ok(ToolResult::error(format!("find_id failed: {}", e)))
                    }
                }

                "screenshot_sf" => {
                    if !SurfaceFlingerCapture::is_available() {
                        return Ok(ToolResult::error("screencap not available (framework not running?)"));
                    }

                    match SurfaceFlingerCapture::capture_png() {
                        Ok(png_bytes) => {
                            let b64 = base64::Engine::encode(
                                &base64::engine::general_purpose::STANDARD,
                                &png_bytes,
                            );
                            Ok(ToolResult::with_image(
                                "Screenshot captured via SurfaceFlinger".to_string(),
                                b64,
                                "image/png".to_string(),
                            ))
                        }
                        Err(e) => Ok(ToolResult::error(format!("screenshot failed: {}", e)))
                    }
                }

                _ => Ok(ToolResult::error(format!("unknown ui_inspect action: {}", action)))
            }
        })
    }
}
