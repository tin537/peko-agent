use std::sync::Arc;
use tokio::sync::Mutex;
use chrono::{Utc, Datelike, Timelike};
use tracing::{info, warn, error};
use serde::{Deserialize, Serialize};

use crate::cron::CronExpr;
use crate::memory::MemoryStore;
use crate::skills::SkillStore;
use crate::runtime::AgentRuntime;
use crate::session::SessionStore;
use crate::tool::ToolRegistry;
use crate::prompt::SystemPrompt;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScheduledTask {
    pub name: String,
    pub cron: String,
    pub task: String,
    #[serde(default = "default_notify")]
    pub notify: String,
    #[serde(default = "bool_true")]
    pub enabled: bool,
    // Runtime state (not serialized to config)
    #[serde(skip)]
    pub last_run: Option<String>,
    #[serde(skip)]
    pub run_count: u64,
    #[serde(skip)]
    pub last_result: Option<String>,
    #[serde(skip)]
    pub last_error: Option<String>,
}

fn default_notify() -> String { "log".to_string() }
fn bool_true() -> bool { true }

pub struct Scheduler {
    tasks: Arc<Mutex<Vec<ScheduledTask>>>,
    parsed_crons: Vec<Option<CronExpr>>,
    tools: Arc<ToolRegistry>,
    config: Arc<Mutex<serde_json::Value>>,
    session_db_path: std::path::PathBuf,
    memory: Arc<Mutex<MemoryStore>>,
    skills: Arc<Mutex<SkillStore>>,
    soul: Arc<Mutex<String>>,
    telegram_sender: Option<TelegramSender>,
}

/// Minimal Telegram sender for notifications
#[derive(Clone)]
pub struct TelegramSender {
    client: reqwest::Client,
    token: String,
    chat_ids: Vec<i64>,
}

impl TelegramSender {
    pub fn new(token: String, chat_ids: Vec<i64>) -> Self {
        Self {
            client: reqwest::Client::new(),
            token,
            chat_ids,
        }
    }

    pub async fn send(&self, text: &str) {
        for chat_id in &self.chat_ids {
            let url = format!("https://api.telegram.org/bot{}/sendMessage", self.token);
            let _ = self.client.post(&url)
                .json(&serde_json::json!({
                    "chat_id": chat_id,
                    "text": text,
                }))
                .send().await;
        }
    }
}

impl Scheduler {
    pub fn new(
        tasks: Vec<ScheduledTask>,
        tools: Arc<ToolRegistry>,
        config: Arc<Mutex<serde_json::Value>>,
        session_db_path: std::path::PathBuf,
        memory: Arc<Mutex<MemoryStore>>,
        skills: Arc<Mutex<SkillStore>>,
        soul: Arc<Mutex<String>>,
        telegram_sender: Option<TelegramSender>,
    ) -> Self {
        // Parse every cron expression up front. A bad expression used
        // to silently set Some(c)→None and the task would never fire,
        // leaving the operator wondering why their daily research cron
        // produced no notes. Now we also stamp the task with a clear
        // last_error string so /api/schedule surfaces broken tasks
        // visibly. Parse failures NO LONGER block startup — disabled
        // tasks are still listed so the operator can fix and reload —
        // but they're loud at error level + visible in the API.
        let mut tasks = tasks;
        let parsed_crons: Vec<Option<CronExpr>> = tasks.iter_mut().map(|t| {
            match CronExpr::parse(&t.cron) {
                Ok(c) => {
                    info!(name = %t.name, cron = %t.cron, desc = %c.describe(), "scheduled task registered");
                    Some(c)
                }
                Err(e) => {
                    error!(name = %t.name, cron = %t.cron, error = %e,
                        "invalid cron expression — task will NEVER fire until fixed");
                    t.enabled = false;
                    t.last_error = Some(format!("invalid cron '{}': {e}", t.cron));
                    None
                }
            }
        }).collect();

        Self {
            tasks: Arc::new(Mutex::new(tasks)),
            parsed_crons,
            tools,
            config,
            session_db_path,
            memory,
            skills,
            soul,
            telegram_sender,
        }
    }

    /// Get a handle to the task list for the API
    pub fn tasks_handle(&self) -> Arc<Mutex<Vec<ScheduledTask>>> {
        self.tasks.clone()
    }

    /// Main scheduler loop — runs forever, checks every 60 seconds
    pub async fn run(&self) {
        info!(task_count = self.parsed_crons.len(), "scheduler started");

        // Wait until the start of the next minute for precise alignment
        let now = Utc::now();
        let secs_until_next_minute = 60 - now.second() as u64;
        tokio::time::sleep(std::time::Duration::from_secs(secs_until_next_minute)).await;

        loop {
            self.check_and_run().await;
            // Sleep until start of next minute
            let now = Utc::now();
            let secs_remaining = 60u64.saturating_sub(now.second() as u64);
            tokio::time::sleep(std::time::Duration::from_secs(secs_remaining.max(55))).await;
        }
    }

    async fn check_and_run(&self) {
        let mut tasks = self.tasks.lock().await;

        for (i, cron) in self.parsed_crons.iter().enumerate() {
            let Some(ref cron_expr) = cron else { continue };
            let task = &tasks[i];

            if !task.enabled { continue; }
            if !cron_expr.matches_now() { continue; }

            // Prevent running same task twice in the same minute
            let now_key = Utc::now().format("%Y-%m-%d %H:%M").to_string();
            if task.last_run.as_deref() == Some(&now_key) { continue; }

            info!(name = %task.name, task = %task.task, "scheduled task triggered");

            let task_input = task.task.clone();
            let task_name = task.name.clone();
            let notify = task.notify.clone();

            // Mark as running
            tasks[i].last_run = Some(now_key);
            tasks[i].run_count += 1;

            // Release lock before running the agent
            drop(tasks);

            // Run the task
            let result = self.execute_task(&task_input).await;

            // Store result
            let mut tasks = self.tasks.lock().await;
            match &result {
                Ok(response) => {
                    let text = if response.text.is_empty() {
                        format!("[{}] completed in {} iterations", task_name, response.iterations)
                    } else {
                        response.text.clone()
                    };
                    tasks[i].last_result = Some(text.clone());
                    tasks[i].last_error = None;
                    info!(name = %task_name, iterations = response.iterations, "scheduled task completed");

                    // Deliver notification
                    self.deliver(&notify, &task_name, &text).await;
                }
                Err(e) => {
                    let err_msg = format!("{}", e);
                    tasks[i].last_error = Some(err_msg.clone());
                    error!(name = %task_name, error = %e, "scheduled task failed");
                    self.deliver(&notify, &task_name, &format!("ERROR: {}", err_msg)).await;
                }
            }

            // Only run one task per check cycle to avoid contention
            return;
        }
    }

    async fn execute_task(&self, input: &str) -> anyhow::Result<crate::runtime::AgentResponse> {
        let config = self.config.lock().await.clone();

        let provider = crate::runtime::build_provider_helper(&config)?;
        let session = SessionStore::open(&self.session_db_path)?;

        let agent_config = peko_config::AgentConfig {
            max_iterations: config["agent"]["max_iterations"].as_u64().unwrap_or(50) as usize,
            context_window: config["agent"]["context_window"].as_u64().unwrap_or(200000) as usize,
            history_share: config["agent"]["history_share"].as_f64().unwrap_or(0.7) as f32,
            data_dir: std::path::PathBuf::from(
                config["agent"]["data_dir"].as_str().unwrap_or("/data/peko")
            ),
            log_level: "info".to_string(),
        };

        let soul_text = self.soul.lock().await.clone();
        let prompt = SystemPrompt::new().with_soul(soul_text);

        let mut runtime = AgentRuntime::new(
            &agent_config,
            self.tools.clone(),
            provider,
            session,
        )
        .with_system_prompt(prompt)
        .with_memory(self.memory.clone())
        .with_skills(self.skills.clone());

        runtime.run_task(input).await
    }

    async fn deliver(&self, method: &str, task_name: &str, text: &str) {
        match method {
            "telegram" => {
                if let Some(ref sender) = self.telegram_sender {
                    let msg = format!("[Scheduled: {}]\n\n{}", task_name, text);
                    sender.send(&msg).await;
                } else {
                    warn!("telegram delivery requested but no bot configured");
                }
            }
            "log" | _ => {
                info!(task = %task_name, "scheduled result: {}", &text[..text.len().min(200)]);
            }
        }
    }
}
