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

    /// Return a new registry containing only the tools whose names
    /// are in `allowed`. Used to expose a narrowed surface to remote
    /// transports (Telegram, future webhooks) without affecting the
    /// full local registry.
    ///
    /// Tools are reference-counted (Arc<dyn Tool>), so the narrowed
    /// registry shares state with the original — registering a new
    /// memory or skill in the full registry is visible through the
    /// narrowed one too.
    pub fn narrow_to(&self, allowed: &[String]) -> Self {
        let mut out = Self::new();
        for name in allowed {
            if let Some(tool) = self.tools.get(name) {
                out.tools.insert(name.clone(), tool.clone());
            }
        }
        out
    }
}

impl Default for ToolRegistry {
    fn default() -> Self { Self::new() }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::future::Future;
    use std::pin::Pin;

    struct DummyTool(&'static str);
    impl Tool for DummyTool {
        fn name(&self) -> &str { self.0 }
        fn description(&self) -> &str { "test tool" }
        fn parameters_schema(&self) -> serde_json::Value { serde_json::json!({}) }
        fn execute(
            &self,
            _args: serde_json::Value,
        ) -> Pin<Box<dyn Future<Output = anyhow::Result<ToolResult>> + Send + '_>> {
            let name = self.0.to_string();
            Box::pin(async move { Ok(ToolResult::success(name)) })
        }
    }

    #[test]
    fn narrow_to_drops_disallowed_tools() {
        let mut full = ToolRegistry::new();
        full.register(DummyTool("screenshot"));
        full.register(DummyTool("shell"));
        full.register(DummyTool("memory"));
        full.register(DummyTool("package_manager"));

        let narrow = full.narrow_to(&[
            "screenshot".to_string(),
            "memory".to_string(),
        ]);

        let names = narrow.available_tools();
        assert!(names.contains(&"screenshot"));
        assert!(names.contains(&"memory"));
        assert!(!names.contains(&"shell"), "shell must NOT survive narrowing");
        assert!(!names.contains(&"package_manager"));
    }

    #[test]
    fn narrow_to_silently_skips_unknown_names() {
        let mut full = ToolRegistry::new();
        full.register(DummyTool("screenshot"));
        let narrow = full.narrow_to(&[
            "screenshot".to_string(),
            "ghost_tool".to_string(),
        ]);
        assert_eq!(narrow.available_tools(), vec!["screenshot"]);
    }

    #[test]
    fn narrow_to_with_empty_allowlist_yields_empty_registry() {
        let mut full = ToolRegistry::new();
        full.register(DummyTool("screenshot"));
        let narrow = full.narrow_to(&[]);
        assert!(narrow.available_tools().is_empty());
    }
}
