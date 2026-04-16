#!/bin/bash
# patch_and_boot.sh — Patch Android emulator ROM with peko-agent and boot
#
# This script:
#   1. Installs a rootable system image (google_apis, NOT playstore)
#   2. Creates an AVD with writable system
#   3. Boots the emulator
#   4. Patches the system: injects peko-agent binary + init scripts + config
#   5. The binary persists across reboots (baked into /system)
#
# Usage:
#   ./emulator/patch_and_boot.sh                    # Full flow
#   ./emulator/patch_and_boot.sh --skip-build       # Reuse existing binary
#   ./emulator/patch_and_boot.sh --headless          # No GUI
#   ./emulator/patch_and_boot.sh --api 35            # Specific API level

set -euo pipefail

export JAVA_HOME="/Applications/Android Studio.app/Contents/jbr/Contents/Home"
export ANDROID_HOME=~/Library/Android/sdk
export PATH="$JAVA_HOME/bin:$ANDROID_HOME/cmdline-tools/latest/bin:$ANDROID_HOME/platform-tools:$ANDROID_HOME/emulator:$HOME/.cargo/bin:$PATH"

PROJECT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
AVD_NAME="peko_test"

GREEN='\033[0;32m'
RED='\033[0;31m'
CYAN='\033[0;36m'
YELLOW='\033[0;33m'
NC='\033[0m'

log()     { echo -e "${GREEN}[+]${NC} $1"; }
warn()    { echo -e "${YELLOW}[!]${NC} $1"; }
err()     { echo -e "${RED}[x]${NC} $1"; exit 1; }
section() { echo -e "\n${CYAN}═══ $1 ═══${NC}"; }

SKIP_BUILD=false
HEADLESS=false
API_LEVEL="35"

while [[ $# -gt 0 ]]; do
    case $1 in
        --skip-build) SKIP_BUILD=true; shift ;;
        --headless)   HEADLESS=true; shift ;;
        --api)        API_LEVEL="$2"; shift 2 ;;
        *)            shift ;;
    esac
done

# ═══════════════════════════════════════════════════════════
section "1. Detect host & select system image"
# ═══════════════════════════════════════════════════════════

HOST_ARCH=$(uname -m)
case "$HOST_ARCH" in
    arm64|aarch64) EMU_ABI="arm64-v8a"; RUST_TARGET="aarch64-linux-android"; NDK_PREFIX="aarch64-linux-android" ;;
    x86_64)        EMU_ABI="x86_64";    RUST_TARGET="x86_64-linux-android";  NDK_PREFIX="x86_64-linux-android" ;;
    *)             err "Unsupported host: $HOST_ARCH" ;;
esac

log "Host: $HOST_ARCH → Emulator ABI: $EMU_ABI → Rust: $RUST_TARGET"

# google_apis = rootable, google_apis_playstore = NOT rootable
SYS_IMAGE="system-images;android-${API_LEVEL};google_apis;${EMU_ABI}"

# ═══════════════════════════════════════════════════════════
section "2. Install rootable system image"
# ═══════════════════════════════════════════════════════════

INSTALLED=$(sdkmanager --list_installed 2>/dev/null | grep "google_apis;${EMU_ABI}" | grep "android-${API_LEVEL}" | grep -v playstore || true)
if [ -z "$INSTALLED" ]; then
    log "Installing $SYS_IMAGE ..."
    echo "y" | sdkmanager --install "$SYS_IMAGE" "platforms;android-${API_LEVEL}" 2>&1 | grep -v "^\[" | tail -5

    # Verify
    INSTALLED=$(sdkmanager --list_installed 2>/dev/null | grep "google_apis;${EMU_ABI}" | grep "android-${API_LEVEL}" | grep -v playstore || true)
    if [ -z "$INSTALLED" ]; then
        warn "google_apis image not available for API $API_LEVEL, trying default image..."
        SYS_IMAGE="system-images;android-${API_LEVEL};default;${EMU_ABI}"
        echo "y" | sdkmanager --install "$SYS_IMAGE" 2>&1 | tail -5
    fi
else
    log "System image already installed: $INSTALLED"
fi

# ═══════════════════════════════════════════════════════════
section "3. Create AVD"
# ═══════════════════════════════════════════════════════════

# Kill any running emulator first
adb devices 2>/dev/null | grep -q "emulator" && {
    warn "Stopping existing emulator..."
    adb emu kill 2>/dev/null || true
    sleep 3
}

if avdmanager list avd 2>/dev/null | grep -q "$AVD_NAME"; then
    log "Deleting existing AVD '$AVD_NAME'..."
    avdmanager delete avd -n "$AVD_NAME" 2>/dev/null || true
fi

log "Creating AVD '$AVD_NAME' (API $API_LEVEL, $EMU_ABI, google_apis)..."
echo "no" | avdmanager create avd \
    -n "$AVD_NAME" \
    -k "$SYS_IMAGE" \
    -d "pixel_4" \
    --force 2>&1 | tail -3

# Configure hardware
AVD_DIR="$HOME/.android/avd/${AVD_NAME}.avd"
cat >> "$AVD_DIR/config.ini" << 'AVDCFG'
hw.ramSize=2048
hw.keyboard=yes
hw.lcd.density=420
hw.lcd.width=1080
hw.lcd.height=2340
disk.dataPartition.size=4G
vm.heapSize=256
hw.gpu.enabled=yes
hw.gpu.mode=auto
hw.camera.back=none
hw.camera.front=none
hw.audioInput=no
hw.audioOutput=no
AVDCFG

log "AVD created"

# ═══════════════════════════════════════════════════════════
section "4. Cross-compile peko-agent"
# ═══════════════════════════════════════════════════════════

BINARY="$PROJECT_DIR/target/$RUST_TARGET/release/peko-agent"

if [ "$SKIP_BUILD" = false ]; then
    rustup target add "$RUST_TARGET" 2>/dev/null || true

    NDK_BIN="$ANDROID_HOME/ndk"
    NDK_VERSION=$(ls "$NDK_BIN" 2>/dev/null | sort -V | tail -1)
    NDK_TOOLCHAIN="$NDK_BIN/$NDK_VERSION/toolchains/llvm/prebuilt/darwin-x86_64/bin"
    export PATH="$NDK_TOOLCHAIN:$PATH"

    LINKER=$(ls "$NDK_TOOLCHAIN/${NDK_PREFIX}"*-clang 2>/dev/null | sort -V | tail -1)
    [ -n "$LINKER" ] || err "NDK linker not found for $NDK_PREFIX"
    log "NDK: $NDK_VERSION ($(basename "$LINKER"))"

    log "Building for $RUST_TARGET (release)..."
    cd "$PROJECT_DIR"
    cargo build --target "$RUST_TARGET" --release 2>&1 | tail -3
    cd - > /dev/null

    [ -f "$BINARY" ] || err "Build failed"
fi

[ -f "$BINARY" ] || err "Binary not found at $BINARY — run without --skip-build"
BIN_SIZE=$(du -h "$BINARY" | cut -f1)
log "Binary ready: $BIN_SIZE"

# ═══════════════════════════════════════════════════════════
section "5. Boot emulator"
# ═══════════════════════════════════════════════════════════

EMU_ARGS=(
    -avd "$AVD_NAME"
    -writable-system
    -no-snapshot
    -gpu host
    -memory 2048
    -partition-size 4096
)

if [ "$HEADLESS" = true ]; then
    EMU_ARGS+=(-no-window -no-audio -no-boot-anim)
fi

log "Starting emulator..."
emulator "${EMU_ARGS[@]}" &
EMU_PID=$!

log "Waiting for boot (PID: $EMU_PID)..."
adb wait-for-device

TIMEOUT=120
for i in $(seq 1 $TIMEOUT); do
    BOOT=$(adb shell getprop sys.boot_completed 2>/dev/null | tr -d '\r')
    [ "$BOOT" = "1" ] && break
    [ $i -eq $TIMEOUT ] && err "Boot timeout after ${TIMEOUT}s"
    sleep 1
done
log "Booted in ${i}s"

# ═══════════════════════════════════════════════════════════
section "6. Patch system with peko-agent"
# ═══════════════════════════════════════════════════════════

log "Getting root..."
adb root
sleep 2

log "Remounting /system writable..."
adb remount 2>/dev/null || {
    warn "remount failed, trying disable-verity..."
    adb disable-verity 2>/dev/null || true
    adb reboot
    log "Rebooting..."
    adb wait-for-device
    TIMEOUT=120
    for i in $(seq 1 $TIMEOUT); do
        BOOT=$(adb shell getprop sys.boot_completed 2>/dev/null | tr -d '\r')
        [ "$BOOT" = "1" ] && break
        sleep 1
    done
    adb root && sleep 2
    adb remount || err "Cannot remount system"
}

# Kill existing instance
adb shell "pkill peko-agent" 2>/dev/null || true
sleep 1

# Inject binary
log "Pushing peko-agent → /system/bin/"
adb push "$BINARY" /system/bin/peko-agent
adb shell chmod 755 /system/bin/peko-agent

# Inject init scripts
log "Pushing init scripts → /system/etc/init/"
adb shell mkdir -p /system/etc/init/
if [ -f "$PROJECT_DIR/rom/init/peko-agent.rc" ]; then
    adb push "$PROJECT_DIR/rom/init/peko-agent.rc" /system/etc/init/peko-agent.rc
    log "  peko-agent.rc (hybrid mode)"
fi
if [ -f "$PROJECT_DIR/rom/init/peko-frameworkless.rc" ]; then
    adb push "$PROJECT_DIR/rom/init/peko-frameworkless.rc" /system/etc/init/peko-frameworkless.rc
    log "  peko-frameworkless.rc"
fi

# Create data directory
log "Setting up /data/peko/"
adb shell mkdir -p /data/peko/sessions /data/peko/logs
adb shell chmod 770 /data/peko

# Detect hardware devices on this emulator
log "Detecting emulator hardware..."
INPUT_DEVS=$(adb shell "cat /proc/bus/input/devices" 2>/dev/null)
TOUCH_DEV=""
for dev in $(adb shell "ls /dev/input/" 2>/dev/null | tr -d '\r'); do
    NAME=$(echo "$INPUT_DEVS" | grep -B5 "$dev" | grep "Name=" | head -1 | sed 's/.*Name="//;s/".*//')
    LNAME=$(echo "$NAME" | tr '[:upper:]' '[:lower:]')
    if echo "$LNAME" | grep -qE "touch|ts|goldfish|virtio|qemu|ranchu|generic"; then
        TOUCH_DEV="/dev/input/$dev"
        log "  Touchscreen: $TOUCH_DEV ($NAME)"
        break
    fi
done

FB_DEV=""
adb shell "test -e /dev/graphics/fb0" 2>/dev/null && FB_DEV="/dev/graphics/fb0"
[ -z "$FB_DEV" ] && adb shell "test -e /dev/fb0" 2>/dev/null && FB_DEV="/dev/fb0"
[ -n "$FB_DEV" ] && log "  Framebuffer: $FB_DEV" || log "  Framebuffer: none (screencap fallback)"

# Write config
log "Writing /data/peko/config.toml"
HW_BLOCK=""
if [ -n "$TOUCH_DEV" ] || [ -n "$FB_DEV" ]; then
    HW_BLOCK="
[hardware]"
    [ -n "$TOUCH_DEV" ] && HW_BLOCK="${HW_BLOCK}
touchscreen_device = \"$TOUCH_DEV\""
    [ -n "$FB_DEV" ] && HW_BLOCK="${HW_BLOCK}
framebuffer_device = \"$FB_DEV\""
fi

adb shell "cat > /data/peko/config.toml" << ROMCFG
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
filesystem = true
shell = true

[tools.filesystem_config]
allowed_paths = ["/data/peko", "/sdcard"]

[tools.shell_config]
timeout_seconds = 30
${HW_BLOCK}
ROMCFG

# ═══════════════════════════════════════════════════════════
section "7. Start peko-agent"
# ═══════════════════════════════════════════════════════════

log "Starting agent..."
adb shell "nohup /system/bin/peko-agent \
    --config /data/peko/config.toml \
    --port 8080 \
    > /data/peko/peko.log 2>&1 &"

sleep 3

PID=$(adb shell "pidof peko-agent" 2>/dev/null | tr -d '\r')
if [ -z "$PID" ]; then
    echo ""
    err "Failed to start. Log:\n$(adb shell cat /data/peko/peko.log 2>/dev/null | tail -30)"
fi

log "peko-agent running (PID: $PID)"

# Port forward for host access
adb forward tcp:8080 tcp:8080 2>/dev/null

# ═══════════════════════════════════════════════════════════
section "8. Verify"
# ═══════════════════════════════════════════════════════════

PASS=0; FAIL=0
check() {
    if eval "$2" >/dev/null 2>&1; then
        echo -e "  ${GREEN}PASS${NC} $1"; ((PASS++))
    else
        echo -e "  ${RED}FAIL${NC} $1"; ((FAIL++))
    fi
}

check "Process alive"    "[ -n '$PID' ]"
check "Web UI"           "curl -s --max-time 5 http://localhost:8080/ | grep -q 'Peko'"
check "Status API"       "curl -s --max-time 5 http://localhost:8080/api/status | grep -q 'ready'"
check "Config API"       "curl -s --max-time 5 http://localhost:8080/api/config | grep -q 'provider'"
check "Sessions API"     "curl -s --max-time 5 http://localhost:8080/api/sessions"
check "Input devices"    "adb shell 'ls /dev/input/event*'"

# Memory
MEM=$(adb shell "cat /proc/$PID/status" 2>/dev/null | grep VmRSS | awk '{print $2}')
if [ -n "$MEM" ]; then
    RSS_MB=$((MEM / 1024))
    check "RSS < 50MB ($RSS_MB MB)" "[ $RSS_MB -lt 50 ]"
fi

# ═══════════════════════════════════════════════════════════
section "Done"
# ═══════════════════════════════════════════════════════════

echo ""
echo -e "  Host:    $HOST_ARCH"
echo -e "  Target:  $RUST_TARGET ($EMU_ABI, API $API_LEVEL)"
echo -e "  Binary:  $BIN_SIZE → /system/bin/peko-agent"
echo -e "  PID:     $PID"
echo -e "  ${GREEN}PASS: $PASS${NC}  ${RED}FAIL: $FAIL${NC}"
echo ""
echo -e "  ${CYAN}Web UI:${NC}  http://localhost:8080"
echo -e "  ${CYAN}Logs:${NC}    adb shell cat /data/peko/peko.log"
echo -e "  ${CYAN}Stop:${NC}    adb shell pkill peko-agent"
echo -e "  ${CYAN}Kill:${NC}    ./emulator/stop.sh"
echo ""

if [ "$FAIL" -gt 0 ]; then
    warn "Some checks failed — see: adb shell cat /data/peko/peko.log"
    exit 1
else
    echo -e "${GREEN}ROM patched and agent running. Open http://localhost:8080${NC}"
fi
