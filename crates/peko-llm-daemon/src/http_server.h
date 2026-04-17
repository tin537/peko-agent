#pragma once

#include <string>
#include <memory>
#include <atomic>
#include "llm_session.h"

namespace peko {

struct HttpServerConfig {
    std::string socket_path;       // UDS path, e.g. "/data/local/tmp/peko/llm.sock"
    std::string model_name;        // advertised in /v1/models
};

// Bind an HTTP server to a Unix Domain Socket and route OpenAI-compatible
// endpoints to the LLM session. Blocks until `stop()` is called or the
// process receives SIGTERM/SIGINT.
class HttpServer {
public:
    HttpServer(HttpServerConfig config, std::shared_ptr<LlmSession> session);
    ~HttpServer();

    // Start serving. Blocks.
    bool run();

    // Request shutdown — causes run() to return.
    void stop();

private:
    struct Impl;
    std::unique_ptr<Impl> impl_;
};

} // namespace peko
