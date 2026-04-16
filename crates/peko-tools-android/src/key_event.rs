use peko_core::tool::{Tool, ToolResult};
use peko_hal::InputDevice;
use serde_json::json;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use tokio::sync::Mutex;

pub struct KeyEventTool {
    device: Arc<Mutex<InputDevice>>,
}

impl KeyEventTool {
    pub fn new(device: InputDevice) -> Self {
        Self { device: Arc::new(Mutex::new(device)) }
    }
}

impl Tool for KeyEventTool {
    fn name(&self) -> &str { "key_event" }

    fn description(&self) -> &str {
        "Press a hardware key. Supported keys: HOME, BACK, POWER, VOLUME_UP, VOLUME_DOWN, ENTER"
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "key": {
                    "type": "string",
                    "enum": ["HOME", "BACK", "POWER", "VOLUME_UP", "VOLUME_DOWN", "ENTER"],
                    "description": "The key to press"
                }
            },
            "required": ["key"]
        })
    }

    fn execute(
        &self,
        args: serde_json::Value,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<ToolResult>> + Send + '_>> {
        let device = self.device.clone();
        Box::pin(async move {
            let key_name = args["key"].as_str()
                .ok_or_else(|| anyhow::anyhow!("missing 'key' parameter"))?;

            let keycode = InputDevice::key_code_for_name(key_name)
                .ok_or_else(|| anyhow::anyhow!("unknown key: {}", key_name))?;

            let mut dev = device.lock().await;
            dev.inject_key(keycode)?;

            Ok(ToolResult::success(format!("Pressed {} key", key_name)))
        })
    }
}
