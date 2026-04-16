use peko_core::tool::{Tool, ToolResult};
use peko_hal::SerialModem;
use serde_json::json;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use tokio::sync::Mutex;

pub struct CallTool {
    modem: Arc<Mutex<SerialModem>>,
}

impl CallTool {
    pub fn new(modem: SerialModem) -> Self {
        Self { modem: Arc::new(Mutex::new(modem)) }
    }
}

impl Tool for CallTool {
    fn name(&self) -> &str { "call" }

    fn description(&self) -> &str {
        "Make, answer, or end phone calls. Actions: dial, hangup, answer."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["dial", "hangup", "answer"],
                    "description": "Call action to perform"
                },
                "number": {
                    "type": "string",
                    "description": "Phone number to dial (required for 'dial' action)"
                }
            },
            "required": ["action"]
        })
    }

    fn is_dangerous(&self) -> bool { true }

    fn execute(
        &self,
        args: serde_json::Value,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<ToolResult>> + Send + '_>> {
        let modem = self.modem.clone();
        Box::pin(async move {
            let action = args["action"].as_str()
                .ok_or_else(|| anyhow::anyhow!("missing 'action' parameter"))?;

            let mut m = modem.lock().await;

            match action {
                "dial" => {
                    let number = args["number"].as_str()
                        .ok_or_else(|| anyhow::anyhow!("missing 'number' for dial action"))?;
                    let response = m.dial(number)?;
                    Ok(ToolResult::success(format!("Dialing {}... {}", number, response.trim())))
                }
                "hangup" => {
                    let response = m.hangup()?;
                    Ok(ToolResult::success(format!("Call ended. {}", response.trim())))
                }
                "answer" => {
                    let response = m.answer()?;
                    Ok(ToolResult::success(format!("Call answered. {}", response.trim())))
                }
                _ => Ok(ToolResult::error(format!("unknown call action: {}", action))),
            }
        })
    }
}
