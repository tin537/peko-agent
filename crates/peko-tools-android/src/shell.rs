use peko_core::tool::{Tool, ToolResult};
use serde_json::json;
use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

pub struct ShellTool {
    timeout: Duration,
}

impl ShellTool {
    pub fn new(timeout_seconds: u64) -> Self {
        Self { timeout: Duration::from_secs(timeout_seconds) }
    }
}

impl Tool for ShellTool {
    fn name(&self) -> &str { "shell" }

    fn description(&self) -> &str {
        "Execute a shell command and return its output (stdout and stderr)."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "command": {
                    "type": "string",
                    "description": "Shell command to execute"
                },
                "timeout": {
                    "type": "integer",
                    "description": "Timeout in seconds (optional, default from config)"
                }
            },
            "required": ["command"]
        })
    }

    fn is_dangerous(&self) -> bool { true }

    fn execute(
        &self,
        args: serde_json::Value,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<ToolResult>> + Send + '_>> {
        let default_timeout = self.timeout;
        Box::pin(async move {
            let command = args["command"].as_str()
                .ok_or_else(|| anyhow::anyhow!("missing 'command' parameter"))?;

            let timeout = args["timeout"].as_u64()
                .map(Duration::from_secs)
                .unwrap_or(default_timeout);

            let result = tokio::time::timeout(timeout, async {
                tokio::process::Command::new("sh")
                    .arg("-c")
                    .arg(command)
                    .output()
                    .await
            }).await;

            match result {
                Ok(Ok(output)) => {
                    let stdout = String::from_utf8_lossy(&output.stdout);
                    let stderr = String::from_utf8_lossy(&output.stderr);

                    let mut result_text = String::new();
                    if !stdout.is_empty() {
                        result_text.push_str(&stdout);
                    }
                    if !stderr.is_empty() {
                        if !result_text.is_empty() {
                            result_text.push_str("\nSTDERR: ");
                        }
                        result_text.push_str(&stderr);
                    }
                    if result_text.is_empty() {
                        result_text = format!("Command completed with exit code {}", output.status.code().unwrap_or(-1));
                    }

                    if output.status.success() {
                        Ok(ToolResult::success(result_text))
                    } else {
                        Ok(ToolResult::error(result_text))
                    }
                }
                Ok(Err(e)) => Ok(ToolResult::error(format!("Failed to execute command: {}", e))),
                Err(_) => Ok(ToolResult::error(format!("Command timed out after {:?}", timeout))),
            }
        })
    }
}
