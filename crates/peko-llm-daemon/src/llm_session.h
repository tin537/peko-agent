#pragma once

#include <string>
#include <functional>
#include <mutex>
#include <atomic>
#include <memory>
#include "chat_template.h"

namespace peko {

struct LlmConfig {
    std::string model_path;
    int  context_size    = 2048;
    float temperature    = 0.7f;
    float top_p          = 0.9f;
    int   max_tokens     = 512;
    std::string model_name = "embedded";
    ChatTemplate chat_template = ChatTemplate::Qwen;
};

// Callback signature: return false to stop generation.
using TokenCallback = std::function<bool(const std::string& piece)>;

struct GenerateStats {
    int prompt_tokens = 0;
    int output_tokens = 0;
    int64_t prefill_ms = 0;
    int64_t decode_ms = 0;
};

// Thread-safe LLM session. All inference calls are serialized by an internal mutex —
// LiteRT-LM holds per-session state that can't be shared across concurrent generations.
class LlmSession {
public:
    explicit LlmSession(LlmConfig config);
    ~LlmSession();

    // Load the model. Expensive — call once at startup.
    bool load();

    // Run inference. Blocks the calling thread. `on_token` is called for every piece.
    // Returns the full generated text, or an error string on failure.
    struct Result {
        std::string output;
        std::string error;   // empty on success
        GenerateStats stats;
    };
    Result generate(const std::string& prompt, TokenCallback on_token);

    // Request cancellation — next on_token call returns false
    void cancel();

    const LlmConfig& config() const { return config_; }
    bool is_loaded() const { return loaded_.load(); }

private:
    LlmConfig config_;
    std::mutex gen_mutex_;      // serializes generate() calls
    std::atomic<bool> loaded_{false};
    std::atomic<bool> cancel_flag_{false};

    // PIMPL — hides LiteRT-LM headers from consumers
    struct Impl;
    std::unique_ptr<Impl> impl_;
};

} // namespace peko
