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

The agent sees the screen (framebuffer/screencap), touches it (evdev injection), sends SMS + places calls through a bundled priv-app shim, **records + transcribes + summarises phone calls** into memory, installs apps (pm/installd), and learns from every interaction.

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
- **Multi-provider cloud chain** — `brain = "local:anthropic,openrouter"` tries anthropic first, falls back to openrouter on rate-limit/error. 7 cloud providers supported: anthropic, openrouter, openai, groq, deepseek, mistral, together.
- **13 tools** for full device control
- **Context compression** for long-running tasks
- **Iteration budget** with atomic interrupt
- **Session persistence** (SQLite)

### Learning Loop
- **Memory system** — FTS5 full-text search across persistent memories, auto-injected into prompts, periodic nudge to save important facts
- **Skills system** — Agent creates reusable procedures from experience, tracks success/failure rates, self-improves when steps fail
- **SOUL.md** — Customizable personality loaded from disk, editable via web UI

### Autonomy (Full Life — shipped Apr 2026)
- **Reflector** — auto-evaluates every completed task, writes a structured Reflection memory
- **Life Loop** — 60–300s heartbeat; decays drives, proposes exploration/review tasks when motivation thresholds cross
- **Motivation drives** — curiosity / competence / social / coherence, updated by code events (not LLM), persist across reboots
- **Curiosity** — proposes unseen tools to explore; dedupes against recent pending proposals
- **Goal generator** — pattern-driven "helpful" tasks derived from memory + user model
- **Memory gardener** — daily cron prunes low-importance, un-accessed memories + decays importance
- **Token budget** — sliding-24h cap on autonomy spend; gates each tick
- **Proposal expiry** — pending proposals older than 24h auto-expire
- **Propose-only by default** — queues for user approval; flip to auto-execute after trust is established

### Interfaces
- **Web UI** (port 8080) — Chat, Device monitor, Apps manager, Messages, **Calls** (voice-call transcripts + summaries, hidden when pipeline disabled), Memory, Skills, **Life** tab (drives + proposals + rate limits), Config editor with **Brain Mode picker** (Dual / Local only / Cloud only) + **Voice Calls** section (enable toggle, STT endpoint/model/key, retention) + **Security** section (lockscreen PIN)
- **Floating Peko overlay** (`com.peko.overlay`) — draggable cat mascot that auto-starts on every boot via `BootReceiver`, streams chat from `peko-agent` over localhost SSE. No user relaunch needed.
- **Telegram Bot** — Send tasks, receive responses + screenshots, /status /memories /skills commands
- **Cron Scheduler** — Autonomous recurring tasks with Telegram delivery

### Hardware Access
- **Touch injection** via evdev (`/dev/input/event*`) and uinput virtual devices
- **Screen capture** via framebuffer mmap or screencap fallback
- **SMS send + receive** via a bundled priv-app (`com.peko.shim.sms`) that holds the default-SMS role; `SmsManager.sendTextMessage()` under the hood, same code path as stock Messages. Falls back to AT-over-serial on old dev boards where RILD isn't holding the modem.
- **Voice calls** — place via `am start ACTION_CALL`, answer / hang up via KEYCODE_CALL / KEYCODE_ENDCALL. **Call recording + STT + LLM summary** via the same shim: two consent beeps play on the voice channel at the start of every call, audio is captured from the VOICE_CALL source, uploaded to an OpenAI-compatible `/audio/transcriptions` endpoint, summarised by the configured brain, and stored as an `Observation` memory keyed by caller + timestamp. Opt-in via `[calls].enabled` (Config UI).
- **Text input** via synthetic key injection
- **UI inspection** via uiautomator XML dump
- **Package management** via pm/am/installd
- **Lockscreen auto-unlock** via a configured PIN (Config UI → Security) so tasks can run on a dozing phone.

### Deploy Paths
- **Magisk module** ([`magisk/`](magisk/)) — works on any rooted ROM (stock, LineageOS, Pixel Experience). `./magisk/build-module.sh --install` builds + pushes + shows Magisk install steps. ~10 MB zip.
- **LineageOS overlay** ([`rom/lineage-fajita/`](rom/lineage-fajita/)) — bakes peko into a custom ROM for OnePlus 6T. Docker-based build host (`Dockerfile` + `docker-compose.yml`) for reproducible builds on Apple Silicon / Intel.
- **Stripped AOSP** ([`rom/agent-os/`](rom/agent-os/)) — frameworkless mode, peko IS the userspace. Scaffolded.
- **Rooted ADB** ([`scripts/deploy.sh`](scripts/deploy.sh)) — push binary to `/system/bin/` via USB or wireless ADB.

### ROM Integration (shared across deploy paths)
- `init.rc` service (hybrid + frameworkless modes)
- Complete SELinux policy (5 `.te` files) + Magisk `sepolicy.rule` for rooted installs
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

### Real Android Device — Magisk module (easiest)

```bash
# Build ARM64 binary + daemon, package as Magisk module, push to /sdcard
./magisk/build-module.sh --install

# On phone: Magisk app → Modules → Install from storage → pick zip → Reboot
# Verify
adb forward tcp:8080 tcp:8080 && open http://localhost:8080
```

Works on any rooted ROM (stock, LineageOS, Pixel Experience, crDroid). The
module ships peko-agent + peko-llm-daemon + default config; push a GGUF
model to `/data/peko/models/local.gguf` to enable the local brain.

### Real Android Device — init.rc service (rooted, non-Magisk)

```bash
# Build for ARM64
rustup target add aarch64-linux-android
cargo build --target aarch64-linux-android --release

# Deploy
./scripts/deploy.sh

# Start
adb shell su -c setprop sys.peko.start 1
```

### Custom LineageOS ROM (OnePlus 6T)

```bash
# Use the Docker build host — amd64 Ubuntu 22.04 with NDK + Rust + repo
docker compose -f rom/lineage-fajita/docker-compose.yml run --rm builder

# inside container (one-time):
./rom/lineage-fajita/build.sh --init     # repo sync (~80 GB)
# inside container (each build):
./rom/lineage-fajita/build.sh            # mka bacon

# flash from LineageOS recovery:
adb sideload out/target/product/fajita/lineage-21.0-*-fajita.zip
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
| Tests passing | 103 (lib + integration, workspace-wide) |
| Binary size (ARM64) | 6.8 MB (peko-agent) + 3.1 MB (peko-llm-daemon) |
| Runtime RSS | ~6 MB (agent) + 30–400 MB (daemon, model-dependent) |
| Agent tools | 13 |
| Cloud LLM providers supported | 7 (anthropic, openrouter, openai, groq, deepseek, mistral, together) + generic OpenAI-compat |
| Deploy paths | 4 (Magisk module, LineageOS overlay, stripped AOSP, rooted ADB) |
| Web UI tabs | 9 (Chat, Device, Apps, Messages, Calls, Memory, Skills, Life, Config) — Calls visible only when `[calls].enabled` |
| Autonomy phases shipped | 7/7 (A–G + token budget + proposal expiry) |

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

### Voice-call recording + summary

Opt-in; two short consent beeps play at the start of every recorded call so the
remote party is notified.

```toml
[calls]
enabled       = true
stt_base_url  = "https://api.openai.com/v1"   # any /audio/transcriptions endpoint
stt_model     = "whisper-1"
stt_api_key   = "sk-..."                       # or leave blank to read OPENAI_API_KEY
stt_language  = "en"                           # optional — blank = auto-detect
min_duration_ms   = 2000                       # skip pocket dials
retain_audio_days = 7                          # transcripts + summaries kept forever
```

Summaries land in both a dedicated `calls.db` table (visible in the **Calls**
tab of the web UI) and the main memory store as an `Observation` keyed by
`call:<number>:<ts>`, so the agent can bring up "you mentioned X on that call
last week" in later conversations.

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

Peko Agent is licensed under **[AGPL-3.0-or-later](LICENSE-AGPL)**, and only under
that license — there is no commercial alternative.

- Any use, modification, or distribution — including over a network (AGPL §13) — must comply with the AGPL, meaning Corresponding Source must be offered to all users of the service.
- If your deployment cannot meet AGPL's source-disclosure requirement (for example, a proprietary SaaS product), you cannot use Peko Agent.
- For the consolidated license summary, see [LICENSE](LICENSE).

## Community

- [CONTRIBUTING.md](CONTRIBUTING.md) — how to file bugs, propose changes, and send PRs
- [CODE_OF_CONDUCT.md](CODE_OF_CONDUCT.md) — expected conduct in issues, PRs, and chats
- [SECURITY.md](SECURITY.md) — responsible disclosure for security issues

### Third-party software

Peko Agent incorporates MIT-licensed code (cpp-httplib, nlohmann/json, llama.cpp, ggml). Full attributions and license texts:

- [NOTICE.md](NOTICE.md) — consolidated attribution list
- [third_party/LICENSES/](third_party/LICENSES/) — full MIT license texts

When a built binary is deployed as a network service, the web UI exposes `/source` and `/licenses` endpoints to satisfy AGPL §13 + MIT redistribution requirements.

## Author

Tanuphat Chainaloedwong
