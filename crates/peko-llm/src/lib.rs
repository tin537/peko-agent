//! peko-llm: Embedded LLM inference engine for peko-agent.
//!
//! Runs a GGUF model directly in-process — no HTTP server,
//! no Ollama, no serialization overhead. The LLM is part of the kernel.
//!
//! Uses candle (pure Rust) for inference. Cross-compiles to Android ARM64.

mod engine;
mod provider;

#[cfg(feature = "candle")]
mod candle_backend;

pub use engine::{LlmEngine, LlmEngineConfig, InferenceBackend};
pub use provider::EmbeddedProvider;

/// Load an embedded LLM from a GGUF file using the candle backend.
/// This is the main entry point for creating an in-process local brain.
#[cfg(feature = "candle")]
pub fn load_gguf(config: LlmEngineConfig) -> anyhow::Result<LlmEngine> {
    let backend = candle_backend::CandleBackend::load(&config)?;
    Ok(LlmEngine::with_backend(Box::new(backend), config))
}
