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

## Option 3: Real Android Device

Full Agent-as-OS on real hardware. Requires rooted device.

### Prerequisites
- Rooted Android device (Magisk recommended)
- ADB access
- NDK installed

### Deploy

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
adb forward tcp:8080 tcp:8080
open http://localhost:8080
```

### Frameworkless Mode (no Android framework)

```bash
./scripts/deploy.sh --frameworkless
adb shell su -c setprop persist.peko.frameworkless 1
adb reboot
# Device boots directly into the agent — no launcher, no apps, just Peko Agent
```

## First Steps After Setup

### 1. Configure LLM Provider

Go to **Config** tab in the web UI:
- Select provider (OpenAI-Compatible for most APIs)
- Enter API key
- Set model name and base URL
- Click **Save Changes**

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
