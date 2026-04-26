use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;
use tracing::{info, warn, error};
use reqwest::Client;
use serde::{Deserialize, Serialize};

use peko_config::TelegramConfig;
use peko_core::{AgentRuntime, AgentResponse, SessionStore, ToolRegistry, MemoryStore, SkillStore, SystemPrompt};
use peko_config::AgentConfig;
use peko_transport::LlmProvider;

const TELEGRAM_API: &str = "https://api.telegram.org";

/// Char-aware string truncation. Defensive against multi-byte UTF-8
/// (Thai / CJK / emoji): byte-indexed slicing here would panic the
/// whole tokio worker the same way phase13's compressor bug did.
fn truncate_chars(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        return s.to_string();
    }
    s.chars().take(max_chars).collect::<String>() + "…"
}

pub struct TelegramBot {
    client: Client,
    token: String,
    config: TelegramConfig,
    app_config: Arc<Mutex<serde_json::Value>>,
    /// Tool registry the bot uses to execute agent tasks. May be a
    /// narrowed view of the full registry — see TelegramConfig::allowed_tools
    /// and main.rs's bot construction.
    tools: Arc<ToolRegistry>,
    session_db_path: std::path::PathBuf,
    memory: Arc<Mutex<MemoryStore>>,
    skills: Arc<Mutex<SkillStore>>,
    soul: Arc<Mutex<String>>,
    /// Per-user sliding-window timestamps of recent task starts.
    /// Used to enforce TelegramConfig::rate_limit_per_minute.
    rate_window: Mutex<HashMap<i64, Vec<Instant>>>,
    /// Per-chat conversation memory: the last few (user_input, agent_text)
    /// turns. Pseudo-continuity — full session resume via SessionStore +
    /// run_turn would need a runtime refactor; the prepend-context
    /// approach gives multi-turn coherence at the cost of a single
    /// truncated string passed back to the LLM each call.
    chat_history: Mutex<HashMap<i64, Vec<(String, String)>>>,
}

const CHAT_HISTORY_KEEP_TURNS: usize = 3;
const CHAT_HISTORY_PER_TURN_CHARS: usize = 1500;

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
            rate_window: Mutex::new(HashMap::new()),
            chat_history: Mutex::new(HashMap::new()),
        }
    }

    /// Build the LLM-input string for a chat: optionally prefixed with
    /// the last few turns so multi-turn references like "scroll down"
    /// or "tap the second one" have context.
    async fn build_continuation_input(&self, chat_id: i64, user_input: &str) -> String {
        let history = self.chat_history.lock().await;
        let Some(turns) = history.get(&chat_id) else {
            return user_input.to_string();
        };
        if turns.is_empty() {
            return user_input.to_string();
        }
        let mut buf = String::from("[Recent turns in this chat — for context only, the user's latest request is at the bottom]\n\n");
        for (u, a) in turns.iter() {
            // Char-aware truncation per turn so big tool results don't
            // explode the prefix. The compressor in the runtime will
            // dedupe / trim further if needed.
            let u_short = truncate_chars(u, CHAT_HISTORY_PER_TURN_CHARS);
            let a_short = truncate_chars(a, CHAT_HISTORY_PER_TURN_CHARS);
            buf.push_str(&format!("USER: {u_short}\nAGENT: {a_short}\n\n"));
        }
        buf.push_str("---\nUser's latest request:\n");
        buf.push_str(user_input);
        buf
    }

    /// Append a (user, agent) pair to the chat's history, dropping the
    /// oldest turn once the deque is full. Called from `handle_message`
    /// after a successful agent response.
    async fn record_turn(&self, chat_id: i64, user_input: String, agent_text: String) {
        let mut history = self.chat_history.lock().await;
        let entry = history.entry(chat_id).or_insert_with(Vec::new);
        entry.push((user_input, agent_text));
        let len = entry.len();
        if len > CHAT_HISTORY_KEEP_TURNS {
            entry.drain(0..len - CHAT_HISTORY_KEEP_TURNS);
        }
    }

    async fn reset_chat_history(&self, chat_id: i64) {
        let mut history = self.chat_history.lock().await;
        history.remove(&chat_id);
    }

    fn api_url(&self, method: &str) -> String {
        format!("{}/bot{}/{}", TELEGRAM_API, self.token, method)
    }

    /// Same shape as `api_url` but with the token replaced by
    /// `[REDACTED]`. ALWAYS use this in tracing / error / debug output
    /// so a `RUST_LOG=trace` flip doesn't dump the bot token into
    /// peko.log.
    fn safe_url(&self, method: &str) -> String {
        format!("{}/bot[REDACTED]/{}", TELEGRAM_API, method)
    }

    /// Returns true if `user_id` is currently within the per-minute
    /// rate budget; false if the user is throttled. Garbage-collects
    /// timestamps older than 60s on every call so the map doesn't
    /// grow unbounded.
    async fn check_rate_limit(&self, user_id: i64) -> bool {
        let limit = self.config.rate_limit_per_minute;
        if limit == 0 {
            // Operator explicitly disabled. Document it as a footgun
            // by warn-logging on every overshoot is too noisy, so
            // just let them through.
            return true;
        }
        let now = Instant::now();
        let mut win = self.rate_window.lock().await;
        let entry = win.entry(user_id).or_insert_with(Vec::new);
        // Drop timestamps older than 60s.
        entry.retain(|t| now.duration_since(*t) < Duration::from_secs(60));
        if entry.len() as u32 >= limit {
            return false;
        }
        entry.push(now);
        true
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
        let url = self.api_url("getMe");
        // self.safe_url(...) is what we'd put in any error_chain or
        // tracing::debug call. Reqwest itself never logs URLs at info
        // level, so this is defence-in-depth for ops who flip
        // RUST_LOG=trace later.
        let resp: TgResponse<serde_json::Value> = match self.client
            .get(url)
            .send().await {
                Ok(r) => r.json().await?,
                Err(e) => {
                    error!(url = %self.safe_url("getMe"), error = %e, "getMe request failed");
                    return Err(e.into());
                }
            };

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

        // Auth check. Defence-in-depth: main.rs refuses to spawn the
        // bot at all when allowed_users is empty, but we re-check here
        // so a future code path that bypasses that gate still hits
        // this one.
        if self.config.allowed_users.is_empty() {
            warn!(user_id, username = %username, "telegram bot received message but allowed_users is empty — dropping");
            return;
        }
        if !self.config.allowed_users.contains(&user_id) {
            warn!(user_id, username = %username, "unauthorized telegram user");
            self.send_text(chat_id, "Unauthorized. Your user ID is not in the allowed list.").await;
            return;
        }

        // Rate limit. Drop the message politely with a hint about the
        // configured cap. We rate-limit BEFORE doing any agent work so
        // a flood doesn't burn LLM credits even on the rejection path.
        if !self.check_rate_limit(user_id).await {
            warn!(user_id, username = %username, limit = self.config.rate_limit_per_minute,
                  "telegram rate limit exceeded");
            self.send_text(chat_id, &format!(
                "Rate limit hit ({}/min). Slow down and try again.",
                self.config.rate_limit_per_minute
            )).await;
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

        // Run agent task — prepend recent turns from this chat as
        // context so multi-turn references work ("scroll down", "tap
        // the second one"). The agent's own context compressor will
        // trim if the prefix is too large.
        self.send_typing(chat_id).await;

        let agent_input = self.build_continuation_input(chat_id, &text).await;

        match self.run_agent_task(&agent_input).await {
            Ok(response) => {
                let agent_text = if response.text.is_empty() {
                    "(task completed with no text response)".to_string()
                } else {
                    response.text.clone()
                };
                self.send_text(chat_id, &agent_text).await;

                self.send_text(chat_id, &format!(
                    "[{} iterations | session: {}]",
                    response.iterations, &response.session_id[..8]
                )).await;

                // Record the turn so the next message in this chat
                // gets it in its prefix. Failures during agent task
                // execution are NOT recorded — keeping the history
                // clean of error noise.
                self.record_turn(chat_id, text.clone(), agent_text).await;
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
                     /new — Reset chat memory (start a fresh conversation)\n\
                     /help — This message\n\n\
                     Multi-turn: I remember the last 3 turns in this chat — \
                     so you can say things like \"scroll down\" or \"tap the \
                     second one\" without restating context. Use /new to \
                     start over."
                ).await;
            }
            "/new" => {
                self.reset_chat_history(chat_id).await;
                self.send_text(chat_id, "Chat memory cleared. Fresh start.").await;
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

#[cfg(test)]
mod tests {
    use super::*;
    use peko_config::TelegramConfig;

    fn test_bot(rate: u32) -> TelegramBot {
        let cfg = TelegramConfig {
            bot_token: "TEST".into(),
            allowed_users: vec![123],
            send_screenshots: true,
            max_message_length: 4000,
            rate_limit_per_minute: rate,
            allowed_tools: None,
        };
        // Per-process unique temp dir so concurrent test runs don't
        // clobber each other's SQLite files. We never read these
        // back — the rate-limiter / safe_url tests only touch the
        // bot's in-memory state.
        let pid = std::process::id();
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.subsec_nanos())
            .unwrap_or(0);
        let tmp = std::env::temp_dir().join(format!("peko-tg-test-{pid}-{nanos}"));
        std::fs::create_dir_all(&tmp).ok();
        let mem_db = tmp.join("memory.sqlite");
        let skills_dir = tmp.join("skills");
        std::fs::create_dir_all(&skills_dir).ok();
        TelegramBot::new(
            cfg,
            Arc::new(Mutex::new(serde_json::json!({}))),
            Arc::new(ToolRegistry::new()),
            tmp.join("session.sqlite"),
            Arc::new(Mutex::new(MemoryStore::open(&mem_db).unwrap())),
            Arc::new(Mutex::new(SkillStore::open(&skills_dir).unwrap())),
            Arc::new(Mutex::new(String::new())),
        )
    }

    #[tokio::test]
    async fn rate_limit_allows_under_cap() {
        let bot = test_bot(5);
        for _ in 0..5 {
            assert!(bot.check_rate_limit(123).await, "should allow within cap");
        }
        assert!(!bot.check_rate_limit(123).await, "6th call must be throttled");
    }

    #[tokio::test]
    async fn rate_limit_zero_disables_check() {
        let bot = test_bot(0);
        for _ in 0..1000 {
            assert!(bot.check_rate_limit(123).await, "rate=0 means no limit");
        }
    }

    #[tokio::test]
    async fn rate_limit_per_user_independent() {
        let bot = test_bot(2);
        assert!(bot.check_rate_limit(111).await);
        assert!(bot.check_rate_limit(111).await);
        // user 111 is now at cap; user 222 still has full budget
        assert!(!bot.check_rate_limit(111).await);
        assert!(bot.check_rate_limit(222).await);
        assert!(bot.check_rate_limit(222).await);
        assert!(!bot.check_rate_limit(222).await);
    }

    #[test]
    fn safe_url_redacts_token() {
        let bot = test_bot(5);
        let s = bot.safe_url("getMe");
        assert!(!s.contains("TEST"), "real token must not appear in safe_url");
        assert!(s.contains("[REDACTED]"));
        assert!(s.ends_with("/getMe"));
    }
}
