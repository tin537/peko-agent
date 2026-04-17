// peko-llm-daemon
//
// A small C++17 daemon that exposes an OpenAI-compatible LLM HTTP API
// over a Unix Domain Socket. Designed to run alongside peko-agent on
// Android — the agent connects via UDS (no TCP, no network stack).
//
// Phase 1 (current): stub LLM, full HTTP/UDS/SSE plumbing.
// Phase 2: real LiteRT-LM C++ inference engine linked in.

#include "config.h"
#include "http_server.h"
#include "llm_session.h"
#include "chat_template.h"

#include <iostream>
#include <string>
#include <signal.h>
#include <memory>
#include <cstring>

namespace {

peko::HttpServer* g_server = nullptr;

void signal_handler(int sig) {
    std::cerr << "\n[main] caught signal " << sig << ", shutting down..." << std::endl;
    if (g_server) g_server->stop();
}

void print_usage(const char* prog) {
    std::cerr <<
        "peko-llm-daemon " << peko::VERSION << "\n"
        "\n"
        "USAGE:\n"
        "  " << prog << " [OPTIONS]\n"
        "\n"
        "OPTIONS:\n"
        "  --model PATH            path to .litertlm or .gguf model file (required)\n"
        "  --socket PATH           Unix socket path (default: " << peko::DEFAULT_SOCKET_PATH << ")\n"
        "  --model-name NAME       name advertised via /v1/models (default: filename)\n"
        "  --context N             context window size in tokens (default: " << peko::DEFAULT_CONTEXT << ")\n"
        "  --max-tokens N          max generation length (default: " << peko::DEFAULT_MAX_TOKENS << ")\n"
        "  --temperature F         sampling temperature (default: " << peko::DEFAULT_TEMPERATURE << ")\n"
        "  --top-p F               nucleus sampling top-p (default: " << peko::DEFAULT_TOP_P << ")\n"
        "  --template NAME         chat template: gemma|qwen|llama3|auto (default: auto)\n"
        "  --help                  show this help\n"
        "\n"
        "ENDPOINTS (once running):\n"
        "  GET  /health\n"
        "  GET  /v1/models\n"
        "  POST /v1/chat/completions\n"
        "\n"
        "TEST:\n"
        "  curl --unix-socket " << peko::DEFAULT_SOCKET_PATH << " http://localhost/health\n"
        ;
}

struct Args {
    std::string model_path;
    std::string socket_path = peko::DEFAULT_SOCKET_PATH;
    std::string model_name;
    std::string template_name = "auto";
    int context      = peko::DEFAULT_CONTEXT;
    int max_tokens   = peko::DEFAULT_MAX_TOKENS;
    float temperature = peko::DEFAULT_TEMPERATURE;
    float top_p      = peko::DEFAULT_TOP_P;
    bool help = false;
};

bool parse_args(int argc, char** argv, Args& out) {
    for (int i = 1; i < argc; i++) {
        std::string a = argv[i];
        auto next = [&](const char* name) -> std::string {
            if (i + 1 >= argc) {
                std::cerr << "[args] " << name << " requires a value" << std::endl;
                return {};
            }
            return argv[++i];
        };

        if (a == "--help" || a == "-h")      { out.help = true; return true; }
        else if (a == "--model")             out.model_path = next("--model");
        else if (a == "--socket")            out.socket_path = next("--socket");
        else if (a == "--model-name")        out.model_name = next("--model-name");
        else if (a == "--context")           out.context = std::stoi(next("--context"));
        else if (a == "--max-tokens")        out.max_tokens = std::stoi(next("--max-tokens"));
        else if (a == "--temperature")       out.temperature = std::stof(next("--temperature"));
        else if (a == "--top-p")             out.top_p = std::stof(next("--top-p"));
        else if (a == "--template")          out.template_name = next("--template");
        else {
            std::cerr << "[args] unknown flag: " << a << std::endl;
            return false;
        }
    }
    return true;
}

peko::ChatTemplate template_from_name(const std::string& name, const std::string& model_hint) {
    if (name == "gemma")  return peko::ChatTemplate::Gemma;
    if (name == "qwen")   return peko::ChatTemplate::Qwen;
    if (name == "llama3") return peko::ChatTemplate::Llama3;
    if (name == "generic") return peko::ChatTemplate::Generic;
    return peko::detect_template(model_hint);
}

std::string basename(const std::string& path) {
    auto pos = path.find_last_of('/');
    auto name = (pos == std::string::npos) ? path : path.substr(pos + 1);
    auto dot = name.find_last_of('.');
    return (dot == std::string::npos) ? name : name.substr(0, dot);
}

} // namespace

int main(int argc, char** argv) {
    Args args;
    if (!parse_args(argc, argv, args)) { print_usage(argv[0]); return 2; }
    if (args.help)                    { print_usage(argv[0]); return 0; }

    if (args.model_path.empty()) {
        std::cerr << "[main] --model is required\n\n";
        print_usage(argv[0]);
        return 2;
    }

    if (args.model_name.empty()) {
        args.model_name = basename(args.model_path);
    }

    // Build LLM session
    peko::LlmConfig llm_cfg;
    llm_cfg.model_path    = args.model_path;
    llm_cfg.context_size  = args.context;
    llm_cfg.temperature   = args.temperature;
    llm_cfg.top_p         = args.top_p;
    llm_cfg.max_tokens    = args.max_tokens;
    llm_cfg.model_name    = args.model_name;
    llm_cfg.chat_template = template_from_name(args.template_name, args.model_path);

    std::cerr << "[main] peko-llm-daemon v" << peko::VERSION << std::endl;
    std::cerr << "[main] model:    " << llm_cfg.model_path << std::endl;
    std::cerr << "[main] name:     " << llm_cfg.model_name << std::endl;
    std::cerr << "[main] context:  " << llm_cfg.context_size << std::endl;
    std::cerr << "[main] socket:   " << args.socket_path << std::endl;

    auto session = std::make_shared<peko::LlmSession>(llm_cfg);
    if (!session->load()) {
        std::cerr << "[main] failed to load model" << std::endl;
        return 1;
    }

    // HTTP server
    peko::HttpServerConfig srv_cfg;
    srv_cfg.socket_path = args.socket_path;
    srv_cfg.model_name  = args.model_name;

    peko::HttpServer server(srv_cfg, session);
    g_server = &server;

    // Signal handling
    struct sigaction sa{};
    sa.sa_handler = signal_handler;
    sigemptyset(&sa.sa_mask);
    sigaction(SIGTERM, &sa, nullptr);
    sigaction(SIGINT,  &sa, nullptr);
    // SIGPIPE from client disconnects on SSE — ignore
    signal(SIGPIPE, SIG_IGN);

    std::cerr << "[main] ready. Serving on UDS." << std::endl;
    bool ok = server.run();
    std::cerr << "[main] shutdown " << (ok ? "clean" : "with errors") << std::endl;
    return ok ? 0 : 1;
}
