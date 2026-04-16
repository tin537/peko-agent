#!/bin/bash
# start.sh — Start the Peko test emulator
#
# Usage:
#   ./emulator/start.sh              # Normal start
#   ./emulator/start.sh --headless   # No GUI (CI/server)

set -euo pipefail

export JAVA_HOME="/Applications/Android Studio.app/Contents/jbr/Contents/Home"
export ANDROID_HOME=~/Library/Android/sdk
export PATH="$JAVA_HOME/bin:$ANDROID_HOME/cmdline-tools/latest/bin:$ANDROID_HOME/platform-tools:$ANDROID_HOME/emulator:$PATH"

AVD_NAME="peko_test"
HEADLESS=false

for arg in "$@"; do
    [ "$arg" = "--headless" ] && HEADLESS=true
done

echo "[+] Starting emulator '$AVD_NAME'..."

EMULATOR_ARGS=(
    -avd "$AVD_NAME"
    -writable-system
    -no-snapshot
    -gpu host
    -memory 2048
    -partition-size 4096
)

if [ "$HEADLESS" = true ]; then
    EMULATOR_ARGS+=(-no-window -no-audio -no-boot-anim)
fi

# Start emulator in background
emulator "${EMULATOR_ARGS[@]}" &
EMU_PID=$!
echo "[+] Emulator PID: $EMU_PID"

# Wait for boot
echo "[+] Waiting for device to boot..."
adb wait-for-device

TIMEOUT=120
for i in $(seq 1 $TIMEOUT); do
    BOOT=$(adb shell getprop sys.boot_completed 2>/dev/null | tr -d '\r')
    if [ "$BOOT" = "1" ]; then
        echo "[+] Device booted in ${i}s"
        break
    fi
    if [ $i -eq $TIMEOUT ]; then
        echo "[x] Boot timeout after ${TIMEOUT}s"
        exit 1
    fi
    sleep 1
done

# Remount system as writable
echo "[+] Remounting system as writable..."
adb root
sleep 2
adb remount 2>/dev/null || adb shell "mount -o remount,rw /system" 2>/dev/null || true

echo ""
echo "[+] Emulator ready!"
echo "    ADB:    adb shell"
echo "    Deploy: ./emulator/deploy_test.sh"
echo "    Stop:   kill $EMU_PID"
