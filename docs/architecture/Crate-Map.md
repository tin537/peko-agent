# Crate Map

> Cargo workspace structure and dependency graph.

---

## Workspace Layout

```
peko-agent/
├── Cargo.toml                  # Workspace root (Rust)
├── .cargo/
│   └── config.toml             # Cross-compilation settings
├── crates/
│   ├── peko-core/              # Agent brain + DualBrain router
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── runtime.rs      # AgentRuntime, run_task, run_turn
│   │       ├── brain.rs        # DualBrain, BrainChoice, escalate tool
│   │       ├── task_queue.rs   # TaskQueue (serialized executor)
│   │       ├── tool.rs         # Tool trait + ToolRegistry
│   │       ├── budget.rs       # IterationBudget
│   │       ├── compressor.rs   # ContextCompressor
│   │       ├── session.rs      # SessionStore (SQLite)
│   │       ├── memory.rs       # MemoryStore (FTS5)
│   │       ├── skills.rs       # SkillStore (markdown frontmatter)
│   │       ├── user_model.rs   # UserModel (prefs, patterns)
│   │       ├── scheduler.rs    # Cron scheduler
│   │       ├── mcp.rs          # MCP client
│   │       ├── prompt.rs       # SystemPrompt builder
│   │       └── message.rs      # Message types
│   │
│   ├── peko-transport/         # Network + IPC layer
│   │   └── src/
│   │       ├── provider.rs     # LlmProvider trait
│   │       ├── anthropic.rs    # AnthropicProvider (HTTPS)
│   │       ├── openai_compat.rs# OpenAICompatProvider (HTTPS/HTTP)
│   │       ├── unix_socket.rs  # UnixSocketProvider (HTTP over UDS)
│   │       ├── chain.rs        # ProviderChain (failover)
│   │       ├── sse.rs          # SseParser
│   │       └── stream.rs       # StreamEvent types
│   │
│   ├── peko-tools-android/     # 13 tools
│   │   └── src/ …
│   │
│   ├── peko-hal/               # Hardware abstraction
│   │   └── src/
│   │       ├── input.rs        # InputDevice (evdev)
│   │       ├── framebuffer.rs  # Framebuffer (/dev/fb0)
│   │       ├── uinput.rs       # UInputDevice (virtual)
│   │       ├── modem.rs        # SerialModem (AT)
│   │       ├── accessibility.rs# uiautomator XML parser
│   │       └── package_manager.rs
│   │
│   ├── peko-config/            # TOML config
│   │
│   ├── peko-llm/               # (experimental) Rust-embedded LLM via candle
│   │   └── src/
│   │       ├── engine.rs       # LlmEngine trait
│   │       ├── candle_backend.rs  # candle GGUF quantized
│   │       └── provider.rs     # EmbeddedProvider (impl LlmProvider)
│   │
│   └── peko-llm-daemon/        # C++ LLM daemon — llama.cpp over UDS
│       ├── CMakeLists.txt
│       ├── build-android.sh    # NDK cross-compile
│       ├── src/
│       │   ├── main.cpp
│       │   ├── http_server.cpp # cpp-httplib on UDS
│       │   ├── llm_session.cpp # llama.cpp wrapper
│       │   ├── chat_template.cpp
│       │   └── openai_api.cpp
│       └── third_party/        # httplib.h, json.hpp (single-header)
│
├── src/                        # peko-agent binary (Rust)
│   ├── main.rs                 # Entry point
│   ├── web/                    # axum Web UI + API
│   └── telegram/               # Telegram bot
│
├── config/                     # Example configs (TCP + UDS variants)
└── selinux/
    ├── peko_agent.te
    └── file_contexts
```

## Dependency Graph

```
peko-agent (binary)
├── peko-core
│   ├── peko-transport
│   │   ├── reqwest (rustls-tls)
│   │   ├── serde / serde_json
│   │   ├── tokio
│   │   └── async-trait
│   ├── peko-config
│   │   ├── serde
│   │   └── toml
│   ├── rusqlite (bundled)
│   ├── serde / serde_json
│   └── tokio
├── peko-tools-android
│   ├── peko-hal
│   │   ├── nix (ioctl wrappers)
│   │   └── libc
│   ├── peko-core (Tool trait)
│   ├── image (PNG encoding)
│   ├── base64
│   └── tokio
├── peko-hal
├── peko-config
├── tokio (full runtime)
├── tracing + tracing-subscriber
└── serde_json (JSON-RPC)
```

## Crate Coupling Rules

| From | Can depend on | Cannot depend on |
|---|---|---|
| `peko-core` | `peko-transport`, `peko-config` | `peko-hal`, `peko-tools-android`, `peko-llm` |
| `peko-transport` | (external + `libc` for UDS abstract namespace) | any other `peko-*` crate |
| `peko-tools-android` | `peko-core` (for `Tool` trait), `peko-hal`, `peko-config` | `peko-transport` |
| `peko-hal` | (external crates only) | any `peko-*` crate |
| `peko-config` | (external crates only) | any `peko-*` crate |
| `peko-llm` | `peko-transport` (impl `LlmProvider`) | `peko-core` |
| `peko-llm-daemon` | — (C++, separate build) | — (distinct process) |
| `peko-agent` | everything | — |

This ensures:
- **`peko-core` is platform-agnostic** — no Android-specific code leaks in
- **`peko-transport` is standalone** — pure HTTP/SSE, testable anywhere
- **`peko-hal` is leaf-level** — only talks to the kernel, no agent logic
- **Tools depend on core (for the trait) and HAL (for hardware)** — never on transport

## Inter-Crate Communication

All boundaries use **trait objects**:

```
peko-core defines:
  - trait Tool → implemented by peko-tools-android
  - trait LlmProvider → implemented by peko-transport
  - trait TokenCounter → implemented by peko-transport or peko-core

peko-agent wires them together:
  - Box<dyn LlmProvider> = AnthropicProvider::new(...)
  - Arc<dyn Tool> = TouchTool::new(hal_device)
  - AgentRuntime::new(config, tools, provider)
```

## Desktop Testing Strategy

Swap platform-specific crates with mocks:

```
peko-agent (binary)
├── peko-core          ← same
├── peko-transport     ← same
├── peko-tools-desktop ← mock tools (use xdotool, screenshot via X11)
├── peko-hal-mock      ← fake devices returning test data
└── peko-config        ← same
```

See [[../roadmap/Testing-Strategy]] for details.

## External Dependencies (Full List)

| Crate | Version | Purpose | Size impact |
|---|---|---|---|
| `tokio` | 1.x | Async runtime | ~1 MB |
| `reqwest` | 0.12+ | HTTP client | ~500 KB (with rustls) |
| `rustls` | 0.23+ | TLS (no OpenSSL) | ~400 KB |
| `serde` | 1.x | Serialization | ~200 KB |
| `serde_json` | 1.x | JSON parsing | ~100 KB |
| `rusqlite` | 0.31+ | SQLite (bundled) | ~1.5 MB |
| `image` | 0.25+ | PNG encoding only | ~300 KB |
| `nix` | 0.29+ | Safe ioctl wrappers | ~100 KB |
| `tracing` | 0.1 | Structured logging | ~50 KB |
| `toml` | 0.8+ | Config parsing | ~50 KB |
| `base64` | 0.22+ | Image encoding | ~20 KB |
| `async-trait` | 0.1 | Async trait support | minimal |

**Target binary size: < 15 MB** (statically linked where possible)

## Related

- [[Architecture-Overview]] — Where each crate sits in the system
- [[../implementation/peko-core]] — Core crate deep dive
- [[../implementation/peko-transport]] — Transport crate deep dive
- [[../knowledge/Cross-Compilation]] — How to build for Android

---

#architecture #crates #workspace
