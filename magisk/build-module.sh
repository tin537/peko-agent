#!/usr/bin/env bash
# build-module.sh — Assemble the peko-agent Magisk module zip.
#
# Usage:
#   ./magisk/build-module.sh                  # build with current binaries
#   ./magisk/build-module.sh --skip-build     # don't recompile, just package
#   ./magisk/build-module.sh --install        # build + adb push + trigger
#                                               Magisk install on device
#
# Output: magisk/out/peko-magisk-<version>.zip
#
# Install on phone (manual):
#   adb push peko-magisk-*.zip /sdcard/Download/
#   Open Magisk app → Modules → Install from storage → pick the zip
#   Reboot.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(dirname "$SCRIPT_DIR")"
MODULE_DIR="$SCRIPT_DIR/peko-module"
OUT_DIR="$SCRIPT_DIR/out"
VERSION="$(grep -E '^version=' "$MODULE_DIR/module.prop" | cut -d= -f2)"
ZIP="$OUT_DIR/peko-magisk-${VERSION}.zip"

SKIP_BUILD=false
DO_INSTALL=false
WITH_OVERLAY=false
for a in "$@"; do
    case "$a" in
        --skip-build)   SKIP_BUILD=true ;;
        --install)      DO_INSTALL=true ;;
        --with-overlay) WITH_OVERLAY=true ;;
        *) echo "unknown arg: $a"; exit 1 ;;
    esac
done

# ─── Cross-compile peko-agent for aarch64-linux-android ───────────
if [ "$SKIP_BUILD" = false ]; then
    echo "[+] Building peko-agent (cargo, release, arm64)"
    (
        cd "$REPO_ROOT"
        # NDK path needed for sqlite native code — tweak version if yours differs
        NDK_PATH="$HOME/Library/Android/sdk/ndk/30.0.14904198/toolchains/llvm/prebuilt/darwin-x86_64/bin"
        [ -d "$NDK_PATH" ] || NDK_PATH="$HOME/Library/Android/sdk/ndk/27.1.12297006/toolchains/llvm/prebuilt/darwin-x86_64/bin"
        [ -d "$NDK_PATH" ] || { echo "[x] Android NDK not found at expected paths"; exit 1; }
        PATH="$NDK_PATH:$PATH" \
        CC_aarch64_linux_android=aarch64-linux-android31-clang \
        cargo build --target aarch64-linux-android --release
    )
fi

# ─── Stage binaries into the module tree ──────────────────────────
AGENT_BIN="$REPO_ROOT/target/aarch64-linux-android/release/peko-agent"
DAEMON_BIN="$REPO_ROOT/crates/peko-llm-daemon/build-android-arm64-v8a/peko-llm-daemon"

[ -f "$AGENT_BIN" ] || { echo "[x] peko-agent not built at $AGENT_BIN"; exit 1; }
install -m 0755 "$AGENT_BIN" "$MODULE_DIR/system/bin/peko-agent"

if [ -f "$DAEMON_BIN" ]; then
    install -m 0755 "$DAEMON_BIN" "$MODULE_DIR/system/bin/peko-llm-daemon"
else
    echo "[!] peko-llm-daemon not built — module will ship without local LLM."
    echo "    Build it via crates/peko-llm-daemon/build-android.sh first if needed."
    rm -f "$MODULE_DIR/system/bin/peko-llm-daemon"
fi

# Copy SOUL.md if present
if [ -f "$REPO_ROOT/SOUL.md" ]; then
    install -m 0644 "$REPO_ROOT/SOUL.md" "$MODULE_DIR/system/etc/peko/SOUL.md"
fi

# ─── Optional: Android overlay APK as priv-app ────────────────────
OVERLAY_DIR="$REPO_ROOT/android/peko-overlay"
OVERLAY_APK="$OVERLAY_DIR/app/build/outputs/apk/release/app-release-unsigned.apk"
PRIV_APP_DIR="$MODULE_DIR/system/priv-app/PekoOverlay"

if [ "$WITH_OVERLAY" = true ]; then
    echo "[+] Building peko-overlay APK (Gradle, release)"
    (
        cd "$OVERLAY_DIR"
        ./gradlew :app:assembleRelease
    )
fi

if [ -f "$OVERLAY_APK" ]; then
    echo "[+] Including PekoOverlay.apk as priv-app"
    mkdir -p "$PRIV_APP_DIR"
    install -m 0644 "$OVERLAY_APK" "$PRIV_APP_DIR/PekoOverlay.apk"
else
    # Clean out any stale copy so we don't ship an old APK silently.
    rm -rf "$PRIV_APP_DIR"
    if [ "$WITH_OVERLAY" = true ]; then
        echo "[x] --with-overlay set but APK not produced at $OVERLAY_APK"
        exit 1
    fi
fi

# ─── Pack the zip (Magisk accepts standard zip, META-INF optional) ─
mkdir -p "$OUT_DIR"
rm -f "$ZIP"
(
    cd "$MODULE_DIR"
    zip -rq "$ZIP" . \
        -x "*.DS_Store" \
        -x "*.pyc" \
        -x "__pycache__/*"
)

SIZE=$(du -h "$ZIP" | cut -f1)
echo "[+] Built $ZIP ($SIZE)"

# ─── Optional push + install ──────────────────────────────────────
if [ "$DO_INSTALL" = true ]; then
    ADB="${ADB:-adb}"
    "$ADB" devices | grep -q device$ || { echo "[x] no adb device"; exit 1; }
    echo "[+] Pushing to /sdcard/Download/"
    "$ADB" push "$ZIP" /sdcard/Download/
    echo ""
    echo "On the phone:"
    echo "  1. Open Magisk app"
    echo "  2. Modules → Install from storage"
    echo "  3. Pick $(basename "$ZIP")"
    echo "  4. Reboot"
    echo ""
    echo "After reboot, verify with:"
    echo "  adb shell su -c 'ps -A | grep peko-agent'"
    echo "  adb forward tcp:8080 tcp:8080 && open http://localhost:8080"
fi
