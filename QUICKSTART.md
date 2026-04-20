# Quick Start Guide

Get Peko Agent running in 5 minutes.

---

## Option 1: Desktop (macOS / Linux)

The agent runs on your desktop with shell and filesystem tools. No Android device needed.

```bash
# 1. Install Rust
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source ~/.cargo/env

# 2. Clone and build
cd peko_agent
cargo build --release

# 3. Configure
cp config/config.example.toml config/config.toml
# Edit config/config.toml — set your LLM provider:
#   [provider.local]
#   model = "your-model"
#   base_url = "https://your-api.com/v1"
#   api_key = "your-key"

# 4. Run
cargo run -- --config config/config.toml --port 8080

# 5. Open browser
open http://localhost:8080
```

## Option 2: Android Emulator

Full device control in an emulator. Requires Android Studio.

### Prerequisites
- Android Studio installed
- NDK installed (Android Studio > Settings > SDK Tools > NDK)

### Setup

```bash
# 1. Create emulator (auto-detects ARM64 on Apple Silicon)
./emulator/setup_avd.sh

# 2. Start emulator
./emulator/start.sh

# 3. Build + deploy + test (auto-detects architecture)
./emulator/deploy_test.sh

# 4. Access web UI
# The script prints the URL, typically:
open http://localhost:18080
```

### Quick Redeploy (after code changes)

```bash
./emulator/deploy_test.sh   # full rebuild + deploy
# or
./emulator/deploy_test.sh --skip-build  # deploy existing binary
```

## Option 3: Real Android Device — Magisk module (recommended)

Works on any rooted Android (stock ROM, LineageOS, Pixel Experience, …). No
custom ROM build needed. `peko-agent` + `peko-llm-daemon` drop into
`/system/bin` via Magisk overlay.

### Prerequisites
- Bootloader unlocked + Magisk installed (`adb shell su -c id` shows `uid=0`)
- Android NDK 27+ at `~/Library/Android/sdk/ndk/…` (or env `ANDROID_NDK_HOME`)
- `rustup target add aarch64-linux-android`

### Deploy

```bash
# Cross-build + package + push to /sdcard + print Magisk install steps
./magisk/build-module.sh --install

# On the phone:
#   Magisk app → Modules → Install from storage → pick peko-magisk-*.zip
#   Reboot

# Forward the web UI
adb forward tcp:8080 tcp:8080 && open http://localhost:8080

# See what the hardware auto-probe found
adb shell cat /data/peko/detected_hardware.json
```

**Local LLM** (optional): push a GGUF model to the device, then the
`peko-llm-daemon` started by the Magisk module picks it up at boot:
```bash
adb push your-model.gguf /sdcard/
adb shell su -c 'mkdir -p /data/peko/models && mv /sdcard/your-model.gguf /data/peko/models/local.gguf'
adb shell su -c 'am kill bin.peko-agent ; sleep 2'    # or reboot
```

**Floating Peko overlay** ships in the same module. On the first reboot after
install, the overlay's `BootReceiver` fires, `service.sh` grants it
`SYSTEM_ALERT_WINDOW` + `POST_NOTIFICATIONS`, and the draggable cat mascot
appears on top of every app — no launcher tap required. Tap to open the chat
card, long-press to dismiss. See [`android/peko-overlay/README.md`](android/peko-overlay/README.md).

**SMS + Voice calls**: the bundled `com.peko.shim.sms` priv-app takes the
default-SMS role at boot (granted via `cmd role add-role-holder`), which
unlocks `SEND_SMS` / `RECEIVE_SMS` / `READ_SMS` and installs the incoming-SMS
receiver. The same shim hosts the call recording pipeline — see section
6 below for the config UI flow.

### Brain mode picker (Settings → Brain Mode in the web UI)

Three cards:
- **Dual** — local GGUF + cloud escalation
- **Local only** — just the GGUF
- **Cloud only** — just a cloud provider

Cloud dropdown: anthropic / openrouter / openai / groq / deepseek /
mistral / together. Picking a provider auto-fills a sensible default
model; override or leave blank.

Advanced: chain fallback via `provider.brain` in `config.toml`:
```toml
brain = "local:groq,anthropic,openrouter"   # local escalates to groq,
                                             # then anthropic, then openrouter
```

## Option 4: Real Android Device — init.rc service (non-Magisk)

For rooted devices where you prefer a classic init service (bakes into
`/system/bin` via `chmod +x`, starts via `class core`).

```bash
# 1. Build
rustup target add aarch64-linux-android
export PATH="$ANDROID_HOME/ndk/*/toolchains/llvm/prebuilt/*/bin:$PATH"
cargo build --target aarch64-linux-android --release

# 2. Deploy (auto-detects device, pushes binary, writes config)
./scripts/deploy.sh

# 3. Start
adb shell su -c setprop sys.peko.start 1

# 4. Access web UI
adb forward tcp:8080 tcp:8080 && open http://localhost:8080
```

### Frameworkless Mode (no Android framework)

```bash
./scripts/deploy.sh --frameworkless
adb shell su -c setprop persist.peko.frameworkless 1
adb reboot
# Device boots directly into the agent — no launcher, no apps, just Peko Agent
```

## Option 5: Custom LineageOS ROM (OnePlus 6T)

For when you want peko baked into the ROM itself rather than via Magisk.

```bash
# One-shot Docker build env (Ubuntu 22.04 + NDK + Rust + repo)
docker compose -f rom/lineage-fajita/docker-compose.yml run --rm builder

# Inside the container (first time):
./rom/lineage-fajita/build.sh --init    # repo sync ~80GB, 30-60min
./rom/lineage-fajita/build.sh           # mka bacon, 2-12hr first build

# Flash from LineageOS recovery:
adb sideload out/target/product/fajita/lineage-21.0-*-fajita.zip
```

See `rom/lineage-fajita/Dockerfile` + `docker-compose.yml` for the
build host spec, and `rom/lineage-fajita/peko_overlay.mk` for what
gets added to the ROM (strips ~25 AOSP apps, performance tuning,
peko + daemon preinstalled).

## First Steps After Setup

### 1. Pick Brain Mode + configure provider

Go to **Config** tab in the web UI, then **Brain Mode** section:

- **Dual** — local GGUF + cloud escalation. Enter GGUF path (e.g.
  `/data/peko/models/local.gguf`) and pick a cloud provider + API key.
- **Local only** — just the GGUF path. No cloud keys needed.
- **Cloud only** — pick a provider, paste API key. Default model is
  auto-filled; override if you want a different size.

Click **Save Changes**. The settings are persisted to `config.toml`
and the brain is rebuilt on next request.

### 2. Send Your First Task

Go to **Chat** tab:
```
Take a screenshot and describe what you see
```

### 3. Set Up Telegram (Optional)

1. Message @BotFather on Telegram > `/newbot` > copy token
2. Message @userinfobot > copy your user ID
3. Add to `config.toml`:
```toml
[telegram]
bot_token = "123456:ABC..."
allowed_users = [your_user_id]
```
4. Restart peko-agent

### 4. Add Scheduled Tasks (Optional)

Add to `config.toml`:
```toml
[[schedule]]
name = "status_check"
cron = "0 */6 * * *"
task = "Check battery level and WiFi status. Save to memory."
notify = "telegram"
```

### 5. Customize Personality

Go to **Config** tab > scroll to **SOUL.md** section > edit and save.

### 6. (Optional) Voice-call recording + summary

Requires the Magisk install path — the SMS shim priv-app (`com.peko.shim.sms`)
holds the privileged perms (`CAPTURE_AUDIO_OUTPUT`, `RECORD_AUDIO`,
`READ_CALL_LOG`) and also provides the SMS send/receive path. Once the module
is installed and the device rebooted:

1. Open **Config** tab → **Voice Calls** section.
2. Toggle **Record & summarise phone calls** on.
3. Paste an STT API key (any OpenAI-compatible `/audio/transcriptions`
   endpoint works — defaults target OpenAI's public Whisper).
4. Click **Save Changes**. The daemon re-reads `[calls]` from the live config
   on its next ~10s tick — no restart.

Now every call:
- plays two short 440 Hz beeps on the voice channel at the start (consent
  signal for both parties),
- records the audio to the shim's private files dir,
- is transcribed once the call ends,
- gets a one-paragraph LLM summary,
- lands in the **Calls** tab (web UI) and as an `Observation` memory keyed
  by `call:<number>:<timestamp>` so the agent remembers what was discussed.

State transitions: `recorded → transcribed → summarised`. Short calls
(< `min_duration_ms`) are marked `skipped`. STT / LLM failures surface as
`error` with a reason. The `.m4a` is kept until `retain_audio_days` passes
(default 7 days); transcripts + summaries are retained in the DB indefinitely.

Hardware caveat: `VOICE_CALL` source is OEM-gated on some chipsets — on the
OnePlus 6T the HAL accepts it, but if it ever refuses the pipeline falls back
to `VOICE_COMMUNICATION` / `MIC` which only captures the local side (audible
on speakerphone). See `android/peko-sms-shim/README.md`.

### 7. Turn on Autonomy (Full Life)

`[autonomy]` section of `config.toml`:

```toml
[autonomy]
enabled = true                     # master switch
tick_interval_secs = 300           # 60 for shakedown, 300 for daily use
propose_only = true                # queue proposals for approval (start here)
max_tokens_per_day = 50000         # cost safety — autonomous LLM cap
max_internal_tasks_per_hour = 4    # rate limit
max_internal_tasks_per_day = 20
memory_gardener = true             # daily prune + importance decay
memory_gardener_cron = "0 6 * * *"
reflection = true                  # auto-eval every completed task
curiosity = 0.10
goal_generation = true
```

Watch the **Life** tab: drives tick, proposals appear, you approve/reject.
After a week of propose-only you can flip `propose_only = false` for
auto-execution.

## Troubleshooting

### Agent doesn't respond to tasks
- Check Config tab — is the LLM provider configured with a valid API key?
- Check device logs: `adb shell cat /data/peko/peko.log`

### No touch/screenshot tools
- These require root access or framework running
- On emulator: tools auto-detect available hardware
- Check Device tab > Agent Tools for what's available

### Telegram bot not connecting
- Verify bot token with `curl https://api.telegram.org/bot<TOKEN>/getMe`
- Check that `allowed_users` contains your Telegram user ID
- Check logs for "telegram bot connected" message

### Build fails with linker error
- Ensure NDK is installed: `ls $ANDROID_HOME/ndk/`
- Ensure NDK bin is in PATH: `which aarch64-linux-android31-clang`

## Key URLs

| URL | Description |
|---|---|
| `http://localhost:8080` | Web UI (on device) |
| `http://localhost:18080` | Web UI (port-forwarded via ADB) |
| `/api/status` | Agent status + memory |
| `/api/config` | Configuration |
| `/api/memories` | Persistent memories |
| `/api/skills` | Learned skills |
| `/api/schedule` | Scheduled tasks |
| `/api/device/profile` | Device hardware profile |
| `/api/apps` | Installed applications |
