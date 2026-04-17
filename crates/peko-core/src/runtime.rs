use std::sync::Arc;
use futures::StreamExt;
use tracing::{info, warn, error};

use peko_config::AgentConfig;
use peko_transport::{LlmProvider, StreamEvent, StopReason};

use crate::brain::{DualBrain, BrainChoice, ESCALATE_TOOL_NAME, escalate_tool_schema, build_escalation_context};
use crate::budget::IterationBudget;
use crate::compressor::ContextCompressor;
use crate::memory::MemoryStore;
use crate::skills::SkillStore;
use crate::user_model::UserModel;
use crate::message::{Message, ToolCall, ImageData};
use crate::prompt::SystemPrompt;
use crate::session::SessionStore;
use crate::tool::{ToolRegistry, ToolResult};

const MEMORY_NUDGE: &str = "\n\n[Memory Nudge] Consider:\
\n1. Should anything from this conversation be remembered? Use the memory tool.\
\n2. Did you complete a multi-step task? Save it as a skill for next time.\
\n3. Did a skill's steps fail? Improve it with the correct approach.";

pub struct AgentRuntime {
    tools: Arc<ToolRegistry>,
    budget: IterationBudget,
    compressor: ContextCompressor,
    pub session: SessionStore,
    provider: Box<dyn LlmProvider>,
    system_prompt: SystemPrompt,
    memory: Option<Arc<tokio::sync::Mutex<MemoryStore>>>,
    skills: Option<Arc<tokio::sync::Mutex<SkillStore>>>,
    user_model: Option<Arc<tokio::sync::Mutex<UserModel>>>,
    user_model_path: Option<std::path::PathBuf>,
    nudge_interval: usize,
    /// Dual-brain: if set, enables local/cloud routing with escalation.
    brain: Option<Arc<DualBrain>>,
    /// Which brain is currently active for this task.
    active_brain: Option<BrainChoice>,
}

#[derive(Debug)]
pub struct AgentResponse {
    pub text: String,
    pub iterations: usize,
    pub session_id: String,
}

/// Callback for streaming events to the UI
pub enum StreamCallback {
    TextDelta(String),
    ToolStart { name: String },
    ToolResult { name: String, content: String, is_error: bool, image: Option<(String, String)> },
    Thinking(String),
    /// Emitted once when the task completes.
    /// `brain` indicates which brain ultimately produced the answer:
    ///   "local"     — handled entirely by local
    ///   "cloud"     — handled entirely by cloud
    ///   "escalated" — started on local, escalated to cloud mid-task
    ///   None        — no brain configured (default provider)
    Done {
        iterations: usize,
        session_id: String,
        brain: Option<String>,
    },
    Error(String),
}

impl AgentRuntime {
    pub fn new(
        config: &AgentConfig,
        tools: Arc<ToolRegistry>,
        provider: Box<dyn LlmProvider>,
        session: SessionStore,
    ) -> Self {
        Self {
            tools: tools.clone(),
            budget: IterationBudget::new(config.max_iterations),
            compressor: ContextCompressor::new(config.context_window, config.history_share),
            session,
            provider,
            system_prompt: SystemPrompt::new(),
            memory: None,
            skills: None,
            user_model: None,
            user_model_path: None,
            nudge_interval: 5,
            brain: None,
            active_brain: None,
        }
    }

    pub fn with_system_prompt(mut self, prompt: SystemPrompt) -> Self {
        self.system_prompt = prompt;
        self
    }

    pub fn with_memory(mut self, store: Arc<tokio::sync::Mutex<MemoryStore>>) -> Self {
        self.memory = Some(store);
        self
    }

    pub fn with_skills(mut self, store: Arc<tokio::sync::Mutex<SkillStore>>) -> Self {
        self.skills = Some(store);
        self
    }

    pub fn with_user_model(mut self, model: Arc<tokio::sync::Mutex<UserModel>>, path: std::path::PathBuf) -> Self {
        self.user_model = Some(model);
        self.user_model_path = Some(path);
        self
    }

    pub fn with_nudge_interval(mut self, interval: usize) -> Self {
        self.nudge_interval = interval;
        self
    }

    pub fn with_brain(mut self, brain: Arc<DualBrain>) -> Self {
        self.brain = Some(brain);
        self
    }

    pub fn budget_handle(&self) -> IterationBudget {
        self.budget.clone()
    }

    pub async fn run_task(&mut self, user_input: &str) -> anyhow::Result<AgentResponse> {
        // Dual-brain classification
        let brain_choice = if let Some(ref brain) = self.brain {
            let skills_guard = match &self.skills {
                Some(s) => s.try_lock().ok(),
                None => None,
            };
            let choice = brain.classify(user_input, skills_guard.as_deref());
            self.active_brain = Some(choice.clone());
            info!(
                brain = %choice,
                local_model = brain.local_model_name(),
                cloud_model = brain.cloud_model_name(),
                "dual-brain task classification"
            );
            Some(choice)
        } else {
            None
        };

        self.budget.reset();

        let session_id = self.session.create_session(user_input)?;
        let mut conversation = vec![Message::user(user_input.to_string())];
        self.session.append_message(&session_id, &conversation[0])?;

        let mut total_iterations: usize = 0;
        let mut final_text = String::new();

        loop {
            if self.budget.should_stop() {
                info!(iterations = total_iterations, "budget exhausted or interrupted");
                break;
            }

            // Compress if needed
            self.compressor.check_and_compress(&mut conversation);

            // Build system prompt with memory injection
            let mut system_prompt_text = self.system_prompt.build(&self.tools);

            // Inject relevant memories on first iteration
            if total_iterations == 0 {
                if let Some(ref memory) = self.memory {
                    match memory.try_lock() {
                        Ok(mem_store) => {
                            match mem_store.build_context(user_input, 5) {
                                Ok(ctx) if !ctx.is_empty() => {
                                    system_prompt_text.push_str("\n\n");
                                    system_prompt_text.push_str(&ctx);
                                    info!(memories = ctx.lines().count() - 2, "injected memories into prompt");
                                }
                                _ => {}
                            }
                        }
                        Err(_) => warn!("memory store locked, skipping memory injection"),
                    }
                }
            }

            // Inject relevant skills on first iteration
            if total_iterations == 0 {
                if let Some(ref skills) = self.skills {
                    match skills.try_lock() {
                        Ok(skill_store) => {
                            let ctx = skill_store.build_context(user_input);
                            if !ctx.is_empty() {
                                system_prompt_text.push_str("\n\n");
                                system_prompt_text.push_str(&ctx);
                                info!("injected skills into prompt");
                            }
                        }
                        Err(_) => warn!("skill store locked, skipping skill injection"),
                    }
                }
            }

            // Inject user model context on first iteration
            if total_iterations == 0 {
                if let Some(ref user_model) = self.user_model {
                    match user_model.try_lock() {
                        Ok(model) => {
                            let ctx = model.build_context();
                            if !ctx.is_empty() {
                                system_prompt_text.push_str("\n\n");
                                system_prompt_text.push_str(&ctx);
                            }
                        }
                        Err(_) => warn!("user model locked, skipping user context injection"),
                    }
                }
            }

            // Add memory nudge periodically
            if self.memory.is_some()
                && self.nudge_interval > 0
                && total_iterations > 0
                && total_iterations % self.nudge_interval == 0
            {
                system_prompt_text.push_str(MEMORY_NUDGE);
            }

            // Convert to transport messages
            let transport_messages: Vec<peko_transport::provider::Message> = conversation
                .iter()
                .flat_map(|m| m.to_transport_messages())
                .collect();

            let mut tool_schemas = self.tools.schemas();

            // Inject the escalate tool ONLY when the brain's mode supports
            // escalation (Dual mode) AND we're currently on the local side.
            // LocalOnly / CloudOnly modes don't offer escalation because
            // there's no other brain to hand off to.
            let using_local = matches!(&self.active_brain, Some(BrainChoice::Local));
            let escalation_available = self.brain.as_ref()
                .map(|b| b.mode().supports_escalation())
                .unwrap_or(false);
            if using_local && escalation_available {
                tool_schemas.push(escalate_tool_schema());
            }

            // Select provider: brain-routed or default
            let active_provider: &dyn LlmProvider = match (&self.brain, &self.active_brain) {
                (Some(brain), Some(choice)) => brain.provider(choice),
                _ => self.provider.as_ref(),
            };

            // Stream completion
            let brain_label = self.active_brain.as_ref().map(|b| b.to_string()).unwrap_or_else(|| "default".to_string());
            info!(iteration = total_iterations + 1, brain = %brain_label, model = active_provider.model_name(), "calling LLM");
            let mut stream = active_provider.stream_completion(
                &system_prompt_text,
                &transport_messages,
                &tool_schemas,
            ).await?;

            // Accumulate response
            let mut text_buffer = String::new();
            let mut tool_calls: Vec<ToolCall> = Vec::new();
            let mut current_tool_id = String::new();
            let mut current_tool_name = String::new();
            let mut current_tool_input = String::new();
            let mut stop_reason = StopReason::EndTurn;

            while let Some(event) = stream.next().await {
                match event? {
                    StreamEvent::TextDelta(text) => {
                        text_buffer.push_str(&text);
                    }
                    StreamEvent::ToolUseStart { id, name } => {
                        current_tool_id = id;
                        current_tool_name = name;
                        current_tool_input.clear();
                    }
                    StreamEvent::ToolInputDelta(json) => {
                        current_tool_input.push_str(&json);
                    }
                    StreamEvent::ContentBlockStop { .. } => {
                        if !current_tool_name.is_empty() {
                            let input: serde_json::Value = serde_json::from_str(&current_tool_input)
                                .unwrap_or(serde_json::Value::Object(serde_json::Map::new()));
                            tool_calls.push(ToolCall {
                                id: current_tool_id.clone(),
                                name: current_tool_name.clone(),
                                input,
                            });
                            current_tool_name.clear();
                            current_tool_input.clear();
                        }
                    }
                    StreamEvent::MessageDelta { stop_reason: sr, .. } => {
                        stop_reason = sr;
                        // Finalize pending tool call (OpenAI format)
                        if !current_tool_name.is_empty() {
                            let input: serde_json::Value = serde_json::from_str(&current_tool_input)
                                .unwrap_or(serde_json::Value::Object(serde_json::Map::new()));
                            tool_calls.push(ToolCall {
                                id: current_tool_id.clone(),
                                name: current_tool_name.clone(),
                                input,
                            });
                            current_tool_name.clear();
                            current_tool_input.clear();
                        }
                    }
                    StreamEvent::ThinkingDelta(thought) => {
                        tracing::debug!(thought = %thought, "LLM thinking");
                    }
                    _ => {}
                }
            }

            // Finalize any unclosed tool call
            if !current_tool_name.is_empty() {
                let input: serde_json::Value = serde_json::from_str(&current_tool_input)
                    .unwrap_or(serde_json::Value::Object(serde_json::Map::new()));
                tool_calls.push(ToolCall {
                    id: current_tool_id.clone(),
                    name: current_tool_name.clone(),
                    input,
                });
            }

            // Build assistant message
            let text = if text_buffer.is_empty() { None } else { Some(text_buffer.clone()) };
            let assistant_msg = Message::assistant_tool_calls(text, tool_calls.clone());
            conversation.push(assistant_msg.clone());
            self.session.append_message(&session_id, &assistant_msg)?;

            if tool_calls.is_empty() {
                // No tool calls → final response (regardless of stop_reason)
                final_text = text_buffer;
                total_iterations += 1;
                break;
            }

            if stop_reason == StopReason::EndTurn && !text_buffer.is_empty() {
                // LLM said end_turn with text + tool calls — capture text as final
                final_text = text_buffer;
                total_iterations += 1;
                break;
            }

            // Execute each tool call
            let mut escalated = false;
            for tc in &tool_calls {
                // Intercept escalate tool — switch to cloud brain
                if tc.name == ESCALATE_TOOL_NAME {
                    if let Some(ref brain) = self.brain {
                        let reason = tc.input["reason"].as_str().unwrap_or("local model requested escalation");
                        let analysis = tc.input["analysis"].as_str();
                        let local_model = brain.local_model_name().to_string();

                        info!(
                            reason = reason,
                            local_model = %local_model,
                            cloud_model = brain.cloud_model_name(),
                            "ESCALATING to cloud brain"
                        );

                        // Build escalation context and restart conversation with cloud
                        let esc_context = build_escalation_context(
                            user_input, reason, analysis, &local_model,
                        );
                        conversation.clear();
                        conversation.push(Message::user(esc_context));
                        self.active_brain = Some(BrainChoice::Cloud);
                        self.budget.reset();
                        total_iterations = 0;
                        escalated = true;
                        break;
                    }
                }

                info!(tool = %tc.name, id = %tc.id, "executing tool");

                let result = match self.tools.execute(&tc.name, tc.input.clone()).await {
                    Ok(r) => r,
                    Err(e) => {
                        error!(tool = %tc.name, error = %e, "tool execution failed");
                        ToolResult::error(format!("Error: {}", e))
                    }
                };

                let tool_msg = if let Some(img) = result.image {
                    Message::tool_result_with_image(
                        tc.id.clone(),
                        tc.name.clone(),
                        result.content,
                        result.is_error,
                        img,
                    )
                } else {
                    Message::tool_result(
                        tc.id.clone(),
                        tc.name.clone(),
                        result.content,
                        result.is_error,
                    )
                };

                conversation.push(tool_msg.clone());
                self.session.append_message(&session_id, &tool_msg)?;
            }

            if escalated {
                continue; // restart loop with cloud provider
            }

            total_iterations += 1;

            if let Err(_) = self.budget.decrement() {
                warn!("iteration budget exhausted after {} iterations", total_iterations);
                break;
            }
        }

        let status = if self.budget.is_interrupted() { "interrupted" } else { "completed" };
        self.session.update_status(&session_id, status, total_iterations)?;

        // Update user model with this interaction
        if let Some(ref user_model) = self.user_model {
            if let Ok(mut model) = user_model.try_lock() {
                model.record_task(user_input, total_iterations);
                if let Some(ref path) = self.user_model_path {
                    let _ = model.save(path);
                }
            }
        }

        info!(
            session_id = %session_id,
            iterations = total_iterations,
            status = status,
            "task finished"
        );

        Ok(AgentResponse {
            text: final_text,
            iterations: total_iterations,
            session_id,
        })
    }

    /// Run a turn within an existing conversation, streaming events via a channel.
    /// This enables multi-turn conversations and real-time token streaming.
    pub async fn run_turn(
        &mut self,
        session_id: &str,
        conversation: &mut Vec<Message>,
        user_input: &str,
        tx: tokio::sync::mpsc::Sender<StreamCallback>,
    ) -> anyhow::Result<AgentResponse> {
        // Dual-brain classification (same as run_task)
        if let Some(ref brain) = self.brain {
            let skills_guard = match &self.skills {
                Some(s) => s.try_lock().ok(),
                None => None,
            };
            let choice = brain.classify(user_input, skills_guard.as_deref());
            self.active_brain = Some(choice.clone());
            info!(
                brain = %choice,
                local_model = brain.local_model_name(),
                cloud_model = brain.cloud_model_name(),
                "run_turn: dual-brain classification"
            );
        }

        self.budget.reset();

        // Track whether this task escalated mid-flight — used for the
        // per-message brain badge in the UI (local / cloud / escalated).
        let start_brain = self.active_brain.clone();
        let mut did_escalate = false;

        // Append user message
        let user_msg = Message::user(user_input.to_string());
        conversation.push(user_msg.clone());
        self.session.append_message(session_id, &user_msg)?;

        let mut total_iterations: usize = 0;
        let mut final_text = String::new();

        loop {
            if self.budget.should_stop() {
                break;
            }

            self.compressor.check_and_compress(conversation);

            let using_local_embedded = matches!(&self.active_brain, Some(BrainChoice::Local));
            let mut system_prompt_text = self.system_prompt.build(&self.tools);

            // For Qwen3 local brain, disable thinking mode to save tokens
            if using_local_embedded {
                system_prompt_text.push_str("\n\n/no_think");
            }

            // Inject memories
            if total_iterations == 0 {
                if let Some(ref memory) = self.memory {
                    if let Ok(mem_store) = memory.try_lock() {
                        if let Ok(ctx) = mem_store.build_context(user_input, 5) {
                            if !ctx.is_empty() {
                                system_prompt_text.push_str("\n\n");
                                system_prompt_text.push_str(&ctx);
                            }
                        }
                    }
                }
                if let Some(ref skills) = self.skills {
                    if let Ok(skill_store) = skills.try_lock() {
                        let ctx = skill_store.build_context(user_input);
                        if !ctx.is_empty() {
                            system_prompt_text.push_str("\n\n");
                            system_prompt_text.push_str(&ctx);
                        }
                    }
                }
                if let Some(ref user_model) = self.user_model {
                    if let Ok(model) = user_model.try_lock() {
                        let ctx = model.build_context();
                        if !ctx.is_empty() {
                            system_prompt_text.push_str("\n\n");
                            system_prompt_text.push_str(&ctx);
                        }
                    }
                }
            }

            if self.memory.is_some()
                && self.nudge_interval > 0
                && total_iterations > 0
                && total_iterations % self.nudge_interval == 0
            {
                system_prompt_text.push_str(MEMORY_NUDGE);
            }

            let transport_messages: Vec<peko_transport::provider::Message> = conversation
                .iter()
                .flat_map(|m| m.to_transport_messages())
                .collect();

            let mut tool_schemas = self.tools.schemas();

            // Inject the escalate tool ONLY when the brain's mode supports
            // escalation (Dual mode) AND we're currently on the local side.
            let using_local = matches!(&self.active_brain, Some(BrainChoice::Local));
            let escalation_available = self.brain.as_ref()
                .map(|b| b.mode().supports_escalation())
                .unwrap_or(false);
            if using_local && escalation_available {
                tool_schemas.push(escalate_tool_schema());
            }

            // Select provider: brain-routed or default
            let active_provider: &dyn LlmProvider = match (&self.brain, &self.active_brain) {
                (Some(brain), Some(choice)) => brain.provider(choice),
                _ => self.provider.as_ref(),
            };

            let brain_label = self.active_brain.as_ref().map(|b| b.to_string()).unwrap_or_else(|| "default".to_string());
            info!(iteration = total_iterations + 1, messages = transport_messages.len(), brain = %brain_label, model = active_provider.model_name(), "run_turn: calling LLM");

            let mut stream = match active_provider.stream_completion(
                &system_prompt_text,
                &transport_messages,
                &tool_schemas,
            ).await {
                Ok(s) => s,
                Err(e) => {
                    error!("run_turn: LLM call failed: {}", e);
                    let _ = tx.send(StreamCallback::Error(format!("LLM error: {}", e))).await;
                    break;
                }
            };

            let mut text_buffer = String::new();
            let mut tool_calls: Vec<ToolCall> = Vec::new();
            let mut current_tool_name = String::new();
            let mut current_tool_id = String::new();
            let mut current_tool_input = String::new();
            let mut stop_reason = StopReason::EndTurn;

            while let Some(event) = stream.next().await {
                let event = match event {
                    Ok(e) => e,
                    Err(e) => {
                        error!("run_turn: stream error: {}", e);
                        let _ = tx.send(StreamCallback::Error(format!("LLM stream error: {}", e))).await;
                        break;
                    }
                };
                match event {
                    StreamEvent::TextDelta(text) => {
                        text_buffer.push_str(&text);
                        let _ = tx.send(StreamCallback::TextDelta(text)).await;
                    }
                    StreamEvent::ToolUseStart { id, name } => {
                        current_tool_id = id;
                        current_tool_name = name.clone();
                        current_tool_input.clear();
                        let _ = tx.send(StreamCallback::ToolStart { name }).await;
                    }
                    StreamEvent::ToolInputDelta(json) => {
                        current_tool_input.push_str(&json);
                    }
                    StreamEvent::ContentBlockStop { .. } => {
                        if !current_tool_name.is_empty() {
                            let input: serde_json::Value = serde_json::from_str(&current_tool_input)
                                .unwrap_or(serde_json::Value::Object(serde_json::Map::new()));
                            tool_calls.push(ToolCall {
                                id: current_tool_id.clone(),
                                name: current_tool_name.clone(),
                                input,
                            });
                            current_tool_name.clear();
                            current_tool_input.clear();
                        }
                    }
                    StreamEvent::MessageDelta { stop_reason: sr, .. } => {
                        stop_reason = sr;
                        // Finalize any pending tool call (OpenAI format doesn't send ContentBlockStop)
                        if !current_tool_name.is_empty() {
                            let input: serde_json::Value = serde_json::from_str(&current_tool_input)
                                .unwrap_or(serde_json::Value::Object(serde_json::Map::new()));
                            tool_calls.push(ToolCall {
                                id: current_tool_id.clone(),
                                name: current_tool_name.clone(),
                                input,
                            });
                            current_tool_name.clear();
                            current_tool_input.clear();
                        }
                    }
                    StreamEvent::ThinkingDelta(thought) => {
                        let _ = tx.send(StreamCallback::Thinking(thought)).await;
                    }
                    _ => {}
                }
            }

            // Finalize any tool call that wasn't closed (safety net)
            if !current_tool_name.is_empty() {
                let input: serde_json::Value = serde_json::from_str(&current_tool_input)
                    .unwrap_or(serde_json::Value::Object(serde_json::Map::new()));
                tool_calls.push(ToolCall {
                    id: current_tool_id.clone(),
                    name: current_tool_name.clone(),
                    input,
                });
            }

            let text = if text_buffer.is_empty() { None } else { Some(text_buffer.clone()) };
            let assistant_msg = Message::assistant_tool_calls(text, tool_calls.clone());
            conversation.push(assistant_msg.clone());
            self.session.append_message(session_id, &assistant_msg)?;

            info!(
                text_len = text_buffer.len(),
                tool_count = tool_calls.len(),
                stop = ?stop_reason,
                "run_turn: LLM response received"
            );

            if tool_calls.is_empty() {
                final_text = text_buffer;
                total_iterations += 1;
                info!("run_turn: no tools, breaking with text");
                break;
            }

            if stop_reason == StopReason::EndTurn && !text_buffer.is_empty() {
                final_text = text_buffer;
                total_iterations += 1;
                info!("run_turn: end_turn with text, breaking");
                break;
            }

            for tc in &tool_calls {
                info!(tool = %tc.name, "run_turn: executing tool");
                let result = match self.tools.execute(&tc.name, tc.input.clone()).await {
                    Ok(r) => r,
                    Err(e) => ToolResult::error(format!("Error: {}", e)),
                };

                // Save image to a temp file and send URL instead of inline base64
                let img_for_ui = result.image.as_ref().and_then(|img| {
                    let ext = if img.media_type.contains("jpeg") { "jpg" } else { "png" };
                    let filename = format!("screenshot_{}.{}", chrono::Utc::now().timestamp_millis(), ext);
                    let dir = std::path::Path::new("/data/peko/screenshots");
                    let _ = std::fs::create_dir_all(dir);
                    let path = dir.join(&filename);

                    // Try data dir, fallback to /tmp
                    let (save_path, url_path) = if let Ok(decoded) = base64::Engine::decode(
                        &base64::engine::general_purpose::STANDARD, &img.base64
                    ) {
                        if std::fs::write(&path, &decoded).is_ok() {
                            (path, format!("/api/screenshots/{}", filename))
                        } else {
                            let tmp = std::path::Path::new("/tmp").join(&filename);
                            let _ = std::fs::write(&tmp, &decoded);
                            (tmp, format!("/api/screenshots/{}", filename))
                        }
                    } else {
                        return None;
                    };

                    Some((url_path, img.media_type.clone()))
                });

                let _ = tx.send(StreamCallback::ToolResult {
                    name: tc.name.clone(),
                    content: result.content.clone(),
                    is_error: result.is_error,
                    image: img_for_ui,
                }).await;

                let tool_msg = if let Some(img) = result.image {
                    Message::tool_result_with_image(
                        tc.id.clone(), tc.name.clone(), result.content, result.is_error, img,
                    )
                } else {
                    Message::tool_result(
                        tc.id.clone(), tc.name.clone(), result.content, result.is_error,
                    )
                };
                conversation.push(tool_msg.clone());
                self.session.append_message(session_id, &tool_msg)?;
            }

            total_iterations += 1;
            if let Err(_) = self.budget.decrement() { break; }
        }

        let status = if self.budget.is_interrupted() { "interrupted" } else { "completed" };
        self.session.update_status(session_id, status, total_iterations)?;

        if let Some(ref user_model) = self.user_model {
            if let Ok(mut model) = user_model.try_lock() {
                model.record_task(user_input, total_iterations);
                if let Some(ref path) = self.user_model_path {
                    let _ = model.save(path);
                }
            }
        }

        // Decide the per-task brain label shown in the UI.
        //   did_escalate → "escalated"  (started local, ended on cloud)
        //   else local   → "local"
        //   else cloud   → "cloud"
        //   no brain     → None
        let brain_label: Option<String> = if did_escalate {
            Some("escalated".to_string())
        } else {
            match (&start_brain, &self.active_brain) {
                (_, Some(choice)) => Some(choice.to_string()),
                _ => None,
            }
        };

        let sid = session_id.to_string();
        let _ = tx.send(StreamCallback::Done {
            iterations: total_iterations,
            session_id: sid.clone(),
            brain: brain_label,
        }).await;

        Ok(AgentResponse {
            text: final_text,
            iterations: total_iterations,
            session_id: sid,
        })
    }
}

/// Build an LLM provider from JSON config. Used by web API, Telegram, and scheduler.
pub fn build_provider_helper(config: &serde_json::Value) -> anyhow::Result<Box<dyn LlmProvider>> {
    use peko_transport::{AnthropicProvider, OpenAICompatProvider, ProviderChain};

    let priority = config["provider"]["priority"]
        .as_array()
        .map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect::<Vec<_>>())
        .unwrap_or_else(|| vec!["local".to_string()]);

    let mut providers: Vec<Box<dyn LlmProvider>> = Vec::new();

    for name in &priority {
        let entry = &config["provider"][name.as_str()];
        if entry.is_null() { continue; }

        let api_key = entry["api_key"].as_str().unwrap_or("").to_string();
        let model = entry["model"].as_str().unwrap_or("").to_string();
        let base_url = entry["base_url"].as_str().unwrap_or("").to_string();
        let max_tokens = entry["max_tokens"].as_u64().unwrap_or(4096) as usize;

        if model.is_empty() { continue; }

        match name.as_str() {
            "anthropic" => {
                let key = if api_key.is_empty() {
                    std::env::var("ANTHROPIC_API_KEY").unwrap_or_default()
                } else { api_key };
                if key.is_empty() { continue; }
                providers.push(Box::new(AnthropicProvider::new(
                    key, model, max_tokens,
                    if base_url.is_empty() { None } else { Some(base_url) },
                )));
            }
            "openrouter" => {
                let key = if api_key.is_empty() {
                    std::env::var("OPENROUTER_API_KEY").unwrap_or_default()
                } else { api_key };
                if key.is_empty() { continue; }
                let url = if base_url.is_empty() { "https://openrouter.ai/api/v1".to_string() } else { base_url };
                providers.push(Box::new(OpenAICompatProvider::new(key, model, url, max_tokens)));
            }
            _ => {
                let url = if base_url.is_empty() { "http://localhost:11434/v1".to_string() } else { base_url };
                providers.push(Box::new(OpenAICompatProvider::new(api_key, model, url, max_tokens)));
            }
        }
    }

    if providers.is_empty() {
        // No HTTP providers configured — return a null provider that errors if called.
        // Useful when the agent uses an embedded LLM (via DualBrain) and no cloud key is set.
        warn!("no HTTP LLM providers configured — using null fallback (embedded brain required)");
        return Ok(Box::new(NullProvider));
    }

    if providers.len() == 1 {
        Ok(providers.into_iter().next().unwrap())
    } else {
        Ok(Box::new(ProviderChain::new(providers)))
    }
}

/// Null provider that always errors. Used as a placeholder when the agent
/// relies entirely on an embedded/dual-brain setup and no cloud API is configured.
struct NullProvider;

#[async_trait::async_trait]
impl LlmProvider for NullProvider {
    async fn stream_completion(
        &self,
        _system_prompt: &str,
        _messages: &[peko_transport::provider::Message],
        _tools: &[serde_json::Value],
    ) -> anyhow::Result<futures::stream::BoxStream<'static, anyhow::Result<StreamEvent>>> {
        anyhow::bail!("no LLM provider available — embedded brain may have failed to load")
    }

    fn model_name(&self) -> &str {
        "null-provider"
    }

    fn max_context_tokens(&self) -> usize {
        0
    }
}

/// Build a brain router from config. Supports three modes:
///
/// | `provider.brain` value | Mode        | Behavior                          |
/// |------------------------|-------------|-----------------------------------|
/// | `"local:anthropic"`    | Dual        | classify + route + escalate tool  |
/// | `"local"`              | LocalOnly   | always use `provider.local`       |
/// | `"anthropic"`          | CloudOnly   | always use `provider.anthropic`   |
///
/// Returns None if `brain` is not set or the referenced provider can't be built.
pub fn build_dual_brain(config: &serde_json::Value) -> Option<DualBrain> {
    use crate::brain::BrainMode;

    let brain_str = config["provider"]["brain"].as_str()?;

    // Parse mode + provider names
    let parts: Vec<&str> = brain_str.split(':').collect();
    let (mode, local_name, cloud_name) = match parts.as_slice() {
        // "name"           → single-brain (mode decided by whether there's a cloud fallback,
        //                    but we just treat it as LocalOnly when the only name is a local-like
        //                    provider, else CloudOnly)
        [single] => {
            let single = single.trim();
            // Heuristic: providers named "local", "embedded", or whose base_url uses unix://
            // or points at localhost → LocalOnly. Anything else → CloudOnly.
            let entry = &config["provider"][single];
            let base_url = entry["base_url"].as_str().unwrap_or("");
            let looks_local = matches!(single, "local" | "embedded")
                || base_url.starts_with("unix://")
                || base_url.starts_with("http://localhost")
                || base_url.starts_with("http://127.");
            if looks_local {
                (BrainMode::LocalOnly, single, single)
            } else {
                (BrainMode::CloudOnly, single, single)
            }
        }
        // "local:cloud" → Dual
        [l, c] => (BrainMode::Dual, l.trim(), c.trim()),
        _ => {
            warn!(brain = brain_str, "invalid brain config, expected 'local:cloud' or 'name'");
            return None;
        }
    };

    fn build_single(config: &serde_json::Value, name: &str) -> Option<Box<dyn LlmProvider>> {
        use peko_transport::{AnthropicProvider, OpenAICompatProvider, UnixSocketProvider};

        let entry = &config["provider"][name];
        if entry.is_null() { return None; }

        let api_key = entry["api_key"].as_str().unwrap_or("").to_string();
        let model = entry["model"].as_str().unwrap_or("").to_string();
        let base_url = entry["base_url"].as_str().unwrap_or("").to_string();
        let max_tokens = entry["max_tokens"].as_u64().unwrap_or(4096) as usize;

        if model.is_empty() { return None; }

        // `unix://` scheme routes to the local peko-llm-daemon over a UDS.
        // Examples:
        //   unix:///data/local/tmp/peko/llm.sock    — filesystem
        //   unix://@peko-llm                        — Linux abstract namespace
        if let Some(rest) = base_url.strip_prefix("unix://") {
            let path = rest.to_string();
            info!(name = %name, socket = %path, "building UnixSocketProvider for local daemon");
            return Some(Box::new(UnixSocketProvider::new(path, model, max_tokens)));
        }

        match name {
            "anthropic" => {
                let key = if api_key.is_empty() {
                    std::env::var("ANTHROPIC_API_KEY").unwrap_or_default()
                } else { api_key };
                if key.is_empty() { return None; }
                Some(Box::new(AnthropicProvider::new(
                    key, model, max_tokens,
                    if base_url.is_empty() { None } else { Some(base_url) },
                )))
            }
            "openrouter" => {
                let key = if api_key.is_empty() {
                    std::env::var("OPENROUTER_API_KEY").unwrap_or_default()
                } else { api_key };
                if key.is_empty() { return None; }
                let url = if base_url.is_empty() { "https://openrouter.ai/api/v1".to_string() } else { base_url };
                Some(Box::new(OpenAICompatProvider::new(key, model, url, max_tokens)))
            }
            _ => {
                let url = if base_url.is_empty() { "http://localhost:11434/v1".to_string() } else { base_url };
                Some(Box::new(OpenAICompatProvider::new(api_key, model, url, max_tokens)))
            }
        }
    }

    let local = match build_single(config, local_name) {
        Some(p) => p,
        None => {
            warn!(name = local_name, "failed to build local brain provider");
            return None;
        }
    };
    let cloud = match build_single(config, cloud_name) {
        Some(p) => p,
        None => {
            warn!(
                name = cloud_name,
                "cloud brain unavailable — using local for both brains (escalation disabled)"
            );
            // Rebuild local so both sides have their own provider instance
            match build_single(config, local_name) {
                Some(fallback) => fallback,
                None => {
                    warn!("local fallback also failed, aborting dual-brain");
                    return None;
                }
            }
        }
    };

    info!(
        mode = %mode,
        local = local.model_name(),
        cloud = cloud.model_name(),
        "brain initialized"
    );

    Some(DualBrain::with_mode(mode, local, cloud))
}
