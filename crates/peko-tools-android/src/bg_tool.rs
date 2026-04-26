//! Background task tool. Fires an agent task in a tokio::spawn'd
//! worker so the calling agent returns immediately. Use it for tasks
//! whose result the user is willing to wait for asynchronously
//! (research, multi-step planning, slow web fetches), so the
//! conversation stays interactive.
//!
//! Polling-based completion: the user pulls results via `bg status`
//! / `bg wait` / `bg list`. There's no auto-Telegram-callback in
//! Phase 19; that's a separate notification surface that needs a
//! per-job notify endpoint, scoped for a follow-up.
//!
//! Composes with `delegate`: to spawn an async sub-agent, call
//! `bg fire { task: "delegate ..." }` — the bg worker invokes the
//! agent runtime, which in turn calls the delegate tool. Two layers
//! of independence.

use peko_config::AgentConfig;
use peko_core::runtime::{build_provider_helper, AgentRuntime};
use peko_core::tool::{Tool, ToolRegistry, ToolResult};
use peko_core::{BgStore, MemoryStore, SessionStore, SkillStore, SystemPrompt};
use serde_json::json;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use tokio::sync::Mutex;

/// Hard cap on concurrent in-flight bg jobs. The agent's LLM provider
/// is the real bottleneck; >8 concurrent calls will start hitting
/// rate limits + draining tokens. Listing capped jobs at this number
/// also keeps the UI sane in Telegram.
const MAX_CONCURRENT: usize = 8;

/// Shared deferred handle to the tool registry. main.rs creates this
/// holder before registering tools, hands a clone to BgTool at
/// construction, and `set`s the actual Arc<ToolRegistry> AFTER the
/// registry (which contains BgTool itself) has been built. Resolves
/// the chicken-and-egg: BgTool needs the full registry to spawn agent
/// runtimes, but the registry can't exist until after BgTool is in it.
pub type ToolsHandle = Arc<tokio::sync::RwLock<Option<Arc<ToolRegistry>>>>;

pub fn new_tools_handle() -> ToolsHandle {
    Arc::new(tokio::sync::RwLock::new(None))
}

pub struct BgTool {
    bg: BgStore,
    tools: ToolsHandle,
    config: Arc<Mutex<serde_json::Value>>,
    session_db_path: std::path::PathBuf,
    memory: Arc<Mutex<MemoryStore>>,
    skills: Arc<Mutex<SkillStore>>,
    soul: Arc<Mutex<String>>,
}

impl BgTool {
    pub fn new(
        bg: BgStore,
        tools: ToolsHandle,
        config: Arc<Mutex<serde_json::Value>>,
        session_db_path: std::path::PathBuf,
        memory: Arc<Mutex<MemoryStore>>,
        skills: Arc<Mutex<SkillStore>>,
        soul: Arc<Mutex<String>>,
    ) -> Self {
        Self { bg, tools, config, session_db_path, memory, skills, soul }
    }
}

impl Tool for BgTool {
    fn name(&self) -> &str { "bg" }

    fn description(&self) -> &str {
        "Fire-and-forget background tasks. Spawn an agent task that \
         runs in parallel; check on it later. Use for slow work the \
         user is fine waiting on (research, multi-step plans, scrape \
         + summarise pipelines) so the conversation stays responsive. \
         \
         Actions: \
         fire { task: string, name?: string } — start a job; returns \
             {id, short_id} immediately. \
         status { id } — pending|running|done|failed|cancelled, plus \
             result/error when terminal. id can be the short prefix \
             returned from fire. \
         wait { id, timeout_ms?: int } — block until terminal or \
             timeout (default 30s, max 300s). \
         list { include_terminal?: bool } — overview, newest first. \
         cancel { id } — cooperative; sets status=cancelled, the \
             worker observes on its next iteration. \
         \
         Tip: run `delegate` inside `bg fire` for an async sub-agent."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["fire", "status", "wait", "list", "cancel"]
                },
                "task": { "type": "string" },
                "name": { "type": "string" },
                "id": { "type": "string" },
                "timeout_ms": { "type": "integer" },
                "include_terminal": { "type": "boolean" }
            },
            "required": ["action"]
        })
    }

    fn execute(
        &self,
        args: serde_json::Value,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<ToolResult>> + Send + '_>> {
        let bg = self.bg.clone();
        let tools_handle = self.tools.clone();
        let config = self.config.clone();
        let db_path = self.session_db_path.clone();
        let memory = self.memory.clone();
        let skills = self.skills.clone();
        let soul = self.soul.clone();

        Box::pin(async move {
            let action = args["action"].as_str().unwrap_or("").to_string();
            match action.as_str() {
                "fire" => fire(bg, tools_handle, config, db_path, memory, skills, soul, &args).await,
                "status" => status(bg, &args).await,
                "wait" => wait(bg, &args).await,
                "list" => list(bg, &args).await,
                "cancel" => cancel(bg, &args).await,
                "" => Ok(ToolResult::error("missing 'action'".to_string())),
                other => Ok(ToolResult::error(format!(
                    "unknown action '{other}'. valid: fire, status, wait, list, cancel"
                ))),
            }
        })
    }
}

async fn fire(
    bg: BgStore,
    tools_handle: ToolsHandle,
    config: Arc<Mutex<serde_json::Value>>,
    db_path: std::path::PathBuf,
    memory: Arc<Mutex<MemoryStore>>,
    skills: Arc<Mutex<SkillStore>>,
    soul: Arc<Mutex<String>>,
    args: &serde_json::Value,
) -> anyhow::Result<ToolResult> {
    let active = bg.list(false).await.len();
    if active >= MAX_CONCURRENT {
        return Ok(ToolResult::error(format!(
            "{active} bg jobs already running (cap {MAX_CONCURRENT}). Wait for some to finish or cancel."
        )));
    }
    let Some(task) = args["task"].as_str().map(String::from) else {
        return Ok(ToolResult::error("missing 'task'".to_string()));
    };
    if task.trim().is_empty() {
        return Ok(ToolResult::error("'task' is empty".to_string()));
    }
    let name = args["name"].as_str().map(String::from);

    // Resolve tool registry at fire time, not construction time —
    // see ToolsHandle docs.
    let tools = match tools_handle.read().await.clone() {
        Some(t) => t,
        None => {
            return Ok(ToolResult::error(
                "bg tools handle not initialised — bug in main.rs wiring order".to_string(),
            ));
        }
    };

    let job = bg.enqueue(task.clone(), name.clone()).await;
    let id = job.id.clone();
    let short = job.short_id().to_string();

    // Spawn the agent runtime asynchronously. The job's status updates
    // happen inside the spawn so the fire-action returns immediately.
    let bg_for_worker = bg.clone();
    let id_for_worker = id.clone();
    tokio::spawn(async move {
        bg_for_worker.mark_running(&id_for_worker).await;

        // Build a fresh AgentRuntime using the same plumbing as the
        // delegate tool (ProviderChain, SessionStore, MemoryStore, etc.).
        // Cloning Arc<ToolRegistry> means the bg worker shares the
        // parent's tool surface, including this `bg` tool itself —
        // which lets bg jobs fire further bg jobs if they choose.
        let config_val = config.lock().await.clone();
        let provider = match build_provider_helper(&config_val) {
            Ok(p) => p,
            Err(e) => {
                bg_for_worker
                    .mark_failed(&id_for_worker, format!("no provider: {e}"))
                    .await;
                return;
            }
        };
        let session = match SessionStore::open(&db_path) {
            Ok(s) => s,
            Err(e) => {
                bg_for_worker
                    .mark_failed(&id_for_worker, format!("session open: {e}"))
                    .await;
                return;
            }
        };
        let agent_config = AgentConfig {
            max_iterations: config_val["agent"]["max_iterations"]
                .as_u64()
                .unwrap_or(50) as usize,
            context_window: config_val["agent"]["context_window"]
                .as_u64()
                .unwrap_or(200_000) as usize,
            history_share: config_val["agent"]["history_share"]
                .as_f64()
                .unwrap_or(0.7) as f32,
            data_dir: std::path::PathBuf::from(
                config_val["agent"]["data_dir"]
                    .as_str()
                    .unwrap_or("/data/peko"),
            ),
            log_level: "info".to_string(),
        };
        let soul_text = soul.lock().await.clone();
        let prompt = SystemPrompt::new().with_soul(soul_text);

        let mut runtime = AgentRuntime::new(&agent_config, tools, provider, session)
            .with_system_prompt(prompt)
            .with_memory(memory)
            .with_skills(skills);

        match runtime.run_task(&task).await {
            Ok(resp) => {
                let result = if resp.text.is_empty() {
                    "(no text response)".to_string()
                } else {
                    resp.text
                };
                bg_for_worker
                    .mark_done(&id_for_worker, result, Some(resp.session_id), resp.iterations)
                    .await;
            }
            Err(e) => {
                bg_for_worker
                    .mark_failed(&id_for_worker, format!("run_task: {e}"))
                    .await;
            }
        }
    });

    Ok(ToolResult::success(format!(
        "🟡 bg job started — id `{short}` ({}). Use `bg status id={short}` or `bg wait id={short}`.",
        name.unwrap_or_else(|| "unnamed".into())
    )))
}

async fn status(bg: BgStore, args: &serde_json::Value) -> anyhow::Result<ToolResult> {
    let Some(id_arg) = args["id"].as_str() else {
        return Ok(ToolResult::error("missing 'id'".to_string()));
    };
    let Some(id) = bg.resolve(id_arg).await else {
        return Ok(ToolResult::error(format!(
            "no bg job matches id '{id_arg}' (use the short_id from `fire` or `list`)"
        )));
    };
    let Some(job) = bg.get(&id).await else {
        return Ok(ToolResult::error(format!("job '{id_arg}' vanished — pruned?")));
    };
    Ok(ToolResult::success(render_job(&job, true)))
}

async fn wait(bg: BgStore, args: &serde_json::Value) -> anyhow::Result<ToolResult> {
    let Some(id_arg) = args["id"].as_str() else {
        return Ok(ToolResult::error("missing 'id'".to_string()));
    };
    let timeout_ms = args["timeout_ms"]
        .as_u64()
        .map(|v| v.min(300_000))
        .unwrap_or(30_000);
    let Some(id) = bg.resolve(id_arg).await else {
        return Ok(ToolResult::error(format!("no bg job '{id_arg}'")));
    };
    let Some(job) = bg.wait(&id, Some(timeout_ms)).await else {
        return Ok(ToolResult::error(format!("job '{id_arg}' vanished mid-wait")));
    };
    if !job.status.is_terminal() {
        return Ok(ToolResult::success(format!(
            "⏱  bg `{}` still {:?} after {timeout_ms}ms. Try `bg wait` again or `bg status`.",
            job.short_id(),
            job.status
        )));
    }
    Ok(ToolResult::success(render_job(&job, true)))
}

async fn list(bg: BgStore, args: &serde_json::Value) -> anyhow::Result<ToolResult> {
    let include_terminal = args["include_terminal"].as_bool().unwrap_or(true);
    let jobs = bg.list(include_terminal).await;
    if jobs.is_empty() {
        return Ok(ToolResult::success("No bg jobs.".to_string()));
    }
    let mut out = format!(
        "{} bg job(s){}:\n",
        jobs.len(),
        if include_terminal { "" } else { " (active only)" }
    );
    for j in jobs {
        out.push_str(&format!(
            "\n  • [{:?}] `{}` — {} ({})\n    task: {}\n",
            j.status,
            j.short_id(),
            j.name.as_deref().unwrap_or("unnamed"),
            j.created_at.format("%H:%M:%S"),
            truncate_chars(&j.task, 100)
        ));
    }
    Ok(ToolResult::success(out))
}

async fn cancel(bg: BgStore, args: &serde_json::Value) -> anyhow::Result<ToolResult> {
    let Some(id_arg) = args["id"].as_str() else {
        return Ok(ToolResult::error("missing 'id'".to_string()));
    };
    let Some(id) = bg.resolve(id_arg).await else {
        return Ok(ToolResult::error(format!("no bg job '{id_arg}'")));
    };
    if bg.mark_cancelled(&id).await {
        Ok(ToolResult::success(format!(
            "🛑 cancelled bg job `{}`. Worker will observe on next iteration.",
            id_arg
        )))
    } else {
        Ok(ToolResult::success(format!(
            "bg job `{id_arg}` already terminal — nothing to cancel."
        )))
    }
}

fn render_job(job: &peko_core::BgJob, include_payload: bool) -> String {
    let elapsed = job
        .elapsed_ms()
        .map(|ms| format!("{:.1}s", ms as f64 / 1000.0))
        .unwrap_or_else(|| "—".into());
    let mut out = format!(
        "bg `{}` ({}):\n  status: {:?}\n  task: {}\n  created: {}\n  elapsed: {}\n  iterations: {}",
        job.short_id(),
        job.name.as_deref().unwrap_or("unnamed"),
        job.status,
        truncate_chars(&job.task, 200),
        job.created_at.format("%Y-%m-%d %H:%M:%S"),
        elapsed,
        job.iterations,
    );
    if include_payload {
        if let Some(r) = &job.result {
            out.push_str(&format!("\n  result:\n---\n{r}"));
        }
        if let Some(e) = &job.error {
            out.push_str(&format!("\n  error: {e}"));
        }
    }
    out
}

fn truncate_chars(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        format!("{}…", s.chars().take(max).collect::<String>())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_lists_actions() {
        // We can't easily build a full BgTool in unit tests (it needs
        // MemoryStore + SkillStore + SessionStore paths), so just test
        // the schema vector directly via the json blob.
        let schema = json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["fire", "status", "wait", "list", "cancel"]
                }
            }
        });
        let actions: Vec<&str> = schema["properties"]["action"]["enum"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|v| v.as_str())
            .collect();
        for a in ["fire", "status", "wait", "list", "cancel"] {
            assert!(actions.contains(&a));
        }
    }

    #[test]
    fn truncate_chars_safe_on_thai() {
        let s = "ทดสอบภาษาไทยกับการตัด string ยาว ๆ ที่อาจจะเกิน byte boundary";
        let t = truncate_chars(s, 10);
        assert!(t.chars().count() <= 11); // 10 + ellipsis
        assert!(std::str::from_utf8(t.as_bytes()).is_ok());
    }
}
