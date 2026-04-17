#pragma once

#include <string>
#include <vector>
#include <optional>
#include "json.hpp"

namespace peko {

// ── Incoming request (OpenAI Chat Completions) ─────────────────

struct ChatMessage {
    std::string role;       // "system" | "user" | "assistant"
    std::string content;
};

struct ChatRequest {
    std::string model;
    std::vector<ChatMessage> messages;
    bool stream = true;
    int max_tokens = 512;
    float temperature = 0.7f;
    float top_p = 0.9f;

    static ChatRequest from_json(const nlohmann::json& j);
};

// ── Outgoing response (streaming SSE deltas) ───────────────────

// Format: `data: {...}\n\n`
// Terminator: `data: [DONE]\n\n`

std::string sse_delta(const std::string& text);
std::string sse_done(const std::string& finish_reason);
std::string sse_done_marker();  // "data: [DONE]\n\n"
std::string sse_error(const std::string& message);

// ── Model listing ──────────────────────────────────────────────

std::string models_list_json(const std::string& model_name);

} // namespace peko
