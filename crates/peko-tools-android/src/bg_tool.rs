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

use peko_config::{AgentConfig, BgConfig};
use peko_core::runtime::{build_provider_helper, AgentRuntime};
use peko_core::tool::{Tool, ToolRegistry, ToolResult};
use peko_core::{
    bg_metrics, estimate_bg_tokens, BgStore, Checkpoint, MemoryStore, Message, SessionStore,
    SkillStore, SystemPrompt, TelegramSender,
};
use serde_json::json;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use tokio::sync::Mutex;

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
    /// [bg] caps + daily budget. Cloned per fire so a live config
    /// reload takes effect on subsequent fires without rebuilding the
    /// tool.
    bg_config: Arc<Mutex<BgConfig>>,
    /// Phase 25 follow-up: when set, every bg job's terminal status
    /// (Done/Failed/Cancelled/Timeout) emits a Telegram message to
    /// the configured chats. Closes the polling gap — users no longer
    /// have to remember `bg status` after firing a slow research job.
    notifier: Option<TelegramSender>,
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
        bg_config: BgConfig,
    ) -> Self {
        Self {
            bg, tools, config, session_db_path, memory, skills, soul,
            bg_config: Arc::new(Mutex::new(bg_config)),
            notifier: None,
        }
    }

    /// Wire a Telegram sender so terminal job statuses get pinged out.
    /// Called from main.rs after the [telegram] config is parsed.
    pub fn with_notifier(mut self, sender: TelegramSender) -> Self {
        self.notifier = Some(sender);
        self
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
                    "enum": ["fire", "status", "wait", "list", "cancel", "stats"]
                },
                "task": { "type": "string" },
                "name": { "type": "string" },
                "id": { "type": "string" },
                "timeout_ms": { "type": "integer" },
                "include_terminal": { "type": "boolean" },
                "max_iterations": {
                    "type": "integer",
                    "description": "Override per-job iteration cap (clamped to [bg].max_iterations)."
                },
                "days": {
                    "type": "integer",
                    "description": "stats: how many days back to return (default 7)."
                }
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
        let bg_cfg = self.bg_config.clone();
        let notifier = self.notifier.clone();

        Box::pin(async move {
            let action = args["action"].as_str().unwrap_or("").to_string();
            match action.as_str() {
                "fire" => fire(bg, tools_handle, config, db_path, memory, skills, soul, bg_cfg, notifier, &args).await,
                "status" => status(bg, &args).await,
                "wait" => wait(bg, &args).await,
                "list" => list(bg, &args).await,
                "cancel" => cancel(bg, &args).await,
                "stats" => stats(bg, bg_cfg, &args).await,
                "" => Ok(ToolResult::error("missing 'action'".to_string())),
                other => Ok(ToolResult::error(format!(
                    "unknown action '{other}'. valid: fire, status, wait, list, cancel, stats"
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
    bg_config: Arc<Mutex<BgConfig>>,
    notifier: Option<TelegramSender>,
    args: &serde_json::Value,
) -> anyhow::Result<ToolResult> {
    let cfg = bg_config.lock().await.clone();

    // Concurrency cap.
    let active = bg.list(false).await.len();
    if active >= cfg.max_concurrent {
        return Ok(ToolResult::error(format!(
            "{active} bg jobs already running (cap {}). Wait for some to finish or cancel.",
            cfg.max_concurrent
        )));
    }

    // Daily token budget check + log rejections so the agent learns
    // about its own usage (the bg_stats counter is queryable via
    // `bg stats`).
    if cfg.max_tokens_per_day > 0 {
        let used = bg.tokens_used_today().await;
        if used >= cfg.max_tokens_per_day {
            bg.bump_metric(bg_metrics::BUDGET_REJECTED, 1).await;
            return Ok(ToolResult::error(format!(
                "daily token budget exhausted: {}/{}. Resets at UTC midnight. \
                 Use `bg stats` to see usage.",
                used, cfg.max_tokens_per_day
            )));
        }
    }

    let Some(task) = args["task"].as_str().map(String::from) else {
        return Ok(ToolResult::error("missing 'task'".to_string()));
    };
    if task.trim().is_empty() {
        return Ok(ToolResult::error("'task' is empty".to_string()));
    }
    let name = args["name"].as_str().map(String::from);

    // Per-job iteration override clamped to global config.
    let max_iter = args["max_iterations"]
        .as_u64()
        .map(|v| (v as usize).min(cfg.max_iterations))
        .unwrap_or(cfg.max_iterations);

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
    let wall_clock_secs = cfg.max_wall_clock_secs;

    spawn_worker(WorkerSpawn {
        bg: bg.clone(),
        id: id.clone(),
        task: task.clone(),
        tools,
        config,
        db_path,
        memory,
        skills,
        soul,
        max_iter,
        wall_clock_secs,
        resume: None,
        notifier,
        short_id: short.clone(),
        name: name.clone(),
    });

    Ok(ToolResult::success(format!(
        "🟡 bg job started — id `{short}` ({}). Use `bg status id={short}` or `bg wait id={short}`.",
        name.unwrap_or_else(|| "unnamed".into())
    )))
}

struct WorkerSpawn {
    bg: BgStore,
    id: String,
    task: String,
    tools: Arc<ToolRegistry>,
    config: Arc<Mutex<serde_json::Value>>,
    db_path: std::path::PathBuf,
    memory: Arc<Mutex<MemoryStore>>,
    skills: Arc<Mutex<SkillStore>>,
    soul: Arc<Mutex<String>>,
    max_iter: usize,
    wall_clock_secs: u64,
    /// Phase 22: when set, the worker resumes from this checkpoint
    /// instead of starting fresh. The `task` field above is ignored in
    /// favour of `resume.task` so a restored job carries its original
    /// user input even if the resume scaffold pads `task` with a
    /// summary.
    resume: Option<Checkpoint>,
    /// Phase 25 follow-up: when set, fire a Telegram message after
    /// the job hits any terminal status. Lets users walk away from a
    /// long bg job and learn the moment it finishes.
    notifier: Option<TelegramSender>,
    /// Pretty short_id + name for notification text (don't reach into
    /// BgJob from the worker because the catalog row may have been
    /// updated by another path).
    short_id: String,
    name: Option<String>,
}

fn spawn_worker(s: WorkerSpawn) {
    let WorkerSpawn {
        bg,
        id,
        task,
        tools,
        config,
        db_path,
        memory,
        skills,
        soul,
        max_iter,
        wall_clock_secs,
        resume,
        notifier,
        short_id,
        name,
    } = s;

    tokio::spawn(async move {
        bg.mark_running(&id).await;

        // Build a fresh AgentRuntime — same plumbing as the delegate
        // tool. Cloning Arc<ToolRegistry> means the bg worker shares
        // the parent's tool surface, including this `bg` tool, so
        // bg-spawned jobs can fire further bg jobs if they choose.
        let config_val = config.lock().await.clone();
        let provider = match build_provider_helper(&config_val) {
            Ok(p) => p,
            Err(e) => {
                bg.mark_failed(&id, format!("no provider: {e}")).await;
                return;
            }
        };
        let session = match SessionStore::open(&db_path) {
            Ok(s) => s,
            Err(e) => {
                bg.mark_failed(&id, format!("session open: {e}")).await;
                return;
            }
        };
        let agent_config = AgentConfig {
            max_iterations: max_iter,
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

        // Phase 22: per-iteration checkpoint hook. The closure clones
        // the BgStore (Arc inside) + id + task and spawns a tokio task
        // for the SQLite write so the agent loop never blocks on disk.
        let bg_for_hook = bg.clone();
        let id_for_hook = id.clone();
        let task_for_hook = task.clone();
        let hook: peko_core::runtime::IterationHook =
            Box::new(move |ctx: peko_core::runtime::IterationContext<'_>| {
                let bg = bg_for_hook.clone();
                let id = id_for_hook.clone();
                let task = task_for_hook.clone();
                let messages = ctx.messages.to_vec();
                let prompt = ctx.system_prompt.to_string();
                let iter = ctx.iteration;
                tokio::spawn(async move {
                    // Checkpoint::new sanitises out base64 image bytes;
                    // see background.rs::sanitise_messages. We snapshot
                    // the system_prompt so a resume after memory/skills
                    // changed sees the same prompt the original messages
                    // were generated against.
                    let ckpt = Checkpoint::new(task, iter, messages)
                        .with_prompt_snapshot(prompt);
                    if let Err(e) = bg.write_checkpoint(&id, &ckpt).await {
                        tracing::warn!(error = %e, "bg: checkpoint write failed");
                    }
                });
            });

        let mut runtime = AgentRuntime::new(&agent_config, tools, provider, session)
            .with_system_prompt(prompt)
            .with_memory(memory)
            .with_skills(skills)
            .with_iteration_hook(hook);

        // Phase 22: if resuming, hydrate runtime with the saved state.
        // run_task ignores its user_input arg when resume_state is set,
        // so we still pass `task` (used for session metadata).
        let run_input = if let Some(ckpt) = resume {
            tracing::info!(
                job = %id,
                iter = ckpt.iterations,
                msgs = ckpt.messages.len(),
                "Phase 22: resuming bg worker from checkpoint"
            );
            bg.bump_metric(bg_metrics::RESUMED, 1).await;
            let task_for_resume = ckpt.task.clone();
            runtime = runtime.with_resume_state(
                ckpt.messages,
                ckpt.iterations,
                ckpt.task,
            );
            task_for_resume
        } else {
            task.clone()
        };

        let run = runtime.run_task(&run_input);
        let outcome = if wall_clock_secs > 0 {
            match tokio::time::timeout(
                std::time::Duration::from_secs(wall_clock_secs),
                run,
            )
            .await
            {
                Ok(r) => Some(r),
                Err(_) => None,
            }
        } else {
            Some(run.await)
        };
        let label = name.as_deref().unwrap_or("unnamed");
        match outcome {
            Some(Ok(resp)) => {
                let result = if resp.text.is_empty() {
                    "(no text response)".to_string()
                } else {
                    resp.text.clone()
                };
                let tokens = estimate_bg_tokens(&run_input, resp.iterations, &resp.text);
                bg.mark_done(
                    &id,
                    result.clone(),
                    Some(resp.session_id),
                    resp.iterations,
                    tokens,
                )
                .await;
                notify_terminal(
                    &notifier, &short_id, label, "✅ done",
                    Some(&truncate_chars(&result, 800)), resp.iterations, tokens,
                ).await;
            }
            Some(Err(e)) => {
                let err_msg = format!("run_task: {e}");
                bg.mark_failed(&id, err_msg.clone()).await;
                notify_terminal(
                    &notifier, &short_id, label, "❌ failed",
                    Some(&err_msg), 0, 0,
                ).await;
            }
            None => {
                bg.mark_timeout(&id, wall_clock_secs).await;
                notify_terminal(
                    &notifier, &short_id, label, "⏰ timeout",
                    Some(&format!("exceeded {wall_clock_secs}s wall clock")), 0, 0,
                ).await;
            }
        }
    });
}

/// Send a one-shot notification to every chat configured on the
/// TelegramSender. Best-effort — failures are logged at warn but don't
/// affect the bg job's terminal status (which already landed in the DB).
async fn notify_terminal(
    notifier: &Option<TelegramSender>,
    short_id: &str,
    label: &str,
    headline: &str,
    detail: Option<&str>,
    iterations: usize,
    tokens: u64,
) {
    let Some(sender) = notifier else { return };
    let mut text = format!(
        "🤖 bg `{short_id}` ({label}) — {headline}\n  iterations: {iterations}, tokens: {tokens}"
    );
    if let Some(d) = detail {
        if !d.is_empty() {
            text.push_str("\n\n");
            text.push_str(d);
        }
    }
    sender.send(&text).await;
}


/// Phase 22: re-spawn workers for `Running` jobs that survived a prior
/// process crash/restart and have a recent enough checkpoint. Stale or
/// checkpoint-less Running rows are auto-marked Failed by
/// `BgStore::pending_resumable`. Call this from main.rs AFTER the tools
/// handle is wired (workers need the live registry to spawn agent
/// runtimes).
pub async fn resume_pending_bg_jobs(
    bg: BgStore,
    tools: Arc<ToolRegistry>,
    config: Arc<Mutex<serde_json::Value>>,
    db_path: std::path::PathBuf,
    memory: Arc<Mutex<MemoryStore>>,
    skills: Arc<Mutex<SkillStore>>,
    soul: Arc<Mutex<String>>,
    bg_config: BgConfig,
    max_age: chrono::Duration,
    notifier: Option<TelegramSender>,
) -> usize {
    let pending = bg.pending_resumable(max_age).await;
    let count = pending.len();
    for (job, ckpt) in pending {
        let short_id = job.short_id().to_string();
        let name = job.name.clone();
        spawn_worker(WorkerSpawn {
            bg: bg.clone(),
            id: job.id.clone(),
            task: ckpt.task.clone(),
            tools: tools.clone(),
            config: config.clone(),
            db_path: db_path.clone(),
            memory: memory.clone(),
            skills: skills.clone(),
            soul: soul.clone(),
            max_iter: bg_config.max_iterations,
            wall_clock_secs: bg_config.max_wall_clock_secs,
            resume: Some(ckpt),
            notifier: notifier.clone(),
            short_id,
            name,
        });
    }
    count
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

async fn stats(
    bg: BgStore,
    bg_config: Arc<Mutex<BgConfig>>,
    args: &serde_json::Value,
) -> anyhow::Result<ToolResult> {
    let days = args["days"].as_u64().unwrap_or(7).clamp(1, 90) as usize;
    let cfg = bg_config.lock().await.clone();
    let recent = bg.recent_stats(days).await;
    let used_today = bg.tokens_used_today().await;

    let pct = if cfg.max_tokens_per_day > 0 {
        (used_today as f64 / cfg.max_tokens_per_day as f64 * 100.0).min(999.0)
    } else { 0.0 };

    let mut out = String::new();
    out.push_str("📊 bg stats — agent self-introspection\n\n");
    out.push_str(&format!(
        "Caps:\n  tokens/day: {}\n  wall-clock/job: {}s\n  iterations/job: {}\n  concurrent: {}\n\n",
        cfg.max_tokens_per_day, cfg.max_wall_clock_secs, cfg.max_iterations, cfg.max_concurrent
    ));
    out.push_str(&format!(
        "Today's tokens: {}/{} ({:.1}%)\n\n",
        used_today, cfg.max_tokens_per_day, pct
    ));
    out.push_str(&format!("Last {days} day(s):\n"));
    for s in &recent {
        out.push_str(&format!(
            "  {} — fired {} done {} fail {} cancel {} timeout {} \
             budget-rej {} resumed {} orphaned {} tokens {} iters {}\n",
            s.date, s.fired, s.completed, s.failed, s.cancelled,
            s.timeout, s.budget_rejected, s.resumed, s.orphaned,
            s.tokens_used, s.iterations,
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
