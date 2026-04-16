use std::sync::Arc;
use futures::StreamExt;
use tracing::{info, warn, error};

use peko_config::AgentConfig;
use peko_transport::{LlmProvider, StreamEvent, StopReason};

use crate::budget::IterationBudget;
use crate::compressor::ContextCompressor;
use crate::memory::MemoryStore;
use crate::skills::SkillStore;
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
    session: SessionStore,
    provider: Box<dyn LlmProvider>,
    system_prompt: SystemPrompt,
    memory: Option<Arc<tokio::sync::Mutex<MemoryStore>>>,
    skills: Option<Arc<tokio::sync::Mutex<SkillStore>>>,
    nudge_interval: usize,
}

#[derive(Debug)]
pub struct AgentResponse {
    pub text: String,
    pub iterations: usize,
    pub session_id: String,
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
            nudge_interval: 5,
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

    pub fn with_nudge_interval(mut self, interval: usize) -> Self {
        self.nudge_interval = interval;
        self
    }

    pub fn budget_handle(&self) -> IterationBudget {
        self.budget.clone()
    }

    pub async fn run_task(&mut self, user_input: &str) -> anyhow::Result<AgentResponse> {
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
                    if let Ok(mem_store) = memory.try_lock() {
                        match mem_store.build_context(user_input, 5) {
                            Ok(ctx) if !ctx.is_empty() => {
                                system_prompt_text.push_str("\n\n");
                                system_prompt_text.push_str(&ctx);
                                info!(memories = ctx.lines().count() - 2, "injected memories into prompt");
                            }
                            _ => {}
                        }
                    }
                }
            }

            // Inject relevant skills on first iteration
            if total_iterations == 0 {
                if let Some(ref skills) = self.skills {
                    if let Ok(skill_store) = skills.try_lock() {
                        let ctx = skill_store.build_context(user_input);
                        if !ctx.is_empty() {
                            system_prompt_text.push_str("\n\n");
                            system_prompt_text.push_str(&ctx);
                            info!("injected skills into prompt");
                        }
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

            let tool_schemas = self.tools.schemas();

            // Stream completion
            info!(iteration = total_iterations + 1, "calling LLM");
            let mut stream = self.provider.stream_completion(
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
                    }
                    StreamEvent::ThinkingDelta(thought) => {
                        tracing::debug!(thought = %thought, "LLM thinking");
                    }
                    _ => {}
                }
            }

            // Build assistant message
            let text = if text_buffer.is_empty() { None } else { Some(text_buffer.clone()) };
            let assistant_msg = Message::assistant_tool_calls(text, tool_calls.clone());
            conversation.push(assistant_msg.clone());
            self.session.append_message(&session_id, &assistant_msg)?;

            if tool_calls.is_empty() || stop_reason == StopReason::EndTurn {
                // No tool calls → final response
                final_text = text_buffer;
                total_iterations += 1;
                break;
            }

            // Execute each tool call
            for tc in &tool_calls {
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

            total_iterations += 1;

            if let Err(_) = self.budget.decrement() {
                warn!("iteration budget exhausted after {} iterations", total_iterations);
                break;
            }
        }

        let status = if self.budget.is_interrupted() { "interrupted" } else { "completed" };
        self.session.update_status(&session_id, status, total_iterations)?;

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
        anyhow::bail!("no LLM providers configured");
    }

    if providers.len() == 1 {
        Ok(providers.into_iter().next().unwrap())
    } else {
        Ok(Box::new(ProviderChain::new(providers)))
    }
}
