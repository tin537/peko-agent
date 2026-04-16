use peko_core::tool::{Tool, ToolResult};
use peko_hal::SerialModem;
use serde_json::json;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use tokio::sync::Mutex;

pub struct SmsTool {
    modem: Arc<Mutex<SerialModem>>,
}

impl SmsTool {
    pub fn new(modem: SerialModem) -> Self {
        Self { modem: Arc::new(Mutex::new(modem)) }
    }
}

impl Tool for SmsTool {
    fn name(&self) -> &str { "sms" }

    fn description(&self) -> &str {
        "Send an SMS text message to a phone number."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "to": {
                    "type": "string",
                    "description": "Phone number to send to (e.g., +1234567890)"
                },
                "message": {
                    "type": "string",
                    "description": "Message text to send"
                }
            },
            "required": ["to", "message"]
        })
    }

    fn is_dangerous(&self) -> bool { true }

    fn execute(
        &self,
        args: serde_json::Value,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<ToolResult>> + Send + '_>> {
        let modem = self.modem.clone();
        Box::pin(async move {
            let to = args["to"].as_str()
                .ok_or_else(|| anyhow::anyhow!("missing 'to' parameter"))?;
            let message = args["message"].as_str()
                .ok_or_else(|| anyhow::anyhow!("missing 'message' parameter"))?;

            let mut m = modem.lock().await;
            let response = m.send_sms(to, message)?;

            if response.contains("OK") {
                Ok(ToolResult::success(format!("SMS sent to {}", to)))
            } else {
                Ok(ToolResult::error(format!("SMS send failed: {}", response.trim())))
            }
        })
    }
}
