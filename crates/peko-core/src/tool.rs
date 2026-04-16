use async_trait::async_trait;
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use crate::message::ImageData;

#[derive(Debug, Clone)]
pub struct ToolResult {
    pub content: String,
    pub is_error: bool,
    pub image: Option<ImageData>,
}

impl ToolResult {
    pub fn success(content: impl Into<String>) -> Self {
        Self { content: content.into(), is_error: false, image: None }
    }

    pub fn error(content: impl Into<String>) -> Self {
        Self { content: content.into(), is_error: true, image: None }
    }

    pub fn with_image(content: impl Into<String>, base64: String, media_type: String) -> Self {
        Self {
            content: content.into(),
            is_error: false,
            image: Some(ImageData { base64, media_type }),
        }
    }
}

pub trait Tool: Send + Sync + 'static {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn parameters_schema(&self) -> serde_json::Value;

    fn is_available(&self) -> bool { true }
    fn is_dangerous(&self) -> bool { false }

    fn execute(
        &self,
        args: serde_json::Value,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<ToolResult>> + Send + '_>>;
}

pub struct ToolRegistry {
    tools: HashMap<String, Arc<dyn Tool>>,
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self { tools: HashMap::new() }
    }

    pub fn register(&mut self, tool: impl Tool) {
        let name = tool.name().to_string();
        self.tools.insert(name, Arc::new(tool));
    }

    pub fn schemas(&self) -> Vec<serde_json::Value> {
        self.tools.values()
            .filter(|t| t.is_available())
            .map(|t| {
                serde_json::json!({
                    "name": t.name(),
                    "description": t.description(),
                    "input_schema": t.parameters_schema(),
                })
            })
            .collect()
    }

    pub async fn execute(&self, name: &str, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let tool = self.tools.get(name)
            .ok_or_else(|| anyhow::anyhow!("unknown tool: {}", name))?;

        if !tool.is_available() {
            return Ok(ToolResult::error(format!("tool '{}' is not available", name)));
        }

        tool.execute(args).await
    }

    pub fn available_tools(&self) -> Vec<&str> {
        self.tools.values()
            .filter(|t| t.is_available())
            .map(|t| t.name())
            .collect()
    }

    pub fn is_dangerous(&self, name: &str) -> bool {
        self.tools.get(name).map(|t| t.is_dangerous()).unwrap_or(false)
    }
}

impl Default for ToolRegistry {
    fn default() -> Self { Self::new() }
}
