#pragma once

// Compile-time defaults. Overridable via CLI args.

namespace peko {

constexpr const char* DEFAULT_SOCKET_PATH = "/data/local/tmp/peko/llm.sock";
constexpr const char* DEFAULT_MODEL_NAME  = "embedded";
constexpr int         DEFAULT_MAX_TOKENS  = 512;
constexpr float       DEFAULT_TEMPERATURE = 0.7f;
constexpr float       DEFAULT_TOP_P       = 0.9f;
constexpr int         DEFAULT_CONTEXT     = 2048;

constexpr const char* VERSION = "0.1.0";
constexpr const char* USER_AGENT = "peko-llm-daemon/0.1.0";

// Single-file-header deps
// #include "httplib.h"  — cpp-httplib v0.18+
// #include "json.hpp"   — nlohmann/json v3.11+

} // namespace peko
