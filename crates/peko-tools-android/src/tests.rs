#[cfg(test)]
mod tests {
    use peko_core::tool::{Tool, ToolResult, ToolRegistry};
    use peko_core::memory::{MemoryStore, MemoryCategory};
    use crate::shell::ShellTool;
    use crate::filesystem::FileSystemTool;
    use crate::memory_tool::MemoryTool;
    use std::sync::Arc;
    use tokio::sync::Mutex;

    #[test]
    fn test_shell_tool_schema() {
        let tool = ShellTool::new(30);
        assert_eq!(tool.name(), "shell");
        assert!(tool.is_dangerous());
        assert!(tool.is_available());

        let schema = tool.parameters_schema();
        assert!(schema["properties"]["command"].is_object());
        let required = schema["required"].as_array().unwrap();
        assert!(required.iter().any(|v| v == "command"));
    }

    #[test]
    fn test_filesystem_tool_schema() {
        let tool = FileSystemTool::new(vec![std::path::PathBuf::from("/tmp")]);
        assert_eq!(tool.name(), "filesystem");
        assert!(!tool.is_dangerous());

        let schema = tool.parameters_schema();
        let actions = schema["properties"]["action"]["enum"].as_array().unwrap();
        assert!(actions.iter().any(|v| v == "read"));
        assert!(actions.iter().any(|v| v == "write"));
        assert!(actions.iter().any(|v| v == "list"));
        assert!(actions.iter().any(|v| v == "delete"));
    }

    #[test]
    fn test_memory_tool_schema() {
        let store = Arc::new(Mutex::new(MemoryStore::open_in_memory().unwrap()));
        let tool = MemoryTool::new(store);
        assert_eq!(tool.name(), "memory");
        assert!(!tool.is_dangerous());

        let schema = tool.parameters_schema();
        let actions = schema["properties"]["action"]["enum"].as_array().unwrap();
        assert_eq!(actions.len(), 4); // save, search, list, delete
    }

    #[tokio::test]
    async fn test_shell_tool_execute() {
        let tool = ShellTool::new(10);
        let result = tool.execute(serde_json::json!({"command": "echo hello_test"})).await.unwrap();
        assert!(!result.is_error);
        assert!(result.content.contains("hello_test"));
    }

    #[tokio::test]
    async fn test_shell_tool_timeout() {
        let tool = ShellTool::new(1); // 1 second timeout
        let result = tool.execute(serde_json::json!({"command": "sleep 10"})).await.unwrap();
        assert!(result.is_error || result.content.contains("timed out"));
    }

    #[tokio::test]
    async fn test_filesystem_tool_write_and_read() {
        let tmp = std::fs::canonicalize(std::env::temp_dir()).unwrap();
        let tool = FileSystemTool::new(vec![tmp.clone()]);
        let test_file = tmp.join("peko_test_fs.txt");

        // Write
        let result = tool.execute(serde_json::json!({
            "action": "write",
            "path": test_file.to_str().unwrap(),
            "content": "test content 123"
        })).await.unwrap();
        assert!(!result.is_error);

        // Read
        let result = tool.execute(serde_json::json!({
            "action": "read",
            "path": test_file.to_str().unwrap()
        })).await.unwrap();
        assert!(!result.is_error);
        assert!(result.content.contains("test content 123"));

        // List
        let result = tool.execute(serde_json::json!({
            "action": "list",
            "path": tmp.to_str().unwrap()
        })).await.unwrap();
        assert!(!result.is_error);

        // Cleanup
        let _ = std::fs::remove_file(&test_file);
    }

    #[tokio::test]
    async fn test_filesystem_tool_sandbox() {
        let tool = FileSystemTool::new(vec![std::path::PathBuf::from("/tmp/peko_safe")]);

        // Should deny access outside allowed paths
        let result = tool.execute(serde_json::json!({
            "action": "read",
            "path": "/etc/passwd"
        })).await.unwrap();
        assert!(result.is_error || result.content.contains("denied"));
    }

    #[tokio::test]
    async fn test_memory_tool_save_search_delete() {
        let store = Arc::new(Mutex::new(MemoryStore::open_in_memory().unwrap()));
        let tool = MemoryTool::new(store);

        // Save
        let result = tool.execute(serde_json::json!({
            "action": "save",
            "key": "test_fact",
            "content": "The sky is blue",
            "category": "fact",
            "importance": 0.8
        })).await.unwrap();
        assert!(!result.is_error);
        assert!(result.content.contains("Memory saved"));

        // Search
        let result = tool.execute(serde_json::json!({
            "action": "search",
            "query": "sky blue"
        })).await.unwrap();
        assert!(!result.is_error);
        assert!(result.content.contains("sky is blue"));

        // List
        let result = tool.execute(serde_json::json!({
            "action": "list"
        })).await.unwrap();
        assert!(!result.is_error);
        assert!(result.content.contains("test_fact"));

        // Delete
        let result = tool.execute(serde_json::json!({
            "action": "delete",
            "key": "test_fact"
        })).await.unwrap();
        assert!(!result.is_error);
        assert!(result.content.contains("deleted"));

        // Verify deleted
        let result = tool.execute(serde_json::json!({
            "action": "list"
        })).await.unwrap();
        assert!(result.content.contains("No memories"));
    }

    #[test]
    fn test_tool_registry() {
        let mut registry = ToolRegistry::new();
        registry.register(ShellTool::new(30));
        registry.register(FileSystemTool::new(vec![std::path::PathBuf::from("/tmp")]));

        let tools = registry.available_tools();
        assert!(tools.contains(&"shell"));
        assert!(tools.contains(&"filesystem"));

        let schemas = registry.schemas();
        assert_eq!(schemas.len(), 2);
        assert!(schemas.iter().any(|s| s["name"] == "shell"));
        assert!(schemas.iter().any(|s| s["name"] == "filesystem"));
    }

    #[tokio::test]
    async fn test_tool_registry_execute_unknown() {
        let registry = ToolRegistry::new();
        let result = registry.execute("nonexistent", serde_json::json!({})).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_tool_registry_execute() {
        let mut registry = ToolRegistry::new();
        registry.register(ShellTool::new(10));

        let result = registry.execute("shell", serde_json::json!({"command": "echo works"})).await.unwrap();
        assert!(!result.is_error);
        assert!(result.content.contains("works"));
    }
}
