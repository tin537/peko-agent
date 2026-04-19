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
WITH_SMS_SHIM=false
for a in "$@"; do
    case "$a" in
        --skip-build)    SKIP_BUILD=true ;;
        --install)       DO_INSTALL=true ;;
        --with-overlay)  WITH_OVERLAY=true ;;
        --with-sms-shim) WITH_SMS_SHIM=true ;;
        *) echo "unknown arg: $a"; exit 1 ;;
    esac
done

# Factored out because we now build two APKs the same way. Takes a
# project dir + the display name, expects the output APK to land at
# app/build/outputs/apk/release/app-release-unsigned.apk.
build_gradle_apk() {
    local dir="$1"
    local label="$2"
    echo "[+] Building $label APK (Gradle, release) in $dir"
    (
        cd "$dir"
        if [ -x ./gradlew ]; then
            ./gradlew :app:assembleRelease
        elif command -v gradle >/dev/null 2>&1; then
            gradle :app:assembleRelease
        else
            echo "[x] Neither ./gradlew nor 'gradle' found."
            echo "    Install Gradle 8.7+ (brew install gradle) or run 'gradle wrapper'"
            echo "    in $dir to generate the wrapper, then retry."
            exit 1
        fi
    )
}

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
# AGP names the APK "app-release.apk" when a signingConfig is applied,
# "app-release-unsigned.apk" when not. Accept whichever is present —
# signed is preferred for priv-app install (Android 13 PackageManager
# silently rejects unsigned APKs from /system/priv-app).
# Use [ -f ] rather than `ls | head` so set -euo pipefail doesn't
# silently abort the script when only one of the candidate filenames
# exists — pipefail + ls-missing-file bit us on this previously.
OVERLAY_APK_DIR="$OVERLAY_DIR/app/build/outputs/apk/release"
if   [ -f "$OVERLAY_APK_DIR/app-release.apk" ]; then
    OVERLAY_APK="$OVERLAY_APK_DIR/app-release.apk"
elif [ -f "$OVERLAY_APK_DIR/app-release-unsigned.apk" ]; then
    OVERLAY_APK="$OVERLAY_APK_DIR/app-release-unsigned.apk"
else
    OVERLAY_APK=""
fi
OVERLAY_PRIV_DIR="$MODULE_DIR/system/priv-app/PekoOverlay"

if [ "$WITH_OVERLAY" = true ]; then
    build_gradle_apk "$OVERLAY_DIR" "peko-overlay"
fi

if [ -f "$OVERLAY_APK" ]; then
    echo "[+] Including PekoOverlay.apk as priv-app"
    mkdir -p "$OVERLAY_PRIV_DIR"
    install -m 0644 "$OVERLAY_APK" "$OVERLAY_PRIV_DIR/PekoOverlay.apk"
else
    # Clean out any stale copy so we don't ship an old APK silently.
    rm -rf "$OVERLAY_PRIV_DIR"
    if [ "$WITH_OVERLAY" = true ]; then
        echo "[x] --with-overlay set but APK not produced at $OVERLAY_APK"
        exit 1
    fi
fi

# ─── Optional: SMS shim APK as priv-app ───────────────────────────
# This is the privileged app that lets peko-agent send SMS via
# SmsManager.sendTextMessage() without needing access to the carrier's
# AT channel (which RILD owns exclusively on most modern phones).
# The matching privapp-permissions-peko.xml is already tracked inside
# the module tree at system/etc/permissions/ — we just need to stage
# the APK here and everything lines up at boot.
SMS_SHIM_DIR="$REPO_ROOT/android/peko-sms-shim"
SMS_SHIM_APK_DIR="$SMS_SHIM_DIR/app/build/outputs/apk/release"
if   [ -f "$SMS_SHIM_APK_DIR/app-release.apk" ]; then
    SMS_SHIM_APK="$SMS_SHIM_APK_DIR/app-release.apk"
elif [ -f "$SMS_SHIM_APK_DIR/app-release-unsigned.apk" ]; then
    SMS_SHIM_APK="$SMS_SHIM_APK_DIR/app-release-unsigned.apk"
else
    SMS_SHIM_APK=""
fi
SMS_SHIM_PRIV_DIR="$MODULE_DIR/system/priv-app/PekoSmsShim"

if [ "$WITH_SMS_SHIM" = true ]; then
    build_gradle_apk "$SMS_SHIM_DIR" "peko-sms-shim"
fi

if [ -f "$SMS_SHIM_APK" ]; then
    echo "[+] Including PekoSmsShim.apk as priv-app"
    mkdir -p "$SMS_SHIM_PRIV_DIR"
    install -m 0644 "$SMS_SHIM_APK" "$SMS_SHIM_PRIV_DIR/PekoSmsShim.apk"
else
    rm -rf "$SMS_SHIM_PRIV_DIR"
    if [ "$WITH_SMS_SHIM" = true ]; then
        echo "[x] --with-sms-shim set but APK not produced at $SMS_SHIM_APK"
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
