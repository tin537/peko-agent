use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::{info, warn, error};
use reqwest::Client;
use serde::{Deserialize, Serialize};

use peko_config::TelegramConfig;
use peko_core::{AgentRuntime, AgentResponse, SessionStore, ToolRegistry, MemoryStore, SkillStore, SystemPrompt};
use peko_config::AgentConfig;
use peko_transport::LlmProvider;

const TELEGRAM_API: &str = "https://api.telegram.org";

pub struct TelegramBot {
    client: Client,
    token: String,
    config: TelegramConfig,
    app_config: Arc<Mutex<serde_json::Value>>,
    tools: Arc<ToolRegistry>,
    session_db_path: std::path::PathBuf,
    memory: Arc<Mutex<MemoryStore>>,
    skills: Arc<Mutex<SkillStore>>,
    soul: Arc<Mutex<String>>,
}

// ── Telegram API types ──

#[derive(Debug, Deserialize)]
struct TgResponse<T> {
    ok: bool,
    result: Option<T>,
    description: Option<String>,
}

#[derive(Debug, Deserialize)]
struct TgUpdate {
    update_id: i64,
    message: Option<TgMessage>,
}

#[derive(Debug, Deserialize)]
struct TgMessage {
    message_id: i64,
    from: Option<TgUser>,
    chat: TgChat,
    text: Option<String>,
    photo: Option<Vec<TgPhotoSize>>,
    caption: Option<String>,
}

#[derive(Debug, Deserialize)]
struct TgUser {
    id: i64,
    first_name: String,
    username: Option<String>,
}

#[derive(Debug, Deserialize)]
struct TgChat {
    id: i64,
}

#[derive(Debug, Deserialize)]
struct TgPhotoSize {
    file_id: String,
    width: i32,
    height: i32,
}

#[derive(Serialize)]
struct SendMessage {
    chat_id: i64,
    text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    parse_mode: Option<String>,
}

impl TelegramBot {
    pub fn new(
        config: TelegramConfig,
        app_config: Arc<Mutex<serde_json::Value>>,
        tools: Arc<ToolRegistry>,
        session_db_path: std::path::PathBuf,
        memory: Arc<Mutex<MemoryStore>>,
        skills: Arc<Mutex<SkillStore>>,
        soul: Arc<Mutex<String>>,
    ) -> Self {
        Self {
            client: Client::builder()
                .timeout(std::time::Duration::from_secs(60))
                .build().unwrap(),
            token: config.bot_token.clone(),
            config,
            app_config,
            tools,
            session_db_path,
            memory,
            skills,
            soul,
        }
    }

    fn api_url(&self, method: &str) -> String {
        format!("{}/bot{}/{}", TELEGRAM_API, self.token, method)
    }

    /// Main polling loop — runs forever
    pub async fn run(&self) {
        info!("telegram bot starting (long-polling)");

        // Verify bot token
        match self.get_me().await {
            Ok(name) => info!(bot = %name, "telegram bot connected"),
            Err(e) => {
                error!(error = %e, "telegram bot token invalid — stopping");
                return;
            }
        }

        let mut offset: i64 = 0;

        loop {
            match self.get_updates(offset).await {
                Ok(updates) => {
                    for update in updates {
                        offset = update.update_id + 1;
                        if let Some(msg) = update.message {
                            self.handle_message(msg).await;
                        }
                    }
                }
                Err(e) => {
                    warn!(error = %e, "telegram polling error, retrying in 5s");
                    tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                }
            }
        }
    }

    async fn get_me(&self) -> anyhow::Result<String> {
        let resp: TgResponse<serde_json::Value> = self.client
            .get(self.api_url("getMe"))
            .send().await?
            .json().await?;

        if resp.ok {
            let name = resp.result
                .and_then(|r| r["first_name"].as_str().map(String::from))
                .unwrap_or_else(|| "unknown".to_string());
            Ok(name)
        } else {
            anyhow::bail!("getMe failed: {}", resp.description.unwrap_or_default())
        }
    }

    async fn get_updates(&self, offset: i64) -> anyhow::Result<Vec<TgUpdate>> {
        let resp: TgResponse<Vec<TgUpdate>> = self.client
            .get(self.api_url("getUpdates"))
            .query(&[
                ("offset", offset.to_string()),
                ("timeout", "30".to_string()),
                ("allowed_updates", "[\"message\"]".to_string()),
            ])
            .send().await?
            .json().await?;

        Ok(resp.result.unwrap_or_default())
    }

    async fn send_text(&self, chat_id: i64, text: &str) {
        // Split long messages (Telegram limit: 4096 chars)
        let max_len = self.config.max_message_length.min(4096);
        let chunks: Vec<&str> = if text.len() <= max_len {
            vec![text]
        } else {
            text.as_bytes()
                .chunks(max_len)
                .map(|chunk| std::str::from_utf8(chunk).unwrap_or("..."))
                .collect()
        };

        for chunk in chunks {
            let msg = SendMessage {
                chat_id,
                text: chunk.to_string(),
                parse_mode: None,
            };

            if let Err(e) = self.client
                .post(self.api_url("sendMessage"))
                .json(&msg)
                .send().await
            {
                error!(error = %e, "failed to send telegram message");
            }
        }
    }

    async fn send_photo(&self, chat_id: i64, photo_bytes: Vec<u8>, caption: &str) {
        let part = reqwest::multipart::Part::bytes(photo_bytes)
            .file_name("screenshot.png")
            .mime_str("image/png").unwrap();

        let form = reqwest::multipart::Form::new()
            .text("chat_id", chat_id.to_string())
            .text("caption", caption.to_string())
            .part("photo", part);

        if let Err(e) = self.client
            .post(self.api_url("sendPhoto"))
            .multipart(form)
            .send().await
        {
            error!(error = %e, "failed to send telegram photo");
        }
    }

    async fn send_typing(&self, chat_id: i64) {
        let _ = self.client
            .post(self.api_url("sendChatAction"))
            .json(&serde_json::json!({"chat_id": chat_id, "action": "typing"}))
            .send().await;
    }

    async fn handle_message(&self, msg: TgMessage) {
        let chat_id = msg.chat.id;
        let user_id = msg.from.as_ref().map(|u| u.id).unwrap_or(0);
        let username = msg.from.as_ref()
            .and_then(|u| u.username.clone())
            .unwrap_or_else(|| msg.from.as_ref().map(|u| u.first_name.clone()).unwrap_or_default());

        // Auth check
        if !self.config.allowed_users.is_empty() && !self.config.allowed_users.contains(&user_id) {
            warn!(user_id, username = %username, "unauthorized telegram user");
            self.send_text(chat_id, "Unauthorized. Your user ID is not in the allowed list.").await;
            return;
        }

        let text = msg.text.or(msg.caption).unwrap_or_default();
        if text.is_empty() { return; }

        info!(user = %username, task = %text, "telegram task received");

        // Handle commands
        if text.starts_with('/') {
            self.handle_command(chat_id, &text).await;
            return;
        }

        // Run agent task
        self.send_typing(chat_id).await;

        match self.run_agent_task(&text).await {
            Ok(response) => {
                // Send response text
                if !response.text.is_empty() {
                    self.send_text(chat_id, &response.text).await;
                } else {
                    self.send_text(chat_id, "(task completed with no text response)").await;
                }

                // Send iteration info
                self.send_text(chat_id, &format!(
                    "[{} iterations | session: {}]",
                    response.iterations, &response.session_id[..8]
                )).await;
            }
            Err(e) => {
                self.send_text(chat_id, &format!("Error: {}", e)).await;
            }
        }
    }

    async fn handle_command(&self, chat_id: i64, cmd: &str) {
        let parts: Vec<&str> = cmd.splitn(2, ' ').collect();
        match parts[0] {
            "/start" | "/help" => {
                self.send_text(chat_id,
                    "Peko Agent — Android Agent-as-OS\n\n\
                     Send me any task and I'll execute it on the device.\n\n\
                     Commands:\n\
                     /status — Device status and memory\n\
                     /screenshot — Take a screenshot\n\
                     /memories — List saved memories\n\
                     /skills — List learned skills\n\
                     /apps — List installed apps\n\
                     /help — This message"
                ).await;
            }
            "/status" => {
                let mem = peko_core::MemMonitor::snapshot();
                let mem_count = self.memory.lock().await.count().unwrap_or(0);
                let skill_count = self.skills.lock().await.count();
                self.send_text(chat_id, &format!(
                    "Status: ready\nRSS: {}MB\nMemories: {}\nSkills: {}\nTools: {}",
                    mem["rss_mb"].as_str().unwrap_or("?"),
                    mem_count, skill_count,
                    self.tools.available_tools().len()
                )).await;
            }
            "/screenshot" => {
                self.send_typing(chat_id).await;
                match self.take_screenshot().await {
                    Ok(png) => self.send_photo(chat_id, png, "Current screen").await,
                    Err(e) => self.send_text(chat_id, &format!("Screenshot failed: {}", e)).await,
                }
            }
            "/memories" => {
                let store = self.memory.lock().await;
                match store.list(20, None) {
                    Ok(mems) if mems.is_empty() => {
                        self.send_text(chat_id, "No memories saved yet.").await;
                    }
                    Ok(mems) => {
                        let mut text = format!("{} memories:\n\n", mems.len());
                        for m in &mems {
                            text.push_str(&format!("[{}] {}: {}\n", m.category, m.key, m.content));
                        }
                        self.send_text(chat_id, &text).await;
                    }
                    Err(e) => self.send_text(chat_id, &format!("Error: {}", e)).await,
                }
            }
            "/skills" => {
                let store = self.skills.lock().await;
                let skills = store.list();
                if skills.is_empty() {
                    self.send_text(chat_id, "No skills learned yet.").await;
                } else {
                    let mut text = format!("{} skills:\n\n", skills.len());
                    for s in &skills {
                        text.push_str(&format!(
                            "• {} — {} ({:.0}% success)\n",
                            s.name, s.description, s.success_rate()
                        ));
                    }
                    self.send_text(chat_id, &text).await;
                }
            }
            "/apps" => {
                self.send_typing(chat_id).await;
                let output = std::process::Command::new("pm")
                    .args(["list", "packages", "-3"])
                    .output();
                match output {
                    Ok(o) => {
                        let text = String::from_utf8_lossy(&o.stdout);
                        let apps: Vec<&str> = text.lines()
                            .filter_map(|l| l.strip_prefix("package:"))
                            .collect();
                        self.send_text(chat_id, &format!("{} user apps:\n{}", apps.len(), apps.join("\n"))).await;
                    }
                    Err(e) => self.send_text(chat_id, &format!("Error: {}", e)).await,
                }
            }
            _ => {
                self.send_text(chat_id, "Unknown command. Try /help").await;
            }
        }
    }

    async fn run_agent_task(&self, input: &str) -> anyhow::Result<AgentResponse> {
        let config = self.app_config.lock().await.clone();

        // Build provider
        let provider = crate::web::api::build_provider_from_json_pub(&config)?;

        let session = SessionStore::open(&self.session_db_path)?;

        let agent_config = AgentConfig {
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

    async fn take_screenshot(&self) -> anyhow::Result<Vec<u8>> {
        let output = tokio::process::Command::new("screencap")
            .arg("-p")
            .output().await?;

        if output.status.success() {
            Ok(output.stdout)
        } else {
            anyhow::bail!("screencap failed")
        }
    }
}
