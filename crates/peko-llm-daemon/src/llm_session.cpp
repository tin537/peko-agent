#include "llm_session.h"

#include <chrono>
#include <iostream>
#include <cstring>
#include <vector>
#include <memory>

#include "llama.h"

// Real inference via llama.cpp. Supports:
//   - CPU backend (always)
//   - Vulkan backend (when compiled with -DGGML_VULKAN=ON / PEKO_VULKAN=1)
//     for Adreno/Mali/Intel/NVIDIA GPUs
//
// Phase 1's stub API is preserved — only the implementation changes.

namespace peko {

// ── RAII wrappers so we can use unique_ptr ─────────────────────────

struct LlamaModelDeleter {
    void operator()(llama_model* m) const { if (m) llama_model_free(m); }
};
struct LlamaContextDeleter {
    void operator()(llama_context* c) const { if (c) llama_free(c); }
};
struct LlamaSamplerDeleter {
    void operator()(llama_sampler* s) const { if (s) llama_sampler_free(s); }
};

using ModelPtr   = std::unique_ptr<llama_model,   LlamaModelDeleter>;
using ContextPtr = std::unique_ptr<llama_context, LlamaContextDeleter>;
using SamplerPtr = std::unique_ptr<llama_sampler, LlamaSamplerDeleter>;

// ── PIMPL ───────────────────────────────────────────────────────────

struct LlmSession::Impl {
    ModelPtr   model;
    ContextPtr ctx;
    SamplerPtr sampler;
    const llama_vocab* vocab = nullptr;

    static bool backends_initialized_;
};

bool LlmSession::Impl::backends_initialized_ = false;

LlmSession::LlmSession(LlmConfig config)
    : config_(std::move(config)),
      impl_(std::make_unique<Impl>()) {}

LlmSession::~LlmSession() = default;

// ── Load ────────────────────────────────────────────────────────────

bool LlmSession::load() {
    // One-time backend registration: picks up ggml-cpu + ggml-vulkan (if compiled)
    if (!Impl::backends_initialized_) {
        ggml_backend_load_all();
        Impl::backends_initialized_ = true;
        std::cerr << "[llm] backends loaded" << std::endl;
    }

    // ── Model ──
    llama_model_params mparams = llama_model_default_params();
    mparams.n_gpu_layers = 99;          // offload everything to GPU if available
    mparams.use_mmap     = true;
    mparams.use_mlock    = false;

    std::cerr << "[llm] loading model: " << config_.model_path << std::endl;
    auto t0 = std::chrono::steady_clock::now();

    llama_model* raw_model = llama_model_load_from_file(
        config_.model_path.c_str(), mparams);
    if (!raw_model) {
        std::cerr << "[llm] failed to load model from " << config_.model_path << std::endl;
        return false;
    }
    impl_->model.reset(raw_model);
    impl_->vocab = llama_model_get_vocab(raw_model);

    auto load_ms = std::chrono::duration_cast<std::chrono::milliseconds>(
        std::chrono::steady_clock::now() - t0).count();
    std::cerr << "[llm] model loaded in " << load_ms << "ms" << std::endl;

    // ── Context ──
    llama_context_params cparams = llama_context_default_params();
    cparams.n_ctx        = static_cast<uint32_t>(config_.context_size);
    cparams.n_batch      = 512;
    cparams.n_ubatch     = 512;
    cparams.n_threads    = 4;
    cparams.n_threads_batch = 4;

    llama_context* raw_ctx = llama_init_from_model(raw_model, cparams);
    if (!raw_ctx) {
        std::cerr << "[llm] failed to create context" << std::endl;
        return false;
    }
    impl_->ctx.reset(raw_ctx);

    // ── Sampler chain: min_p → temp → top_p → distribution ──
    llama_sampler_chain_params sp = llama_sampler_chain_default_params();
    llama_sampler* chain = llama_sampler_chain_init(sp);
    llama_sampler_chain_add(chain, llama_sampler_init_penalties(64, 1.1f, 0.0f, 0.0f));
    llama_sampler_chain_add(chain, llama_sampler_init_top_k(40));
    llama_sampler_chain_add(chain, llama_sampler_init_top_p(config_.top_p, 1));
    llama_sampler_chain_add(chain, llama_sampler_init_temp(config_.temperature));
    llama_sampler_chain_add(chain, llama_sampler_init_dist(LLAMA_DEFAULT_SEED));
    impl_->sampler.reset(chain);

    loaded_.store(true);
    std::cerr << "[llm] ready — ctx=" << config_.context_size
              << " temp=" << config_.temperature
              << " top_p=" << config_.top_p
              << " n_gpu_layers=" << mparams.n_gpu_layers
              << std::endl;
    return true;
}

// ── Cancellation ───────────────────────────────────────────────────

void LlmSession::cancel() {
    cancel_flag_.store(true);
}

// ── Tokenization helpers ───────────────────────────────────────────

static std::vector<llama_token> tokenize(
    const llama_vocab* vocab,
    const std::string& text,
    bool add_special,
    bool parse_special
) {
    // First pass: get token count
    int n = -llama_tokenize(vocab, text.c_str(), text.size(),
                            nullptr, 0, add_special, parse_special);
    std::vector<llama_token> tokens(n);
    // Second pass: write tokens
    int got = llama_tokenize(vocab, text.c_str(), text.size(),
                             tokens.data(), tokens.size(),
                             add_special, parse_special);
    if (got < 0) tokens.clear();
    else         tokens.resize(got);
    return tokens;
}

static std::string token_to_piece(const llama_vocab* vocab, llama_token tok) {
    char buf[256];
    int n = llama_token_to_piece(vocab, tok, buf, sizeof(buf), 0, true);
    if (n <= 0) return "";
    return std::string(buf, n);
}

// ── Generate ───────────────────────────────────────────────────────

LlmSession::Result LlmSession::generate(const std::string& prompt, TokenCallback on_token) {
    std::lock_guard<std::mutex> lock(gen_mutex_);
    cancel_flag_.store(false);

    Result result;
    if (!loaded_.load()) {
        result.error = "model not loaded";
        return result;
    }

    llama_context* ctx    = impl_->ctx.get();
    llama_model*   model  = impl_->model.get();
    const llama_vocab* vocab = impl_->vocab;
    llama_sampler* smpl   = impl_->sampler.get();

    // Fresh KV cache per request — simpler than juggling conversation state
    llama_memory_clear(llama_get_memory(ctx), true);

    // ── Tokenize prompt ──
    auto t_start = std::chrono::steady_clock::now();
    std::vector<llama_token> prompt_tokens = tokenize(vocab, prompt, true, true);
    if (prompt_tokens.empty()) {
        result.error = "tokenization failed or empty prompt";
        return result;
    }

    int n_ctx = llama_n_ctx(ctx);
    if ((int)prompt_tokens.size() >= n_ctx) {
        result.error = "prompt too long for context window";
        return result;
    }
    result.stats.prompt_tokens = static_cast<int>(prompt_tokens.size());

    // ── Prefill ──
    llama_batch batch = llama_batch_get_one(prompt_tokens.data(), prompt_tokens.size());
    if (llama_decode(ctx, batch) != 0) {
        result.error = "prefill decode failed";
        return result;
    }
    auto t_prefill_done = std::chrono::steady_clock::now();
    result.stats.prefill_ms = std::chrono::duration_cast<std::chrono::milliseconds>(
        t_prefill_done - t_start).count();

    // ── Decode loop ──
    std::string output;
    output.reserve(1024);

    int n_decoded = 0;
    for (int i = 0; i < config_.max_tokens; i++) {
        if (cancel_flag_.load()) {
            result.error = "cancelled";
            break;
        }

        // Sample next token
        llama_token tok = llama_sampler_sample(smpl, ctx, -1);
        if (llama_vocab_is_eog(vocab, tok)) break;

        std::string piece = token_to_piece(vocab, tok);
        if (!piece.empty()) {
            output += piece;
            if (!on_token(piece)) break;
        }

        // Feed the sampled token back in for the next step
        llama_batch next = llama_batch_get_one(&tok, 1);
        if (llama_decode(ctx, next) != 0) {
            result.error = "decode step failed";
            break;
        }
        n_decoded++;
    }

    auto t_done = std::chrono::steady_clock::now();
    result.stats.decode_ms = std::chrono::duration_cast<std::chrono::milliseconds>(
        t_done - t_prefill_done).count();
    result.stats.output_tokens = n_decoded;
    result.output = std::move(output);

    double tok_per_sec = (result.stats.decode_ms > 0)
        ? (n_decoded * 1000.0 / result.stats.decode_ms)
        : 0.0;

    std::cerr << "[llm] gen stats: prompt=" << result.stats.prompt_tokens
              << " output=" << n_decoded
              << " prefill=" << result.stats.prefill_ms << "ms"
              << " decode=" << result.stats.decode_ms << "ms"
              << " (" << tok_per_sec << " tok/s)"
              << (result.error.empty() ? "" : " err=" + result.error)
              << std::endl;

    return result;
}

} // namespace peko
