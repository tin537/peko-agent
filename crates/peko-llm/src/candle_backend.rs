//! Candle backend — pure Rust GGUF inference, no C dependencies.
//! Cross-compiles cleanly to Android ARM64.

use std::path::Path;
use std::sync::Mutex;
use anyhow::Context;
use tracing::info;

use candle_core::{Device, Tensor};
use candle_transformers::generation::LogitsProcessor;
use candle_transformers::models::{quantized_llama, quantized_qwen3, quantized_qwen2, quantized_gemma3};

use crate::engine::{InferenceBackend, LlmEngineConfig};

/// Quantized model variants. Auto-detected from GGUF metadata architecture field.
pub enum QuantizedModel {
    Llama(quantized_llama::ModelWeights),
    Qwen3(quantized_qwen3::ModelWeights),
    Qwen2(quantized_qwen2::ModelWeights),
    Gemma3(quantized_gemma3::ModelWeights),
}

impl QuantizedModel {
    fn forward(&mut self, input: &Tensor, pos: usize) -> candle_core::Result<Tensor> {
        match self {
            Self::Llama(m) => m.forward(input, pos),
            Self::Qwen3(m) => m.forward(input, pos),
            Self::Qwen2(m) => m.forward(input, pos),
            Self::Gemma3(m) => m.forward(input, pos),
        }
    }

    fn arch_name(&self) -> &'static str {
        match self {
            Self::Llama(_) => "llama",
            Self::Qwen3(_) => "qwen3",
            Self::Qwen2(_) => "qwen2",
            Self::Gemma3(_) => "gemma3",
        }
    }
}

/// GGUF model loaded via candle quantized inference
pub struct CandleBackend {
    model: Mutex<QuantizedModel>,
    tokenizer: tokenizers::Tokenizer,
    config: LlmEngineConfig,
    device: Device,
}

// Safety: candle model is behind a Mutex
unsafe impl Send for CandleBackend {}
unsafe impl Sync for CandleBackend {}

impl CandleBackend {
    /// Load a GGUF model file from disk, auto-detecting the architecture
    pub fn load(config: &LlmEngineConfig) -> anyhow::Result<Self> {
        info!(path = %config.model_path.display(), "loading GGUF model via candle");

        let device = Device::Cpu;

        // Read GGUF metadata to detect architecture
        let mut file = std::fs::File::open(&config.model_path)
            .context("failed to open GGUF model file")?;
        let gguf = candle_core::quantized::gguf_file::Content::read(&mut file)
            .context("failed to parse GGUF file")?;

        // Read "general.architecture" metadata field to decide which model family to load
        let arch = gguf.metadata.get("general.architecture")
            .and_then(|v| v.to_string().ok())
            .map(|s| s.to_lowercase())
            .unwrap_or_else(|| "llama".to_string());

        info!(arch = %arch, "detected GGUF architecture");

        let model = match arch.as_str() {
            "qwen3" => {
                let m = quantized_qwen3::ModelWeights::from_gguf(gguf, &mut file, &device)
                    .context("failed to load qwen3 weights")?;
                QuantizedModel::Qwen3(m)
            }
            "qwen2" | "qwen2.5" => {
                let m = quantized_qwen2::ModelWeights::from_gguf(gguf, &mut file, &device)
                    .context("failed to load qwen2 weights")?;
                QuantizedModel::Qwen2(m)
            }
            "gemma3" | "gemma2" => {
                let m = quantized_gemma3::ModelWeights::from_gguf(gguf, &mut file, &device)
                    .context("failed to load gemma3 weights")?;
                QuantizedModel::Gemma3(m)
            }
            _ => {
                // Fall back to llama-compatible (covers llama, mistral, many tunes)
                let m = quantized_llama::ModelWeights::from_gguf(gguf, &mut file, &device)
                    .context("failed to load llama-compat weights")?;
                QuantizedModel::Llama(m)
            }
        };

        info!(arch = model.arch_name(), "GGUF model loaded");

        // Load tokenizer
        let tokenizer = Self::load_tokenizer(config)?;

        info!(
            model = %config.model_name,
            vocab = tokenizer.get_vocab_size(true),
            "candle backend ready"
        );

        Ok(Self {
            model: Mutex::new(model),
            tokenizer,
            config: config.clone(),
            device,
        })
    }

    fn load_tokenizer(config: &LlmEngineConfig) -> anyhow::Result<tokenizers::Tokenizer> {
        // Try explicit path first
        if let Some(ref path) = config.tokenizer_path {
            if path.exists() {
                info!(path = %path.display(), "loading tokenizer from file");
                return tokenizers::Tokenizer::from_file(path)
                    .map_err(|e| anyhow::anyhow!("tokenizer load failed: {}", e));
            }
        }

        // Try tokenizer.json next to the model file
        let model_dir = config.model_path.parent().unwrap_or(Path::new("."));
        let adjacent = model_dir.join("tokenizer.json");
        if adjacent.exists() {
            info!(path = %adjacent.display(), "loading tokenizer from model directory");
            return tokenizers::Tokenizer::from_file(&adjacent)
                .map_err(|e| anyhow::anyhow!("tokenizer load failed: {}", e));
        }

        // Try HuggingFace hub download (only on platforms with network)
        #[cfg(feature = "hf-hub")]
        if let Some(ref model_id) = config.hf_model_id {
            info!(model_id = %model_id, "downloading tokenizer from HuggingFace");
            let api = hf_hub::api::sync::Api::new()?;
            let repo = api.model(model_id.to_string());
            let tokenizer_path = repo.get("tokenizer.json")?;
            return tokenizers::Tokenizer::from_file(&tokenizer_path)
                .map_err(|e| anyhow::anyhow!("tokenizer load failed: {}", e));
        }

        anyhow::bail!(
            "no tokenizer found. Place tokenizer.json next to {} or set tokenizer_path/hf_model_id in config",
            config.model_path.display()
        )
    }
}

impl InferenceBackend for CandleBackend {
    fn generate(
        &self,
        prompt: &str,
        max_tokens: u32,
        stop_sequences: &[&str],
        on_token: &mut dyn FnMut(&str) -> bool,
    ) -> anyhow::Result<String> {
        let t_total_start = std::time::Instant::now();

        let encoding = self.tokenizer.encode(prompt, true)
            .map_err(|e| anyhow::anyhow!("tokenize failed: {}", e))?;
        let prompt_tokens = encoding.get_ids().to_vec();

        info!(
            prompt_tokens = prompt_tokens.len(),
            max_tokens,
            arch = self.model.lock().map(|m| m.arch_name()).unwrap_or("locked"),
            "candle.generate: starting"
        );

        let mut model = self.model.lock()
            .map_err(|e| anyhow::anyhow!("model lock poisoned: {}", e))?;

        let mut logits_processor = LogitsProcessor::new(
            rand_seed(),
            Some(self.config.temperature as f64),
            Some(self.config.top_p as f64),
        );

        let mut output_tokens: Vec<u32> = Vec::new();
        let mut output_text = String::new();
        let mut all_tokens = prompt_tokens.clone();

        // Process prompt (prefill)
        let t_prefill = std::time::Instant::now();
        let input = Tensor::new(prompt_tokens.as_slice(), &self.device)?
            .unsqueeze(0)?;
        let logits = model.forward(&input, 0)?;
        let prefill_ms = t_prefill.elapsed().as_millis();
        info!(prefill_ms, "candle.generate: prefill done");

        let logits = logits.squeeze(0)?;
        // Last token logits — shape depends on whether model returns per-token or last-only logits
        let logits = if logits.dims().len() > 1 {
            logits.get(logits.dim(0)? - 1)?
        } else {
            logits
        };

        let next_token = logits_processor.sample(&logits)?;
        output_tokens.push(next_token);
        all_tokens.push(next_token);

        // Decode and stream first token
        if let Some(text) = self.decode_token(&output_tokens) {
            info!(token_id = next_token, first_piece = %text, "candle.generate: first token");
            output_text.push_str(&text);
            if !on_token(&text) {
                return Ok(output_text);
            }
        }

        // Generate remaining tokens (decode phase)
        let t_decode_start = std::time::Instant::now();
        for i in 0..max_tokens.saturating_sub(1) {
            let input = Tensor::new(&[*all_tokens.last().unwrap()], &self.device)?
                .unsqueeze(0)?;
            let pos = prompt_tokens.len() + i as usize;
            let logits = model.forward(&input, pos)?;
            let logits = logits.squeeze(0)?;
            let logits = if logits.dims().len() > 1 {
                logits.get(logits.dim(0)? - 1)?
            } else {
                logits
            };

            // Apply repeat penalty
            let logits = if self.config.repeat_penalty != 1.0 {
                candle_transformers::utils::apply_repeat_penalty(
                    &logits,
                    self.config.repeat_penalty,
                    all_tokens.as_slice(),
                )?
            } else {
                logits
            };

            let next_token = logits_processor.sample(&logits)?;

            // Check EOS tokens
            if is_eos_token(next_token, &self.tokenizer) {
                break;
            }

            output_tokens.push(next_token);
            all_tokens.push(next_token);

            // Decode incrementally
            if let Some(piece) = self.decode_token(&output_tokens) {
                output_text.push_str(&piece);

                // Check stop sequences
                if stop_sequences.iter().any(|s| output_text.contains(s)) {
                    // Trim the stop sequence from output
                    for s in stop_sequences {
                        if let Some(pos) = output_text.find(s) {
                            output_text.truncate(pos);
                            break;
                        }
                    }
                    break;
                }

                if !on_token(&piece) {
                    break;
                }
            }
        }

        let total_ms = t_total_start.elapsed().as_millis();
        let decode_ms = t_decode_start.elapsed().as_millis();
        let decoded = output_tokens.len();
        let tok_per_sec = if decode_ms > 0 { (decoded as f64 * 1000.0) / decode_ms as f64 } else { 0.0 };
        info!(
            total_ms,
            decode_ms,
            decoded_tokens = decoded,
            tok_per_sec = format!("{:.2}", tok_per_sec),
            "candle.generate: done"
        );

        Ok(output_text)
    }

    fn token_count(&self, text: &str) -> usize {
        self.tokenizer.encode(text, false)
            .map(|e| e.get_ids().len())
            .unwrap_or(text.len() / 4)
    }

    fn model_name(&self) -> &str {
        &self.config.model_name
    }

    fn max_context(&self) -> usize {
        self.config.context_size as usize
    }
}

impl CandleBackend {
    /// Decode the last token from the accumulated output_tokens.
    /// Returns the new text piece (handles multi-byte token boundaries).
    fn decode_token(&self, tokens: &[u32]) -> Option<String> {
        let full = self.tokenizer.decode(tokens, true).ok()?;

        if tokens.len() <= 1 {
            return Some(full);
        }

        // Decode without last token to find the new piece
        let prev = self.tokenizer.decode(&tokens[..tokens.len() - 1], true).ok()?;
        let piece = full.strip_prefix(&prev).unwrap_or(&full);

        if piece.is_empty() {
            None
        } else {
            Some(piece.to_string())
        }
    }
}

fn is_eos_token(token_id: u32, tokenizer: &tokenizers::Tokenizer) -> bool {
    // Common EOS token IDs
    if token_id == 0 || token_id == 1 || token_id == 2 {
        // Check if this is actually an EOS by looking at the text
        if let Some(text) = tokenizer.id_to_token(token_id) {
            return text.contains("eos") || text.contains("<|end") || text == "</s>" || text == "<s>";
        }
        return true;
    }

    // Check by text content
    if let Some(text) = tokenizer.id_to_token(token_id) {
        return text == "</s>" || text == "<|endoftext|>" || text == "<|end|>"
            || text == "<|im_end|>" || text == "<end_of_turn>"
            || text == "<|eot_id|>";
    }

    false
}

fn rand_seed() -> u64 {
    use std::time::SystemTime;
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(42)
}
