#!/bin/bash
# deploy_test.sh — Build, deploy, and test peko-agent on the emulator
#
# Supports any emulator architecture (arm64-v8a, x86_64).
# Auto-detects the running emulator's ABI and builds accordingly.
#
# Usage:
#   ./emulator/deploy_test.sh              # Full build + deploy + test
#   ./emulator/deploy_test.sh --skip-build # Deploy existing binary only

set -euo pipefail

export JAVA_HOME="/Applications/Android Studio.app/Contents/jbr/Contents/Home"
export ANDROID_HOME=~/Library/Android/sdk
export PATH="$JAVA_HOME/bin:$ANDROID_HOME/cmdline-tools/latest/bin:$ANDROID_HOME/platform-tools:$ANDROID_HOME/emulator:$HOME/.cargo/bin:$PATH"

PROJECT_DIR="$(cd "$(dirname "$0")/.." && pwd)"

GREEN='\033[0;32m'
RED='\033[0;31m'
CYAN='\033[0;36m'
NC='\033[0m'

log()     { echo -e "${GREEN}[+]${NC} $1"; }
err()     { echo -e "${RED}[x]${NC} $1"; exit 1; }
section() { echo -e "\n${CYAN}═══ $1 ═══${NC}"; }

SKIP_BUILD=false
[ "${1:-}" = "--skip-build" ] && SKIP_BUILD=true

# Check emulator is running
adb devices | grep -q "emulator\|device$" || err "No device running. Start with: ./emulator/start.sh"

# ═══════════════════════════════════════════════════════════
section "0. Detect emulator architecture"
# ═══════════════════════════════════════════════════════════

EMU_ABI=$(adb shell getprop ro.product.cpu.abi 2>/dev/null | tr -d '\r')
EMU_SDK=$(adb shell getprop ro.build.version.sdk 2>/dev/null | tr -d '\r')
EMU_MODEL=$(adb shell getprop ro.product.model 2>/dev/null | tr -d '\r')

log "Device: $EMU_MODEL (API $EMU_SDK, ABI: $EMU_ABI)"

case "$EMU_ABI" in
    arm64-v8a|aarch64)
        TARGET="aarch64-linux-android"
        NDK_PREFIX="aarch64-linux-android"
        ;;
    x86_64)
        TARGET="x86_64-linux-android"
        NDK_PREFIX="x86_64-linux-android"
        ;;
    armeabi-v7a|armeabi)
        TARGET="armv7-linux-androideabi"
        NDK_PREFIX="armv7a-linux-androideabi"
        ;;
    x86)
        TARGET="i686-linux-android"
        NDK_PREFIX="i686-linux-android"
        ;;
    *)
        err "Unsupported ABI: $EMU_ABI"
        ;;
esac

BINARY="$PROJECT_DIR/target/$TARGET/release/peko-agent"
log "Rust target: $TARGET"

# ═══════════════════════════════════════════════════════════
section "1. Build for $TARGET"
# ═══════════════════════════════════════════════════════════

if [ "$SKIP_BUILD" = false ]; then
    log "Adding Rust target..."
    rustup target add $TARGET 2>/dev/null || true

    # Find NDK
    NDK_BIN="$ANDROID_HOME/ndk"
    if [ -d "$NDK_BIN" ]; then
        NDK_VERSION=$(ls "$NDK_BIN" | sort -V | tail -1)
        NDK_TOOLCHAIN="$NDK_BIN/$NDK_VERSION/toolchains/llvm/prebuilt/darwin-x86_64/bin"

        # Pick highest available API level linker
        LINKER=$(ls "$NDK_TOOLCHAIN/${NDK_PREFIX}"*-clang 2>/dev/null | sort -V | tail -1)
        if [ -n "$LINKER" ]; then
            export PATH="$NDK_TOOLCHAIN:$PATH"
            log "Using NDK $NDK_VERSION ($(basename "$LINKER"))"
        else
            err "NDK linker not found for $NDK_PREFIX. Install NDK via Android Studio > SDK Tools > NDK"
        fi
    else
        err "Android NDK not found at $NDK_BIN"
    fi

    log "Building peko-agent for $TARGET (release)..."
    cd "$PROJECT_DIR"
    cargo build --target $TARGET --release 2>&1 | tail -5
    cd - > /dev/null

    [ -f "$BINARY" ] || err "Build failed — binary not found at $BINARY"
    log "Binary: $(du -h "$BINARY" | cut -f1)"
fi

[ -f "$BINARY" ] || err "Binary not found at $BINARY. Run without --skip-build"

# ═══════════════════════════════════════════════════════════
section "2. Deploy to emulator"
# ═══════════════════════════════════════════════════════════

log "Gaining root..."
adb root 2>/dev/null || true
sleep 2

log "Remounting system..."
adb remount 2>/dev/null || adb shell "mount -o remount,rw /system" 2>/dev/null || {
    log "Remount failed — trying disable-verity..."
    adb disable-verity 2>/dev/null || true
    adb reboot 2>/dev/null
    echo "[+] Waiting for reboot..."
    adb wait-for-device
    sleep 10
    adb root 2>/dev/null || true
    sleep 2
    adb remount 2>/dev/null || true
}

log "Killing any existing instance..."
adb shell "pkill peko-agent" 2>/dev/null || true
sleep 1

log "Pushing binary to /system/bin/peko-agent..."
adb push "$BINARY" /system/bin/peko-agent
adb shell chmod 755 /system/bin/peko-agent

log "Creating data directory..."
adb shell mkdir -p /data/peko

log "Pushing config..."
adb push "$PROJECT_DIR/config/config.example.toml" /data/peko/config.toml

# ═══════════════════════════════════════════════════════════
section "3. Detect emulator hardware"
# ═══════════════════════════════════════════════════════════

# Find touchscreen input device
TOUCH_DEV=""
for dev in $(adb shell "ls /dev/input/" 2>/dev/null | tr -d '\r'); do
    NAME=$(adb shell "cat /proc/bus/input/devices" 2>/dev/null | grep -A2 "$dev" | grep -i "Name" | head -1)
    if echo "$NAME" | grep -qi "touch\|ts\|virtio\|goldfish"; then
        TOUCH_DEV="/dev/input/$dev"
        log "Touchscreen: $TOUCH_DEV ($NAME)"
        break
    fi
done

# Find framebuffer
FB_DEV=""
if adb shell "test -e /dev/graphics/fb0" 2>/dev/null; then
    FB_DEV="/dev/graphics/fb0"
elif adb shell "test -e /dev/fb0" 2>/dev/null; then
    FB_DEV="/dev/fb0"
fi
[ -n "$FB_DEV" ] && log "Framebuffer: $FB_DEV" || log "No framebuffer (will use screencap fallback)"

# Check screencap availability
SCREENCAP_OK=$(adb shell "which screencap" 2>/dev/null | tr -d '\r')
[ -n "$SCREENCAP_OK" ] && log "screencap: available" || log "screencap: not found"

# Check uiautomator
UIAUTOMATOR_OK=$(adb shell "which uiautomator" 2>/dev/null | tr -d '\r')
[ -n "$UIAUTOMATOR_OK" ] && log "uiautomator: available" || log "uiautomator: not found"

# Write emulator-specific config with detected hardware
log "Writing emulator config with detected devices..."
HARDWARE_SECTION=""
if [ -n "$TOUCH_DEV" ] || [ -n "$FB_DEV" ]; then
    HARDWARE_SECTION="[hardware]"
    [ -n "$TOUCH_DEV" ] && HARDWARE_SECTION="$HARDWARE_SECTION
touchscreen_device = \"$TOUCH_DEV\""
    [ -n "$FB_DEV" ] && HARDWARE_SECTION="$HARDWARE_SECTION
framebuffer_device = \"$FB_DEV\""
fi

adb shell "cat > /data/peko/config.toml" << EOFCONFIG
[agent]
max_iterations = 50
context_window = 200000
history_share = 0.7
data_dir = "/data/peko"
log_level = "info"

[provider]
priority = ["openrouter"]

[provider.openrouter]
model = "ftmstars/peko-3-llama-3.1-405b"
base_url = "https://openrouter.ai/api/v1"
max_tokens = 4096

[tools]
screenshot = true
touch = true
key_event = true
text_input = true
sms = false
call = false
ui_dump = true
notification = true
filesystem = true
shell = true

[tools.filesystem_config]
allowed_paths = ["/data/peko", "/sdcard"]

[tools.shell_config]
timeout_seconds = 30

$HARDWARE_SECTION
EOFCONFIG

log "Config written to /data/peko/config.toml"

# ═══════════════════════════════════════════════════════════
section "4. Start peko-agent"
# ═══════════════════════════════════════════════════════════

log "Starting peko-agent on emulator..."
adb shell "nohup /system/bin/peko-agent \
    --config /data/peko/config.toml \
    --port 8080 \
    > /data/peko/peko.log 2>&1 &"

sleep 3

# Check if running
PID=$(adb shell "pidof peko-agent" 2>/dev/null | tr -d '\r')
if [ -n "$PID" ]; then
    log "peko-agent running! PID: $PID"
else
    err "Failed to start. Log:\n$(adb shell cat /data/peko/peko.log 2>/dev/null | tail -20)"
fi

# ═══════════════════════════════════════════════════════════
section "5. Test"
# ═══════════════════════════════════════════════════════════

PASS=0
FAIL=0

check() {
    if eval "$2" >/dev/null 2>&1; then
        echo -e "  ${GREEN}PASS${NC} $1"
        ((PASS++))
    else
        echo -e "  ${RED}FAIL${NC} $1"
        ((FAIL++))
    fi
}

# Port forward
adb forward tcp:8080 tcp:8080 2>/dev/null

# Tests
check "Process running" "[ -n '$PID' ]"

check "Web UI responds" "curl -s --max-time 5 http://localhost:8080/ | grep -q 'Peko'"

check "Status API" "curl -s --max-time 5 http://localhost:8080/api/status | grep -q 'ready'"

check "Config API" "curl -s --max-time 5 http://localhost:8080/api/config | grep -q 'provider'"

check "Sessions API" "curl -s --max-time 5 http://localhost:8080/api/sessions"

# Hardware access checks
check "Input devices exist" "adb shell 'ls /dev/input/event*' 2>/dev/null"

if [ -n "$SCREENCAP_OK" ]; then
    check "screencap works" "adb shell 'screencap -p /data/peko/test_screenshot.png && test -s /data/peko/test_screenshot.png'"
    adb shell "rm -f /data/peko/test_screenshot.png" 2>/dev/null || true
fi

# Memory check
RSS_INFO=$(curl -s http://localhost:8080/api/status 2>/dev/null)
echo ""
log "Status: $RSS_INFO"

MEM_INFO=$(adb shell "cat /proc/$PID/status" 2>/dev/null | grep -E "VmRSS|VmSize|Threads")
echo ""
log "Process memory:"
echo "$MEM_INFO" | sed 's/^/    /'

RSS_KB=$(echo "$MEM_INFO" | grep VmRSS | awk '{print $2}')
if [ -n "$RSS_KB" ]; then
    RSS_MB=$((RSS_KB / 1024))
    if [ "$RSS_MB" -lt 50 ]; then
        echo -e "  ${GREEN}PASS${NC} RSS: ${RSS_MB}MB < 50MB target"
        ((PASS++))
    else
        echo -e "  ${RED}WARN${NC} RSS: ${RSS_MB}MB exceeds 50MB target"
        ((FAIL++))
    fi
fi

# ═══════════════════════════════════════════════════════════
section "Results"
# ═══════════════════════════════════════════════════════════

echo ""
echo -e "  Device:  $EMU_MODEL ($EMU_ABI, API $EMU_SDK)"
echo -e "  Target:  $TARGET"
echo -e "  ${GREEN}PASS: $PASS${NC}  ${RED}FAIL: $FAIL${NC}"
echo ""
echo "  Web UI:  http://localhost:8080"
echo "  Logs:    adb shell cat /data/peko/peko.log"
echo "  Shell:   adb shell"
echo "  Stop:    adb shell pkill peko-agent"
echo ""

if [ "$FAIL" -gt 0 ]; then
    echo -e "${RED}Some tests failed.${NC}"
    echo "Check logs: adb shell cat /data/peko/peko.log"
    exit 1
else
    echo -e "${GREEN}All tests passed! Open http://localhost:8080 in your browser.${NC}"
fi
