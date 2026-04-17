use std::path::PathBuf;
use tracing::info;

/// Configuration for the embedded LLM engine
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct LlmEngineConfig {
    /// Path to the GGUF model file
    pub model_path: PathBuf,
    /// Path to the tokenizer.json (HF format)
    pub tokenizer_path: Option<PathBuf>,
    /// HuggingFace model ID for auto-downloading tokenizer
    pub hf_model_id: Option<String>,
    /// Context window size in tokens
    pub context_size: u32,
    /// Temperature for sampling
    pub temperature: f32,
    /// Top-p sampling
    pub top_p: f32,
    /// Repetition penalty
    pub repeat_penalty: f32,
    /// Max tokens to generate per response
    pub max_tokens: u32,
    /// Model name for display / logging
    pub model_name: String,
    /// Number of CPU threads
    pub threads: u32,
}

impl Default for LlmEngineConfig {
    fn default() -> Self {
        Self {
            model_path: PathBuf::from("/data/peko/models/model.gguf"),
            tokenizer_path: None,
            hf_model_id: None,
            context_size: 2048,
            temperature: 0.7,
            top_p: 0.9,
            repeat_penalty: 1.1,
            max_tokens: 1024,
            model_name: "embedded".to_string(),
            threads: 4,
        }
    }
}

/// Trait for the inference backend.
/// Each backend (candle, llama.cpp, LiteRT) implements this.
pub trait InferenceBackend: Send + Sync {
    /// Generate tokens one at a time. Calls `on_token` for each generated token.
    /// Return false from on_token to stop generation.
    fn generate(
        &self,
        prompt: &str,
        max_tokens: u32,
        stop_sequences: &[&str],
        on_token: &mut dyn FnMut(&str) -> bool,
    ) -> anyhow::Result<String>;

    /// Count tokens in text
    fn token_count(&self, text: &str) -> usize;

    /// Model name
    fn model_name(&self) -> &str;

    /// Max context tokens
    fn max_context(&self) -> usize;
}

/// The LLM engine — wraps a backend and manages model lifecycle
pub struct LlmEngine {
    backend: Box<dyn InferenceBackend>,
    config: LlmEngineConfig,
}

impl LlmEngine {
    /// Create engine with a specific backend
    pub fn with_backend(backend: Box<dyn InferenceBackend>, config: LlmEngineConfig) -> Self {
        info!(
            model = %config.model_name,
            ctx = config.context_size,
            path = %config.model_path.display(),
            "embedded LLM engine ready"
        );
        Self { backend, config }
    }

    pub fn backend(&self) -> &dyn InferenceBackend {
        self.backend.as_ref()
    }

    pub fn config(&self) -> &LlmEngineConfig {
        &self.config
    }
}
