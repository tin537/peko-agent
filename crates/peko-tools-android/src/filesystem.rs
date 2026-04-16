use peko_core::tool::{Tool, ToolResult};
use serde_json::json;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;

pub struct FileSystemTool {
    allowed_paths: Vec<PathBuf>,
}

impl FileSystemTool {
    pub fn new(allowed_paths: Vec<PathBuf>) -> Self {
        Self { allowed_paths }
    }

    fn is_path_allowed(&self, path: &Path) -> bool {
        let canonical = std::fs::canonicalize(path)
            .or_else(|_| {
                // path might not exist yet (for write), check parent
                path.parent()
                    .and_then(|p| std::fs::canonicalize(p).ok())
                    .ok_or(std::io::Error::new(std::io::ErrorKind::NotFound, ""))
            })
            .unwrap_or_else(|_| path.to_path_buf());

        self.allowed_paths.iter().any(|allowed| canonical.starts_with(allowed))
    }
}

impl Tool for FileSystemTool {
    fn name(&self) -> &str { "filesystem" }

    fn description(&self) -> &str {
        "Read, write, list, or delete files. Operations are sandboxed to allowed directories."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["read", "write", "list", "delete"],
                    "description": "File operation to perform"
                },
                "path": {
                    "type": "string",
                    "description": "File or directory path"
                },
                "content": {
                    "type": "string",
                    "description": "Content to write (for write action)"
                }
            },
            "required": ["action", "path"]
        })
    }

    fn is_dangerous(&self) -> bool { false }

    fn execute(
        &self,
        args: serde_json::Value,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<ToolResult>> + Send + '_>> {
        Box::pin(async move {
            let action = args["action"].as_str()
                .ok_or_else(|| anyhow::anyhow!("missing 'action'"))?;
            let path_str = args["path"].as_str()
                .ok_or_else(|| anyhow::anyhow!("missing 'path'"))?;
            let path = Path::new(path_str);

            if !self.is_path_allowed(path) {
                return Ok(ToolResult::error(format!(
                    "Access denied: {} is not in allowed paths",
                    path_str
                )));
            }

            match action {
                "read" => {
                    match std::fs::read_to_string(path) {
                        Ok(content) => Ok(ToolResult::success(content)),
                        Err(e) => Ok(ToolResult::error(format!("Read failed: {}", e))),
                    }
                }
                "write" => {
                    let content = args["content"].as_str()
                        .ok_or_else(|| anyhow::anyhow!("missing 'content' for write"))?;
                    if let Some(parent) = path.parent() {
                        std::fs::create_dir_all(parent)?;
                    }
                    match std::fs::write(path, content) {
                        Ok(()) => Ok(ToolResult::success(format!("Written {} bytes to {}", content.len(), path_str))),
                        Err(e) => Ok(ToolResult::error(format!("Write failed: {}", e))),
                    }
                }
                "list" => {
                    match std::fs::read_dir(path) {
                        Ok(entries) => {
                            let names: Vec<String> = entries
                                .filter_map(|e| e.ok())
                                .map(|e| {
                                    let name = e.file_name().to_string_lossy().to_string();
                                    let file_type = if e.path().is_dir() { "dir" } else { "file" };
                                    format!("{} ({})", name, file_type)
                                })
                                .collect();
                            Ok(ToolResult::success(names.join("\n")))
                        }
                        Err(e) => Ok(ToolResult::error(format!("List failed: {}", e))),
                    }
                }
                "delete" => {
                    if path.is_dir() {
                        match std::fs::remove_dir_all(path) {
                            Ok(()) => Ok(ToolResult::success(format!("Deleted directory {}", path_str))),
                            Err(e) => Ok(ToolResult::error(format!("Delete failed: {}", e))),
                        }
                    } else {
                        match std::fs::remove_file(path) {
                            Ok(()) => Ok(ToolResult::success(format!("Deleted {}", path_str))),
                            Err(e) => Ok(ToolResult::error(format!("Delete failed: {}", e))),
                        }
                    }
                }
                _ => Ok(ToolResult::error(format!("Unknown action: {}", action))),
            }
        })
    }
}
