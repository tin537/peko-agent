use peko_core::tool::{Tool, ToolResult, ToolRegistry};
use peko_core::runtime::{AgentRuntime, build_provider_helper};
use peko_core::session::SessionStore;
use peko_core::memory::MemoryStore;
use peko_core::skills::SkillStore;
use peko_core::prompt::SystemPrompt;
use peko_config::AgentConfig;
use serde_json::json;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use tokio::sync::Mutex;

/// DelegateTool spawns a child AgentRuntime to handle a subtask in parallel.
/// The parent agent can delegate work and get results back.
pub struct DelegateTool {
    tools: Arc<ToolRegistry>,
    config: Arc<Mutex<serde_json::Value>>,
    session_db_path: std::path::PathBuf,
    memory: Arc<Mutex<MemoryStore>>,
    skills: Arc<Mutex<SkillStore>>,
    soul: Arc<Mutex<String>>,
    max_iterations: usize,
}

impl DelegateTool {
    pub fn new(
        tools: Arc<ToolRegistry>,
        config: Arc<Mutex<serde_json::Value>>,
        session_db_path: std::path::PathBuf,
        memory: Arc<Mutex<MemoryStore>>,
        skills: Arc<Mutex<SkillStore>>,
        soul: Arc<Mutex<String>>,
    ) -> Self {
        Self {
            tools,
            config,
            session_db_path,
            memory,
            skills,
            soul,
            max_iterations: 20, // Subagents get a smaller budget
        }
    }
}

impl Tool for DelegateTool {
    fn name(&self) -> &str { "delegate" }

    fn description(&self) -> &str {
        "Delegate a subtask to a child agent that runs independently. \
         Use this to parallelize work or break complex tasks into smaller pieces. \
         The child agent has access to the same tools but runs with its own session \
         and a smaller iteration budget (max 20). \
         Returns the child agent's response when complete."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "task": {
                    "type": "string",
                    "description": "The subtask for the child agent to complete"
                },
                "max_iterations": {
                    "type": "integer",
                    "description": "Max iterations for the child (default 20, max 50)"
                }
            },
            "required": ["task"]
        })
    }

    fn execute(
        &self,
        args: serde_json::Value,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<ToolResult>> + Send + '_>> {
        let tools = self.tools.clone();
        let config = self.config.clone();
        let db_path = self.session_db_path.clone();
        let memory = self.memory.clone();
        let skills = self.skills.clone();
        let soul = self.soul.clone();
        let default_max = self.max_iterations;

        Box::pin(async move {
            let task = args["task"].as_str()
                .ok_or_else(|| anyhow::anyhow!("missing 'task'"))?;

            let max_iter = args["max_iterations"].as_u64()
                .map(|v| (v as usize).min(50))
                .unwrap_or(default_max);

            tracing::info!(task = %task, max_iter, "delegating subtask to child agent");

            let config_val = config.lock().await.clone();

            let provider = match build_provider_helper(&config_val) {
                Ok(p) => p,
                Err(e) => return Ok(ToolResult::error(format!("delegate failed: no provider: {}", e))),
            };

            let session = match SessionStore::open(&db_path) {
                Ok(s) => s,
                Err(e) => return Ok(ToolResult::error(format!("delegate failed: {}", e))),
            };

            let agent_config = AgentConfig {
                max_iterations: max_iter,
                context_window: config_val["agent"]["context_window"].as_u64().unwrap_or(200000) as usize,
                history_share: config_val["agent"]["history_share"].as_f64().unwrap_or(0.7) as f32,
                data_dir: std::path::PathBuf::from(
                    config_val["agent"]["data_dir"].as_str().unwrap_or("/data/peko")
                ),
                log_level: "info".to_string(),
            };

            let soul_text = soul.lock().await.clone();
            let prompt = SystemPrompt::new().with_soul(soul_text);

            let mut runtime = AgentRuntime::new(
                &agent_config,
                tools,
                provider,
                session,
            )
            .with_system_prompt(prompt)
            .with_memory(memory)
            .with_skills(skills);

            match runtime.run_task(task).await {
                Ok(response) => {
                    let result = format!(
                        "[Subtask completed in {} iterations]\n\n{}",
                        response.iterations,
                        if response.text.is_empty() { "(no text response)".to_string() } else { response.text }
                    );
                    Ok(ToolResult::success(result))
                }
                Err(e) => Ok(ToolResult::error(format!("Subtask failed: {}", e))),
            }
        })
    }
}
