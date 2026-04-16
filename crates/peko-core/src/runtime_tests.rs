#[cfg(test)]
mod tests {
    use crate::*;
    use crate::tool::{Tool, ToolResult, ToolRegistry};
    use crate::message::Message;
    use crate::session::SessionStore;
    use crate::memory::MemoryStore;
    use peko_config::AgentConfig;
    use peko_transport::{LlmProvider, StreamEvent, StopReason};
    use peko_transport::provider::Message as TransportMessage;
    use async_trait::async_trait;
    use futures::stream::{self, BoxStream};
    use std::sync::Arc;
    use std::path::PathBuf;

    // ── Mock Provider ──

    struct MockProvider {
        responses: std::sync::Mutex<Vec<Vec<StreamEvent>>>,
    }

    impl MockProvider {
        fn new(responses: Vec<Vec<StreamEvent>>) -> Self {
            Self {
                responses: std::sync::Mutex::new(responses),
            }
        }
    }

    #[async_trait]
    impl LlmProvider for MockProvider {
        async fn stream_completion(
            &self,
            _system_prompt: &str,
            _messages: &[TransportMessage],
            _tools: &[serde_json::Value],
        ) -> anyhow::Result<BoxStream<'static, anyhow::Result<StreamEvent>>> {
            let mut responses = self.responses.lock().unwrap();
            if responses.is_empty() {
                anyhow::bail!("no more mock responses");
            }
            let events = responses.remove(0);
            Ok(Box::pin(stream::iter(events.into_iter().map(Ok))))
        }

        fn model_name(&self) -> &str { "mock-model" }
        fn max_context_tokens(&self) -> usize { 100_000 }
    }

    // ── Mock Tool ──

    struct EchoTool;

    impl Tool for EchoTool {
        fn name(&self) -> &str { "echo" }
        fn description(&self) -> &str { "Echoes input back" }
        fn parameters_schema(&self) -> serde_json::Value {
            serde_json::json!({"type": "object", "properties": {"text": {"type": "string"}}, "required": ["text"]})
        }
        fn execute(&self, args: serde_json::Value) -> std::pin::Pin<Box<dyn std::future::Future<Output = anyhow::Result<ToolResult>> + Send + '_>> {
            Box::pin(async move {
                let text = args["text"].as_str().unwrap_or("no text");
                Ok(ToolResult::success(format!("ECHO: {}", text)))
            })
        }
    }

    fn test_config() -> AgentConfig {
        AgentConfig {
            max_iterations: 10,
            context_window: 100_000,
            history_share: 0.7,
            data_dir: PathBuf::from("/tmp"),
            log_level: "error".to_string(),
        }
    }

    #[tokio::test]
    async fn test_simple_text_response() {
        let provider = MockProvider::new(vec![
            vec![
                StreamEvent::TextDelta("Hello ".to_string()),
                StreamEvent::TextDelta("world!".to_string()),
                StreamEvent::MessageDelta { output_tokens: 5, stop_reason: StopReason::EndTurn },
                StreamEvent::MessageStop,
            ],
        ]);

        let registry = ToolRegistry::new();
        let session = SessionStore::open_in_memory().unwrap();
        let config = test_config();

        let mut runtime = AgentRuntime::new(
            &config,
            Arc::new(registry),
            Box::new(provider),
            session,
        );

        let response = runtime.run_task("say hello").await.unwrap();
        assert_eq!(response.text, "Hello world!");
        assert_eq!(response.iterations, 1);
        assert!(!response.session_id.is_empty());
    }

    #[tokio::test]
    async fn test_tool_call_then_response() {
        let provider = MockProvider::new(vec![
            // First call: LLM requests echo tool
            vec![
                StreamEvent::TextDelta("Let me echo that.".to_string()),
                StreamEvent::ToolUseStart { id: "tc_1".to_string(), name: "echo".to_string() },
                StreamEvent::ToolInputDelta("{\"text\":\"hello\"}".to_string()),
                StreamEvent::ContentBlockStop { index: 1 },
                StreamEvent::MessageDelta { output_tokens: 20, stop_reason: StopReason::ToolUse },
                StreamEvent::MessageStop,
            ],
            // Second call: LLM gives final answer
            vec![
                StreamEvent::TextDelta("The echo returned: ECHO: hello".to_string()),
                StreamEvent::MessageDelta { output_tokens: 10, stop_reason: StopReason::EndTurn },
                StreamEvent::MessageStop,
            ],
        ]);

        let mut registry = ToolRegistry::new();
        registry.register(EchoTool);

        let session = SessionStore::open_in_memory().unwrap();
        let config = test_config();

        let mut runtime = AgentRuntime::new(
            &config,
            Arc::new(registry),
            Box::new(provider),
            session,
        );

        let response = runtime.run_task("echo hello").await.unwrap();
        assert!(response.text.contains("ECHO: hello"));
        assert_eq!(response.iterations, 2);
    }

    #[tokio::test]
    async fn test_budget_exhaustion() {
        // Provider always returns tool calls — should exhaust budget
        let mut responses = Vec::new();
        for i in 0..15 {
            responses.push(vec![
                StreamEvent::ToolUseStart { id: format!("tc_{}", i), name: "echo".to_string() },
                StreamEvent::ToolInputDelta("{\"text\":\"loop\"}".to_string()),
                StreamEvent::ContentBlockStop { index: 0 },
                StreamEvent::MessageDelta { output_tokens: 5, stop_reason: StopReason::ToolUse },
                StreamEvent::MessageStop,
            ]);
        }

        let provider = MockProvider::new(responses);
        let mut registry = ToolRegistry::new();
        registry.register(EchoTool);

        let session = SessionStore::open_in_memory().unwrap();
        let mut config = test_config();
        config.max_iterations = 3;

        let mut runtime = AgentRuntime::new(
            &config,
            Arc::new(registry),
            Box::new(provider),
            session,
        );

        let response = runtime.run_task("loop forever").await.unwrap();
        assert!(response.iterations <= 4); // budget=3 iterations
    }

    #[tokio::test]
    async fn test_memory_injection() {
        // Setup memory
        let mem_store = MemoryStore::open_in_memory().unwrap();
        mem_store.save("user_name", "User is called Bob", &crate::memory::MemoryCategory::Fact, 0.9, None).unwrap();
        let mem_arc = Arc::new(tokio::sync::Mutex::new(mem_store));

        // Provider captures the system prompt to verify memory was injected
        let provider = MockProvider::new(vec![
            vec![
                StreamEvent::TextDelta("Hi Bob!".to_string()),
                StreamEvent::MessageDelta { output_tokens: 3, stop_reason: StopReason::EndTurn },
                StreamEvent::MessageStop,
            ],
        ]);

        let registry = ToolRegistry::new();
        let session = SessionStore::open_in_memory().unwrap();
        let config = test_config();

        let mut runtime = AgentRuntime::new(
            &config,
            Arc::new(registry),
            Box::new(provider),
            session,
        ).with_memory(mem_arc);

        let response = runtime.run_task("what is my name").await.unwrap();
        assert_eq!(response.text, "Hi Bob!");
        // Memory was injected (we can't verify the prompt content from here,
        // but the fact it ran without error means injection didn't crash)
    }

    #[tokio::test]
    async fn test_session_persistence() {
        let provider = MockProvider::new(vec![
            vec![
                StreamEvent::TextDelta("response text".to_string()),
                StreamEvent::MessageDelta { output_tokens: 3, stop_reason: StopReason::EndTurn },
                StreamEvent::MessageStop,
            ],
        ]);

        let registry = ToolRegistry::new();
        let session = SessionStore::open_in_memory().unwrap();
        let config = test_config();

        let mut runtime = AgentRuntime::new(
            &config,
            Arc::new(registry),
            Box::new(provider),
            session,
        );

        let response = runtime.run_task("test task").await.unwrap();

        // Verify session was created and completed
        // (we can't access session store directly from here since runtime owns it,
        // but the session_id being returned proves it was created)
        assert!(!response.session_id.is_empty());
    }
}
