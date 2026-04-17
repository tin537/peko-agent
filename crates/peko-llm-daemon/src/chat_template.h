#pragma once

#include <string>
#include <vector>
#include "openai_api.h"

namespace peko {

// Supported chat templates
enum class ChatTemplate {
    Gemma,      // <start_of_turn>role\n...<end_of_turn>
    Qwen,       // <|im_start|>role\n...<|im_end|>
    Llama3,     // <|start_header_id|>role<|end_header_id|>\n\n...<|eot_id|>
    Generic,    // fallback: "role: content\n"
};

// Format a conversation into a prompt string for the model.
// Returns the string terminated with the assistant's generation prefix.
std::string format_prompt(
    ChatTemplate tmpl,
    const std::vector<ChatMessage>& messages
);

// Auto-detect template from model name or file path (best-effort)
ChatTemplate detect_template(const std::string& model_hint);

// Stop sequences for each template — the model should stop when generating these
std::vector<std::string> stop_sequences(ChatTemplate tmpl);

} // namespace peko
