use crate::provider::{LlmProvider, Message};
use crate::StreamEvent;
use async_trait::async_trait;
use futures::stream::BoxStream;
use tracing::warn;

pub struct ProviderChain {
    providers: Vec<Box<dyn LlmProvider>>,
}

impl ProviderChain {
    pub fn new(providers: Vec<Box<dyn LlmProvider>>) -> Self {
        assert!(!providers.is_empty(), "ProviderChain requires at least one provider");
        Self { providers }
    }

    pub fn single(provider: Box<dyn LlmProvider>) -> Self {
        Self { providers: vec![provider] }
    }
}

#[async_trait]
impl LlmProvider for ProviderChain {
    async fn stream_completion(
        &self,
        system_prompt: &str,
        messages: &[Message],
        tools: &[serde_json::Value],
    ) -> anyhow::Result<BoxStream<'static, anyhow::Result<StreamEvent>>> {
        let mut last_error = None;

        for (i, provider) in self.providers.iter().enumerate() {
            match provider.stream_completion(system_prompt, messages, tools).await {
                Ok(stream) => return Ok(stream),
                Err(e) => {
                    let err_str = e.to_string();
                    let is_retryable = err_str.contains("429")
                        || err_str.contains("500")
                        || err_str.contains("502")
                        || err_str.contains("503")
                        || err_str.contains("connection")
                        || err_str.contains("timeout");

                    if is_retryable && i + 1 < self.providers.len() {
                        warn!(
                            provider = provider.model_name(),
                            error = %e,
                            "provider failed, trying next"
                        );
                        last_error = Some(e);
                        continue;
                    } else {
                        return Err(e);
                    }
                }
            }
        }

        Err(last_error.unwrap_or_else(|| anyhow::anyhow!("no providers available")))
    }

    fn model_name(&self) -> &str {
        self.providers.first().map(|p| p.model_name()).unwrap_or("unknown")
    }

    fn max_context_tokens(&self) -> usize {
        self.providers.first().map(|p| p.max_context_tokens()).unwrap_or(128_000)
    }
}
