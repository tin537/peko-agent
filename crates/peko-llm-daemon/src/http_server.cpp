#include "http_server.h"
#include "openai_api.h"
#include "chat_template.h"
#include "config.h"

#define CPPHTTPLIB_NO_EXCEPTIONS  // we use nlohmann/json's exceptions only
#include "httplib.h"
#include "json.hpp"

#include <iostream>
#include <sys/stat.h>
#include <sys/un.h>
#include <unistd.h>
#include <errno.h>
#include <cstring>
#include <filesystem>

namespace peko {

namespace fs = std::filesystem;
using nlohmann::json;

struct HttpServer::Impl {
    HttpServerConfig cfg;
    std::shared_ptr<LlmSession> session;
    httplib::Server srv;
    std::atomic<bool> running{false};

    Impl(HttpServerConfig c, std::shared_ptr<LlmSession> s)
        : cfg(std::move(c)), session(std::move(s)) {
        install_routes();
    }

    void install_routes();
    bool bind_unix_socket();
};

// ── Routes ─────────────────────────────────────────────────────

void HttpServer::Impl::install_routes() {
    // Health check
    srv.Get("/health", [this](const httplib::Request&, httplib::Response& res) {
        json j = {
            {"status",  "ok"},
            {"model",   cfg.model_name},
            {"loaded",  session->is_loaded()},
            {"version", VERSION},
        };
        res.set_content(j.dump(), "application/json");
    });

    // Model listing (OpenAI-compat)
    srv.Get("/v1/models", [this](const httplib::Request&, httplib::Response& res) {
        res.set_content(models_list_json(cfg.model_name), "application/json");
    });

    // Chat completions — streaming SSE
    srv.Post("/v1/chat/completions", [this](const httplib::Request& req, httplib::Response& res) {
        // Parse request body
        ChatRequest chat_req;
        try {
            auto body = json::parse(req.body);
            chat_req = ChatRequest::from_json(body);
        } catch (const std::exception& e) {
            res.status = 400;
            res.set_content(
                json{{"error", {{"message", std::string("invalid JSON: ") + e.what()}}}}.dump(),
                "application/json");
            return;
        }

        if (chat_req.messages.empty()) {
            res.status = 400;
            res.set_content(
                json{{"error", {{"message", "messages[] is required"}}}}.dump(),
                "application/json");
            return;
        }

        // Format prompt via chat template
        auto tmpl = session->config().chat_template;
        std::string prompt = format_prompt(tmpl, chat_req.messages);

        if (!chat_req.stream) {
            // Non-streaming: run to completion, return single JSON response
            auto result = session->generate(prompt, [](const std::string&) { return true; });
            if (!result.error.empty()) {
                res.status = 500;
                res.set_content(
                    json{{"error", {{"message", result.error}}}}.dump(),
                    "application/json");
                return;
            }
            json resp = {
                {"id", "chatcmpl-peko"},
                {"object", "chat.completion"},
                {"model", cfg.model_name},
                {"choices", json::array({
                    {
                        {"index", 0},
                        {"message", {{"role", "assistant"}, {"content", result.output}}},
                        {"finish_reason", "stop"}
                    }
                })},
                {"usage", {
                    {"prompt_tokens", result.stats.prompt_tokens},
                    {"completion_tokens", result.stats.output_tokens},
                    {"total_tokens", result.stats.prompt_tokens + result.stats.output_tokens}
                }}
            };
            res.set_content(resp.dump(), "application/json");
            return;
        }

        // Streaming: SSE with chunked encoding
        // We use set_chunked_content_provider which gives us a DataSink to push chunks into.
        res.set_header("Cache-Control", "no-cache");
        res.set_header("Connection", "keep-alive");
        res.set_header("X-Accel-Buffering", "no");

        // IMPORTANT: must capture `this` + request data by value — the provider
        // runs on a separate thread, request object may go out of scope.
        res.set_chunked_content_provider(
            "text/event-stream",
            [this, prompt = std::move(prompt)](size_t /*offset*/, httplib::DataSink& sink) {
                auto result = session->generate(prompt,
                    [&sink](const std::string& piece) -> bool {
                        auto chunk = sse_delta(piece);
                        if (!sink.write(chunk.data(), chunk.size())) {
                            // client disconnected — signal stop
                            return false;
                        }
                        return true;
                    });

                if (!result.error.empty()) {
                    auto err = sse_error(result.error);
                    sink.write(err.data(), err.size());
                } else {
                    auto done = sse_done("stop");
                    sink.write(done.data(), done.size());
                }
                auto marker = sse_done_marker();
                sink.write(marker.data(), marker.size());
                sink.done();
                return true;
            });
    });
}

// ── Unix Domain Socket binding ─────────────────────────────────

bool HttpServer::Impl::bind_unix_socket() {
    const auto& path = cfg.socket_path;

    // Ensure parent directory exists
    try {
        auto parent = fs::path(path).parent_path();
        if (!parent.empty() && !fs::exists(parent)) {
            fs::create_directories(parent);
        }
    } catch (const std::exception& e) {
        std::cerr << "[http] failed to create parent dir: " << e.what() << std::endl;
        return false;
    }

    // Clean up stale socket file if present
    if (fs::exists(path)) {
        std::cerr << "[http] removing stale socket: " << path << std::endl;
        unlink(path.c_str());
    }

    // cpp-httplib 0.18+ provides bind_to_any_unix_domain_socket via set_address_family
    // Workaround: use AF_UNIX socket directly, then hand fd to cpp-httplib's internals.
    //
    // httplib::Server doesn't expose UDS directly but does expose bind via hostname.
    // We rely on the fact that cpp-httplib calls socket(AF_INET/AF_INET6) — for UDS
    // we need to pre-bind and use set_read_timeout / listen manually.
    //
    // SIMPLER APPROACH: use cpp-httplib's `bind_to_socket(fd)` helper if available.
    // In v0.18.x that's not standard — so we use a workaround: bind to 127.0.0.1 on
    // a high ephemeral port and rely on UDS being layered via an OS-level redirect.
    //
    // BEST APPROACH: cpp-httplib v0.19+ has direct UDS support.
    // For our cpp-httplib v0.18.7, we use the `Server::set_address_family` with AF_UNIX.
    //
    // Checking httplib.h version: if `set_address_family` exists, prefer that route.

    // cpp-httplib 0.18+ exposes `set_address_family(AF_UNIX)` and then listen("socket_path").
    // Let's try that path.

    srv.set_address_family(AF_UNIX);
    std::cerr << "[http] listening on UDS: " << path << std::endl;
    return true;
}

// ── Public API ─────────────────────────────────────────────────

HttpServer::HttpServer(HttpServerConfig config, std::shared_ptr<LlmSession> session)
    : impl_(std::make_unique<Impl>(std::move(config), std::move(session))) {}

HttpServer::~HttpServer() { stop(); }

bool HttpServer::run() {
    if (!impl_->bind_unix_socket()) return false;

    impl_->running.store(true);

    // `listen` is blocking. For AF_UNIX the port is ignored, but cpp-httplib
    // treats port=0 as "any port" and then fails to resolve it for AF_UNIX.
    // So we pass an arbitrary non-zero value.
    // Port must be non-zero (cpp-httplib treats 0 as "any" and fails for AF_UNIX)
    // and non-privileged (Android denies bind on port < 1024 for shell user).
    // Value is ignored for AF_UNIX but still passed through bind().
    bool ok = impl_->srv.listen(impl_->cfg.socket_path, /*port=*/8889);
    if (!ok) {
        std::cerr << "[http] srv.listen failed (errno=" << errno << " " << strerror(errno) << ")" << std::endl;
    }

    impl_->running.store(false);

    // Clean up socket file on exit
    unlink(impl_->cfg.socket_path.c_str());
    return ok;
}

void HttpServer::stop() {
    if (impl_->running.load()) {
        impl_->srv.stop();
    }
}

} // namespace peko
