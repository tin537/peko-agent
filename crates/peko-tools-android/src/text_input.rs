use peko_core::tool::{Tool, ToolResult};
use peko_hal::UInputDevice;
use serde_json::json;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use tokio::sync::Mutex;

pub struct TextInputTool {
    device: Arc<Mutex<UInputDevice>>,
}

impl TextInputTool {
    pub fn new(device: UInputDevice) -> Self {
        Self { device: Arc::new(Mutex::new(device)) }
    }
}

impl Tool for TextInputTool {
    fn name(&self) -> &str { "text_input" }

    fn description(&self) -> &str {
        "Type text into the currently focused input field using synthetic key events."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "text": {
                    "type": "string",
                    "description": "Text to type into the focused field"
                }
            },
            "required": ["text"]
        })
    }

    fn execute(
        &self,
        args: serde_json::Value,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<ToolResult>> + Send + '_>> {
        let device = self.device.clone();
        Box::pin(async move {
            let text = args["text"].as_str()
                .ok_or_else(|| anyhow::anyhow!("missing 'text' parameter"))?;

            let mut dev = device.lock().await;
            dev.type_text(text)?;

            Ok(ToolResult::success(format!("Typed {} characters", text.len())))
        })
    }
}
