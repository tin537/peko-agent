#!/bin/bash
# deploy.sh — Deploy Peko Agent to a rooted Android device via ADB
#
# Usage:
#   ./scripts/deploy.sh                    # Build + deploy
#   ./scripts/deploy.sh --skip-build       # Deploy existing binary
#   ./scripts/deploy.sh --frameworkless    # Enable frameworkless mode
#
# Prerequisites:
#   - Rooted Android device connected via ADB
#   - Rust toolchain with aarch64-linux-android target
#   - Android NDK in PATH

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"
BINARY="$PROJECT_DIR/target/aarch64-linux-android/release/peko-agent"
DEVICE_BIN="/system/bin/peko-agent"
DEVICE_DATA="/data/peko"
DEVICE_INIT="/system/etc/init"

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m'

log()  { echo -e "${GREEN}[+]${NC} $1"; }
warn() { echo -e "${YELLOW}[!]${NC} $1"; }
err()  { echo -e "${RED}[x]${NC} $1"; exit 1; }

# ─── Parse args ────────────────────────────────────────────
SKIP_BUILD=false
FRAMEWORKLESS=false
for arg in "$@"; do
    case $arg in
        --skip-build) SKIP_BUILD=true ;;
        --frameworkless) FRAMEWORKLESS=true ;;
    esac
done

# ─── Check prerequisites ──────────────────────────────────
command -v adb >/dev/null || err "adb not found in PATH"
adb devices | grep -q "device$" || err "no Android device connected"

ARCH=$(adb shell getprop ro.product.cpu.abi | tr -d '\r')
log "Device architecture: $ARCH"
[[ "$ARCH" == *"arm64"* ]] || [[ "$ARCH" == *"aarch64"* ]] || err "Device is $ARCH, need arm64"

# Check root
adb shell su -c id 2>/dev/null | grep -q "uid=0" || err "Device is not rooted (su failed)"

# ─── Build ─────────────────────────────────────────────────
if [ "$SKIP_BUILD" = false ]; then
    log "Building for aarch64-linux-android (release)..."
    cd "$PROJECT_DIR"
    cargo build --target aarch64-linux-android --release 2>&1 | tail -5
    [ -f "$BINARY" ] || err "Build failed — binary not found"
    SIZE=$(du -h "$BINARY" | cut -f1)
    log "Binary size: $SIZE"
fi

[ -f "$BINARY" ] || err "Binary not found at $BINARY (run without --skip-build)"

# ─── Deploy ────────────────────────────────────────────────
log "Remounting /system read-write..."
adb shell su -c "mount -o remount,rw /system" 2>/dev/null || \
    adb shell su -c "mount -o remount,rw /" 2>/dev/null || \
    warn "Could not remount /system — trying direct push"

log "Pushing binary..."
adb push "$BINARY" /data/local/tmp/peko-agent
adb shell su -c "cp /data/local/tmp/peko-agent $DEVICE_BIN"
adb shell su -c "chmod 755 $DEVICE_BIN"
adb shell su -c "chown root:root $DEVICE_BIN"

log "Pushing init scripts..."
adb push "$PROJECT_DIR/rom/init/peko-agent.rc" /data/local/tmp/
adb shell su -c "cp /data/local/tmp/peko-agent.rc $DEVICE_INIT/peko-agent.rc"

if [ "$FRAMEWORKLESS" = true ]; then
    adb push "$PROJECT_DIR/rom/init/peko-frameworkless.rc" /data/local/tmp/
    adb shell su -c "cp /data/local/tmp/peko-frameworkless.rc $DEVICE_INIT/peko-frameworkless.rc"
    log "Frameworkless mode init script installed"
fi

log "Setting up data directory..."
adb shell su -c "mkdir -p $DEVICE_DATA"
adb shell su -c "chmod 770 $DEVICE_DATA"

# Push default config if none exists
if ! adb shell su -c "test -f $DEVICE_DATA/config.toml" 2>/dev/null; then
    log "Pushing default config..."
    adb push "$PROJECT_DIR/config/config.example.toml" /data/local/tmp/config.toml
    adb shell su -c "cp /data/local/tmp/config.toml $DEVICE_DATA/config.toml"
fi

# Set SELinux labels
log "Setting SELinux labels..."
adb shell su -c "chcon u:object_r:peko_agent_exec:s0 $DEVICE_BIN" 2>/dev/null || \
    warn "Could not set SELinux label (may need policy loaded first)"
adb shell su -c "restorecon -R $DEVICE_DATA" 2>/dev/null || true

# Remount read-only
adb shell su -c "mount -o remount,ro /system" 2>/dev/null || true

log "Cleaning up temp files..."
adb shell rm -f /data/local/tmp/peko-agent
adb shell rm -f /data/local/tmp/peko-agent.rc
adb shell rm -f /data/local/tmp/peko-frameworkless.rc
adb shell rm -f /data/local/tmp/config.toml

# ─── Verify ────────────────────────────────────────────────
log "Verifying installation..."
adb shell su -c "ls -la $DEVICE_BIN"
adb shell su -c "$DEVICE_BIN --help" 2>/dev/null && log "Binary runs!" || warn "Binary may need SELinux policy"

echo ""
log "Deploy complete!"
echo ""
echo "  Start agent:    adb shell su -c setprop sys.peko.start 1"
echo "  Stop agent:     adb shell su -c setprop sys.peko.start 0"
echo "  Manual run:     adb shell su -c $DEVICE_BIN --config $DEVICE_DATA/config.toml --port 8080"
echo "  Web UI:         http://<device-ip>:8080"
echo "  Edit config:    adb shell su -c vi $DEVICE_DATA/config.toml"
echo ""

if [ "$FRAMEWORKLESS" = true ]; then
    echo "  Enable frameworkless mode:"
    echo "    adb shell su -c setprop persist.peko.frameworkless 1"
    echo "    adb reboot"
    echo ""
    warn "WARNING: Frameworkless mode disables the entire Android framework!"
fi
