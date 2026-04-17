#include "chat_template.h"
#include <algorithm>
#include <cctype>

namespace peko {

static std::string to_lower(std::string s) {
    std::transform(s.begin(), s.end(), s.begin(),
        [](unsigned char c) { return std::tolower(c); });
    return s;
}

std::string format_prompt(ChatTemplate tmpl, const std::vector<ChatMessage>& messages) {
    std::string out;
    out.reserve(512);

    switch (tmpl) {
    case ChatTemplate::Gemma:
        // Gemma doesn't use a system role — merge system into first user message
        {
            std::string sys_text;
            for (const auto& m : messages) {
                if (m.role == "system") sys_text += m.content + "\n\n";
            }
            bool first_user = true;
            for (const auto& m : messages) {
                if (m.role == "system") continue;
                out += "<start_of_turn>";
                out += (m.role == "assistant") ? "model" : "user";
                out += "\n";
                if (first_user && m.role == "user" && !sys_text.empty()) {
                    out += sys_text;
                    first_user = false;
                }
                out += m.content;
                out += "<end_of_turn>\n";
            }
            out += "<start_of_turn>model\n";
        }
        break;

    case ChatTemplate::Qwen:
        for (const auto& m : messages) {
            out += "<|im_start|>";
            out += m.role;  // "system" | "user" | "assistant" all valid
            out += "\n";
            out += m.content;
            out += "<|im_end|>\n";
        }
        out += "<|im_start|>assistant\n";
        break;

    case ChatTemplate::Llama3:
        out += "<|begin_of_text|>";
        for (const auto& m : messages) {
            out += "<|start_header_id|>";
            out += m.role;
            out += "<|end_header_id|>\n\n";
            out += m.content;
            out += "<|eot_id|>";
        }
        out += "<|start_header_id|>assistant<|end_header_id|>\n\n";
        break;

    case ChatTemplate::Generic:
    default:
        for (const auto& m : messages) {
            out += m.role;
            out += ": ";
            out += m.content;
            out += "\n";
        }
        out += "assistant: ";
        break;
    }

    return out;
}

ChatTemplate detect_template(const std::string& model_hint) {
    std::string h = to_lower(model_hint);
    if (h.find("gemma") != std::string::npos) return ChatTemplate::Gemma;
    if (h.find("qwen")  != std::string::npos) return ChatTemplate::Qwen;
    if (h.find("llama-3") != std::string::npos || h.find("llama3") != std::string::npos)
        return ChatTemplate::Llama3;
    return ChatTemplate::Qwen; // safe default — ChatML is widely compatible
}

std::vector<std::string> stop_sequences(ChatTemplate tmpl) {
    switch (tmpl) {
    case ChatTemplate::Gemma:  return {"<end_of_turn>", "<start_of_turn>"};
    case ChatTemplate::Qwen:   return {"<|im_end|>", "<|im_start|>", "<|endoftext|>"};
    case ChatTemplate::Llama3: return {"<|eot_id|>", "<|end_of_text|>"};
    case ChatTemplate::Generic:
    default:                   return {"\nuser:", "\nsystem:"};
    }
}

} // namespace peko
