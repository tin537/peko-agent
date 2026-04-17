use std::sync::Arc;
use tokio::sync::{mpsc, Mutex, oneshot};
use tracing::{info, warn, error};

use crate::runtime::{AgentRuntime, AgentResponse, StreamCallback};
use crate::message::Message;
use crate::session::SessionStore;
use crate::memory::MemoryStore;
use crate::skills::SkillStore;
use crate::user_model::UserModel;
use crate::prompt::SystemPrompt;
use crate::tool::ToolRegistry;
use crate::runtime::build_provider_helper;
use peko_config::AgentConfig;

/// A task submitted to the queue
pub struct TaskRequest {
    pub input: String,
    pub session_id: Option<String>,
    pub source: TaskSource,
    /// Channel to stream events back to the caller
    pub stream_tx: mpsc::Sender<StreamCallback>,
    /// Oneshot to send the final result
    pub result_tx: oneshot::Sender<Result<AgentResponse, String>>,
}

#[derive(Debug, Clone)]
pub enum TaskSource {
    WebUI,
    Telegram { chat_id: i64 },
    Scheduler { task_name: String },
    Api,
}

impl std::fmt::Display for TaskSource {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            Self::WebUI => write!(f, "web"),
            Self::Telegram { chat_id } => write!(f, "telegram:{}", chat_id),
            Self::Scheduler { task_name } => write!(f, "schedule:{}", task_name),
            Self::Api => write!(f, "api"),
        }
    }
}

/// Queue status for monitoring
#[derive(Debug, Clone, serde::Serialize)]
pub struct QueueStatus {
    pub pending: usize,
    pub processing: bool,
    pub total_processed: u64,
    pub current_task: Option<String>,
    pub current_source: Option<String>,
}

/// The task queue — submit tasks, they execute one at a time
pub struct TaskQueue {
    submit_tx: mpsc::Sender<TaskRequest>,
    status: Arc<Mutex<QueueStatus>>,
}

impl TaskQueue {
    /// Create a new task queue and spawn the executor loop.
    /// All shared state is passed here — the executor owns the runtime.
    pub fn new(
        tools: Arc<ToolRegistry>,
        config: Arc<Mutex<serde_json::Value>>,
        session_db_path: std::path::PathBuf,
        memory: Arc<Mutex<MemoryStore>>,
        skills: Arc<Mutex<SkillStore>>,
        soul: Arc<Mutex<String>>,
        user_model: Arc<Mutex<UserModel>>,
        user_model_path: std::path::PathBuf,
        max_queue_size: usize,
    ) -> Self {
        let (submit_tx, submit_rx) = mpsc::channel::<TaskRequest>(max_queue_size);

        let status = Arc::new(Mutex::new(QueueStatus {
            pending: 0,
            processing: false,
            total_processed: 0,
            current_task: None,
            current_source: None,
        }));

        let status_clone = status.clone();

        // Spawn the single executor
        tokio::spawn(Self::executor_loop(
            submit_rx,
            status_clone,
            tools,
            config,
            session_db_path,
            memory,
            skills,
            soul,
            user_model,
            user_model_path,
        ));

        info!("task queue started (max queue: {})", max_queue_size);

        Self { submit_tx, status }
    }

    /// Submit a task to the queue. Returns immediately.
    /// Events stream back via the stream_tx channel.
    /// Final result arrives via result_tx oneshot.
    pub async fn submit(&self, request: TaskRequest) -> Result<(), String> {
        // Update pending count
        {
            let mut s = self.status.lock().await;
            s.pending += 1;
        }

        self.submit_tx.send(request).await
            .map_err(|_| "task queue is full or closed".to_string())
    }

    /// Submit and wait for result (convenience method)
    pub async fn submit_and_wait(
        &self,
        input: String,
        session_id: Option<String>,
        source: TaskSource,
    ) -> (mpsc::Receiver<StreamCallback>, oneshot::Receiver<Result<AgentResponse, String>>) {
        let (stream_tx, stream_rx) = mpsc::channel(64);
        let (result_tx, result_rx) = oneshot::channel();

        let request = TaskRequest {
            input,
            session_id,
            source,
            stream_tx,
            result_tx,
        };

        let _ = self.submit(request).await;

        (stream_rx, result_rx)
    }

    /// Get current queue status
    pub async fn status(&self) -> QueueStatus {
        self.status.lock().await.clone()
    }

    /// The single executor loop — processes tasks one at a time
    async fn executor_loop(
        mut rx: mpsc::Receiver<TaskRequest>,
        status: Arc<Mutex<QueueStatus>>,
        tools: Arc<ToolRegistry>,
        config: Arc<Mutex<serde_json::Value>>,
        session_db_path: std::path::PathBuf,
        memory: Arc<Mutex<MemoryStore>>,
        skills: Arc<Mutex<SkillStore>>,
        soul: Arc<Mutex<String>>,
        user_model: Arc<Mutex<UserModel>>,
        user_model_path: std::path::PathBuf,
    ) {
        info!("task queue executor started");

        while let Some(task) = rx.recv().await {
            // Update status
            {
                let mut s = status.lock().await;
                s.pending = s.pending.saturating_sub(1);
                s.processing = true;
                s.current_task = Some(task.input.clone());
                s.current_source = Some(task.source.to_string());
            }

            info!(
                source = %task.source,
                task = %task.input,
                "executing queued task"
            );

            // Build runtime for this task
            let result = Self::execute_task(
                &task,
                tools.clone(),
                config.clone(),
                &session_db_path,
                memory.clone(),
                skills.clone(),
                soul.clone(),
                user_model.clone(),
                user_model_path.clone(),
            ).await;

            // Send result
            let _ = task.result_tx.send(result);

            // Update status
            {
                let mut s = status.lock().await;
                s.processing = false;
                s.total_processed += 1;
                s.current_task = None;
                s.current_source = None;
            }
        }

        warn!("task queue executor stopped");
    }

    async fn execute_task(
        task: &TaskRequest,
        tools: Arc<ToolRegistry>,
        config: Arc<Mutex<serde_json::Value>>,
        session_db_path: &std::path::Path,
        memory: Arc<Mutex<MemoryStore>>,
        skills: Arc<Mutex<SkillStore>>,
        soul: Arc<Mutex<String>>,
        user_model: Arc<Mutex<UserModel>>,
        user_model_path: std::path::PathBuf,
    ) -> Result<AgentResponse, String> {
        let config_val = config.lock().await.clone();

        let provider = build_provider_helper(&config_val)
            .map_err(|e| format!("provider error: {}", e))?;

        let session = SessionStore::open(session_db_path)
            .map_err(|e| format!("session error: {}", e))?;

        let agent_config = AgentConfig {
            max_iterations: config_val["agent"]["max_iterations"].as_u64().unwrap_or(50) as usize,
            context_window: config_val["agent"]["context_window"].as_u64().unwrap_or(200000) as usize,
            history_share: config_val["agent"]["history_share"].as_f64().unwrap_or(0.7) as f32,
            data_dir: std::path::PathBuf::from(
                config_val["agent"]["data_dir"].as_str().unwrap_or("/data/peko")
            ),
            log_level: "info".to_string(),
        };

        let soul_text = soul.lock().await.clone();
        let prompt = SystemPrompt::new().with_soul(soul_text);

        let mut runtime = AgentRuntime::new(&agent_config, tools, provider, session)
            .with_system_prompt(prompt)
            .with_memory(memory)
            .with_skills(skills)
            .with_user_model(user_model, user_model_path);

        // Load or create session + conversation
        let (session_id, mut conversation) = if let Some(ref sid) = task.session_id {
            let messages = runtime.session.load_messages(sid).unwrap_or_default();
            let mut conv: Vec<Message> = Vec::new();
            for m in &messages {
                match m.role.as_str() {
                    "user" => conv.push(Message::user(&m.content)),
                    "assistant" => {
                        let tool_calls = m.tool_args.as_ref()
                            .and_then(|s| serde_json::from_str(s).ok())
                            .unwrap_or_default();
                        let text = if m.content.is_empty() { None } else { Some(m.content.clone()) };
                        conv.push(Message::assistant_tool_calls(text, tool_calls));
                    }
                    "tool_result" => {
                        conv.push(Message::tool_result(
                            m.tool_use_id.clone().unwrap_or_default(),
                            m.tool_name.clone().unwrap_or_default(),
                            m.content.clone(),
                            m.is_error,
                        ));
                    }
                    _ => {}
                }
            }
            (sid.clone(), conv)
        } else {
            let sid = runtime.session.create_session(&task.input)
                .map_err(|e| format!("session create error: {}", e))?;
            (sid, Vec::new())
        };

        runtime.run_turn(&session_id, &mut conversation, &task.input, task.stream_tx.clone())
            .await
            .map_err(|e| format!("{}", e))
    }
}

impl Clone for TaskQueue {
    fn clone(&self) -> Self {
        Self {
            submit_tx: self.submit_tx.clone(),
            status: self.status.clone(),
        }
    }
}
