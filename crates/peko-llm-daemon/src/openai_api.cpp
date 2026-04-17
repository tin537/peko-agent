#include "openai_api.h"
#include <sstream>
#include <chrono>

using nlohmann::json;

namespace peko {

ChatRequest ChatRequest::from_json(const json& j) {
    ChatRequest req;
    req.model = j.value("model", std::string(""));
    req.stream = j.value("stream", true);
    req.max_tokens = j.value("max_tokens", 512);
    req.temperature = j.value("temperature", 0.7f);
    req.top_p = j.value("top_p", 0.9f);

    if (j.contains("messages") && j["messages"].is_array()) {
        for (const auto& m : j["messages"]) {
            ChatMessage msg;
            msg.role = m.value("role", std::string("user"));
            // `content` can be string OR array of parts — we only handle string
            if (m.contains("content")) {
                if (m["content"].is_string()) {
                    msg.content = m["content"].get<std::string>();
                } else if (m["content"].is_array()) {
                    // concatenate text parts
                    for (const auto& part : m["content"]) {
                        if (part.is_object() && part.value("type", std::string("")) == "text") {
                            msg.content += part.value("text", std::string(""));
                        }
                    }
                }
            }
            req.messages.push_back(std::move(msg));
        }
    }

    return req;
}

static int64_t now_unix() {
    return std::chrono::duration_cast<std::chrono::seconds>(
        std::chrono::system_clock::now().time_since_epoch()
    ).count();
}

std::string sse_delta(const std::string& text) {
    json j = {
        {"id", "chatcmpl-peko"},
        {"object", "chat.completion.chunk"},
        {"created", now_unix()},
        {"model", "peko-embedded"},
        {"choices", json::array({
            {
                {"index", 0},
                {"delta", {{"content", text}}},
                {"finish_reason", nullptr}
            }
        })}
    };
    // Use error_handler_t::replace so we don't crash on incomplete UTF-8
    // (LLM tokens can be partial multi-byte chars split across emissions).
    std::ostringstream out;
    out << "data: " << j.dump(-1, ' ', false, nlohmann::json::error_handler_t::replace) << "\n\n";
    return out.str();
}

std::string sse_done(const std::string& finish_reason) {
    json j = {
        {"id", "chatcmpl-peko"},
        {"object", "chat.completion.chunk"},
        {"created", now_unix()},
        {"model", "peko-embedded"},
        {"choices", json::array({
            {
                {"index", 0},
                {"delta", json::object()},
                {"finish_reason", finish_reason}
            }
        })}
    };
    // Use error_handler_t::replace so we don't crash on incomplete UTF-8
    // (LLM tokens can be partial multi-byte chars split across emissions).
    std::ostringstream out;
    out << "data: " << j.dump(-1, ' ', false, nlohmann::json::error_handler_t::replace) << "\n\n";
    return out.str();
}

std::string sse_done_marker() {
    return "data: [DONE]\n\n";
}

std::string sse_error(const std::string& message) {
    json j = {
        {"error", {
            {"message", message},
            {"type", "server_error"},
            {"code", "inference_failed"}
        }}
    };
    // Use error_handler_t::replace so we don't crash on incomplete UTF-8
    // (LLM tokens can be partial multi-byte chars split across emissions).
    std::ostringstream out;
    out << "data: " << j.dump(-1, ' ', false, nlohmann::json::error_handler_t::replace) << "\n\n";
    return out.str();
}

std::string models_list_json(const std::string& model_name) {
    json j = {
        {"object", "list"},
        {"data", json::array({
            {
                {"id", model_name},
                {"object", "model"},
                {"created", now_unix()},
                {"owned_by", "peko-llm-daemon"}
            }
        })}
    };
    return j.dump();
}

} // namespace peko
