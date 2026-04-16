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

```
┌──────────────────────────────────────────────────────────┐
│  peko-agent (single binary, 4.1MB)                    │
│                                                          │
│  ┌─────────────┐  ┌──────────────┐  ┌─────────────────┐ │
│  │ Agent Loop   │  │ Web UI :8080 │  │ Telegram Bot    │ │
│  │ (ReAct)      │  │ 8 tabs       │  │ long-polling    │ │
│  └──────┬───────┘  └──────────────┘  └─────────────────┘ │
│         │                                                 │
│  ┌──────▼───────────────────────────────────────────────┐ │
│  │ Learning Loop                                         │ │
│  │ Memory (SQLite+FTS5) | Skills (md files) | SOUL.md   │ │
│  └──────────────────────────────────────────────────────┘ │
│                                                          │
│  ┌──────────────────────────────────────────────────────┐ │
│  │ 13 Tools                                              │ │
│  │ screenshot | touch | key | text_input | shell | fs   │ │
│  │ sms | call | ui_inspect | package_manager            │ │
│  │ memory | skills                                       │ │
│  └──────────────────────────────────────────────────────┘ │
│                                                          │
│  ┌──────────────────────────────────────────────────────┐ │
│  │ Hardware Abstraction Layer                            │ │
│  │ /dev/input/event* (evdev)  | /dev/graphics/fb0 (fb)  │ │
│  │ /dev/uinput (virtual)     | /dev/ttyACM* (modem)     │ │
│  │ screencap + uiautomator   | pm + am + installd       │ │
│  └──────────────────────────────────────────────────────┘ │
│                                                          │
│  ┌──────────────────────────────────────────────────────┐ │
│  │ LLM Transport (SSE streaming)                        │ │
│  │ Anthropic | OpenAI-compatible | Provider failover    │ │
│  └──────────────────────────────────────────────────────┘ │
│                                                          │
│  ┌────────────────┐                                      │
│  │ Cron Scheduler │ ── autonomous recurring tasks        │
│  └────────────────┘                                      │
└──────────────────────────────────────────────────────────┘
         │
         ▼
┌──────────────────┐
│  Linux Kernel    │
│  (Android ACK)   │
└──────────────────┘
```

## Features

### Agent Core
- **ReAct loop** with streaming LLM calls (SSE)
- **13 tools** for full device control
- **Context compression** for long-running tasks
- **Iteration budget** with atomic interrupt
- **Session persistence** (SQLite)
- **Provider failover** chain (Anthropic, OpenAI-compatible, local)

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
│   ├── peko-config/       # TOML config + env overrides
│   ├── peko-transport/    # SSE parser, LLM providers
│   ├── peko-core/         # Agent runtime, memory, skills, scheduler, cron
│   ├── peko-hal/          # Kernel device wrappers (evdev, fb, modem, uinput)
│   └── peko-tools-android/ # 13 tool implementations
├── src/
│   ├── main.rs              # Binary entry point
│   ├── web/                 # Web UI + REST API (axum)
│   └── telegram/            # Telegram bot gateway
├── rom/                     # ROM integration (init.rc, SELinux, build files)
├── emulator/                # AVD setup + deploy scripts
├── scripts/                 # Deploy, boot test, SELinux tools
├── config/                  # Example configuration
└── docs/                    # Obsidian knowledge base (48 docs)
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

Edit `config.toml` or use the web UI Config tab:

```toml
[agent]
max_iterations = 50
context_window = 200000
data_dir = "/data/peko"

[provider]
priority = ["local"]

[provider.local]
model = "mimo-v2-omni"
base_url = "https://api.xiaomimimo.com/v1"
api_key = "your-key"
max_tokens = 4096

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
