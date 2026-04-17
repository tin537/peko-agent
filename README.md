# Peko Agent

**An autonomous AI agent that runs as the Android OS itself.**

Single Rust binary. PID-1 child. Direct kernel access. No framework. No Zygote. No ART.
4MB binary, 6MB RSS, boots from `init.rc`.

An Agent-as-OS architecture — a native Android system process with a closed learning loop, direct kernel hardware access, and autonomous task execution.

---

## What It Does

Peko Agent replaces the Android application stack with a single AI agent that controls the device directly through Linux kernel interfaces:

```
Traditional Android:  App -> Framework -> Binder -> Kernel -> Hardware
Peko Agent:        Agent -> Kernel -> Hardware
```

The agent sees the screen (framebuffer/screencap), touches it (evdev injection), sends SMS (AT commands), installs apps (pm/installd), and learns from every interaction.

## Architecture

Two cooperating processes on the device, linked by a Unix Domain Socket. The agent is pure Rust; the inference engine is C++ with llama.cpp. No HTTP over TCP between them.

```
┌───────────────────────────────────────────────────────────────┐
│ Android device                                                │
│                                                               │
│  peko-agent (Rust, ~7 MB)                                     │
│    ├─ Web UI :8080  ·  Telegram bot  ·  Cron scheduler        │
│    │                                                           │
│    ├─ Agent Loop (ReAct) + Dual-Brain router                  │
│    │     classifies each task → local brain or cloud brain    │
│    │     local brain can escalate to cloud mid-task           │
│    │                                                           │
│    ├─ Learning Loop: Memory (SQLite+FTS5), Skills, SOUL.md    │
│    │                                                           │
│    ├─ 13 Tools (screenshot/touch/shell/sms/…)                 │
│    │                                                           │
│    └─ LLM Transport                                            │
│        ├─ AnthropicProvider     (cloud, HTTPS)                │
│        ├─ OpenAICompatProvider  (cloud or local HTTP)         │
│        └─ UnixSocketProvider    ── HTTP/1.1 over UDS ──┐      │
│                                                         │      │
│                   @peko-llm (abstract socket namespace) │      │
│                                                         ▼      │
│  peko-llm-daemon (C++, ~3 MB)                                  │
│    ├─ cpp-httplib — HTTP/1.1 + SSE bound to UDS               │
│    ├─ OpenAI-compat routes (/v1/chat/completions, /v1/models) │
│    ├─ Chat templates: Gemma / Qwen / Llama3 / Generic         │
│    │                                                           │
│    └─ LlmSession → llama.cpp                                   │
│          ├─ ggml CPU backend (ARM NEON)                       │
│          └─ (optional) ggml-vulkan — Adreno/Mali GPU          │
│                                                                │
│  Hardware Abstraction Layer (Rust, called from peko-agent)    │
│    /dev/input/event* · /dev/graphics/fb0 · /dev/uinput         │
│    /dev/ttyACM* (modem) · screencap · uiautomator · pm/am      │
└───────────────────────────────────────────────────────────────┘
           │
           ▼
   ┌──────────────────┐
   │  Linux Kernel    │
   │  (Android ACK)   │
   └──────────────────┘
```

### Why two processes?

- **Isolation** — GPU driver crash in llama.cpp doesn't kill the agent
- **Language fit** — llama.cpp is C++; agent logic is Rust. No FFI pain.
- **Hot-swap** — update the inference binary without touching the agent
- **Zero network stack** — abstract-namespace UDS is faster than localhost TCP and SELinux-safe on Android `shell` user

## Features

### Agent Core
- **ReAct loop** with streaming LLM calls (SSE)
- **Dual-Brain router** — routes simple/skill-matched tasks to an on-device model, complex tasks to cloud; local brain can escalate
- **13 tools** for full device control
- **Context compression** for long-running tasks
- **Iteration budget** with atomic interrupt
- **Session persistence** (SQLite)
- **Provider failover** chain (Anthropic, OpenAI-compatible, UDS-local)

### Learning Loop
- **Memory system** — FTS5 full-text search across persistent memories, auto-injected into prompts, periodic nudge to save important facts
- **Skills system** — Agent creates reusable procedures from experience, tracks success/failure rates, self-improves when steps fail
- **SOUL.md** — Customizable personality loaded from disk, editable via web UI

### Interfaces
- **Web UI** (port 8080) — Chat, Device monitor, Apps manager, Messages stream, Memory browser, Skills viewer, Config editor with SOUL.md
- **Telegram Bot** — Send tasks, receive responses + screenshots, /status /memories /skills commands
- **Cron Scheduler** — Autonomous recurring tasks with Telegram delivery

### Hardware Access
- **Touch injection** via evdev (`/dev/input/event*`) and uinput virtual devices
- **Screen capture** via framebuffer mmap or screencap
- **SMS/Calls** via AT commands to serial modem
- **Text input** via synthetic key injection
- **UI inspection** via uiautomator XML dump
- **Package management** via pm/am/installd

### ROM Integration
- `init.rc` service (hybrid + frameworkless modes)
- Complete SELinux policy (5 files)
- AOSP build integration (Android.bp + device makefile)
- Deploy script, boot test, SELinux denial collector

## Quick Start

See [QUICKSTART.md](QUICKSTART.md) for detailed setup.

### Desktop Development (macOS/Linux)

```bash
# Install Rust
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# Build
cargo build --release

# Run (shell + filesystem tools work on desktop)
export OPENROUTER_API_KEY="your-key"
cargo run -- --config config/config.example.toml --port 8080

# Open http://localhost:8080
```

### Android Emulator

```bash
# Setup AVD (auto-detects Apple Silicon / Intel)
./emulator/setup_avd.sh

# Start emulator
./emulator/start.sh

# Build, deploy, test
./emulator/deploy_test.sh
```

### Real Android Device (rooted)

```bash
# Build for ARM64
rustup target add aarch64-linux-android
cargo build --target aarch64-linux-android --release

# Deploy
./scripts/deploy.sh

# Start
adb shell su -c setprop sys.peko.start 1
```

## Project Structure

```
peko-agent/
├── crates/
│   ├── peko-config/         # TOML config + env overrides
│   ├── peko-transport/      # SSE parser, LLM providers (incl. UDS)
│   ├── peko-core/           # Agent runtime, memory, skills, scheduler, brain router
│   ├── peko-hal/            # Kernel device wrappers (evdev, fb, modem, uinput)
│   ├── peko-tools-android/  # 13 tool implementations
│   ├── peko-llm/            # (experimental) pure-Rust embedded LLM via candle
│   └── peko-llm-daemon/     # C++ inference daemon — llama.cpp over UDS
│       ├── src/             # main.cpp, http_server.cpp, llm_session.cpp, …
│       ├── third_party/     # cpp-httplib, nlohmann/json (single-header)
│       ├── CMakeLists.txt   # FetchContent llama.cpp, optional Vulkan
│       ├── build-android.sh # NDK cross-compile script
│       └── deploy-test.sh   # deploy + smoke-test over adb
├── src/
│   ├── main.rs              # Binary entry point (peko-agent)
│   ├── web/                 # Web UI + REST API (axum)
│   └── telegram/            # Telegram bot gateway
├── rom/                     # ROM integration (init.rc, SELinux, build files)
├── emulator/                # AVD setup + deploy scripts
├── scripts/                 # Deploy, boot test, SELinux tools
├── config/                  # Example configuration (TCP + UDS variants)
└── docs/                    # Architecture + implementation notes
```

## Stats

| Metric | Value |
|---|---|
| Rust source files | 51 |
| Lines of Rust | 10,175 |
| Tests passing | 54 |
| Binary size (ARM64) | 4.1 MB |
| Runtime RSS | ~6 MB |
| Agent tools | 13 |
| API endpoints | 24 |
| Web UI tabs | 8 |
| Obsidian docs | 48 |

## Configuration

Edit `config.toml` or use the web UI Config tab.

### Cloud-only (simplest)

```toml
[provider]
priority = ["anthropic"]

[provider.anthropic]
# reads ANTHROPIC_API_KEY env var if api_key is empty
model = "claude-sonnet-4-20250514"
max_tokens = 4096
```

### Dual-brain (on-device llama.cpp + cloud escalation)

Start the daemon first:
```bash
./peko-llm-daemon --model /data/local/tmp/peko/models/qwen3-0.6b-q4_k_m.gguf \
    --socket "@peko-llm" --template qwen --context 2048 &
```

Then configure the agent:
```toml
[provider]
brain    = "local:anthropic"        # route simple tasks to local, escalate to cloud
priority = ["anthropic"]

[provider.local]
model    = "qwen3"
base_url = "unix://@peko-llm"        # abstract-namespace Unix Domain Socket
max_tokens = 512

[provider.anthropic]
model      = "claude-sonnet-4-20250514"
max_tokens = 4096
```

The `unix://` scheme routes through `UnixSocketProvider` which speaks HTTP/1.1 over the
abstract UDS — no TCP, no ports, no SELinux pain. The C++ daemon implements the OpenAI
Chat Completions protocol, so it's trivial to swap for any other OpenAI-compatible local
server (Ollama, llama.cpp server, vLLM).

### Telegram + scheduler

```toml
[telegram]
bot_token = "123456:ABC..."
allowed_users = [your_user_id]

[[schedule]]
name = "morning_check"
cron = "0 8 * * *"
task = "Check battery and WiFi status."
notify = "telegram"
```

## License

This project is dual-licensed:

- **Open source:** [AGPL-3.0-or-later](LICENSE-AGPL) — any use, modification, or distribution (including over a network) must comply with the AGPL.
- **Commercial:** A separate commercial license is available for proprietary use. See [LICENSE-COMMERCIAL](LICENSE-COMMERCIAL) or contact [tanuphat.chai@gmail.com](mailto:tanuphat.chai@gmail.com).

## Author

Tanuphat Chainaloedwong
