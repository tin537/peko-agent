//! Smoke test: load a GGUF model, run a simple prompt, print output.
//! Usage: cargo run -p peko-llm --example smoke_test -- /path/to/model.gguf

use std::io::Write;
use std::path::PathBuf;
use peko_llm::{LlmEngineConfig, load_gguf};

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::new("info,peko_llm=debug"))
        .init();

    let args: Vec<String> = std::env::args().collect();
    let model_path = args.get(1).cloned().unwrap_or_else(|| {
        "models/qwen3-1.7b-q4_k_m.gguf".to_string()
    });

    let config = LlmEngineConfig {
        model_path: PathBuf::from(&model_path),
        tokenizer_path: Some(PathBuf::from(
            std::path::Path::new(&model_path).parent().unwrap().join("tokenizer.json")
        )),
        hf_model_id: None,
        context_size: 2048,
        temperature: 0.7,
        top_p: 0.9,
        repeat_penalty: 1.1,
        max_tokens: 128,
        model_name: "Qwen3-1.7B".to_string(),
        threads: 4,
    };

    println!("[smoke] Loading model from {}...", model_path);
    let load_start = std::time::Instant::now();
    let engine = load_gguf(config)?;
    println!("[smoke] Model loaded in {:?}", load_start.elapsed());

    let prompt = "<|im_start|>system\nYou are Peko, a helpful Android agent. Respond briefly.<|im_end|>\n<|im_start|>user\nOpen YouTube and play a song.<|im_end|>\n<|im_start|>assistant\n";

    println!("\n[smoke] Prompt:\n{}\n", prompt);
    println!("[smoke] Generating (streaming):");
    print!("  > ");
    std::io::stdout().flush()?;

    let gen_start = std::time::Instant::now();
    let mut token_count = 0;
    let output = engine.backend().generate(
        prompt,
        128,
        &["<|im_end|>", "<|endoftext|>"],
        &mut |tok| {
            print!("{}", tok);
            let _ = std::io::stdout().flush();
            token_count += 1;
            true
        },
    )?;
    let elapsed = gen_start.elapsed();

    println!("\n\n[smoke] === stats ===");
    println!("  tokens generated: {}", token_count);
    println!("  elapsed: {:?}", elapsed);
    println!("  tok/s: {:.2}", token_count as f64 / elapsed.as_secs_f64());
    println!("  output length: {} chars", output.len());

    Ok(())
}
