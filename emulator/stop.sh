#!/bin/bash
# stop.sh — Stop the Peko test emulator

export ANDROID_HOME=~/Library/Android/sdk
export PATH="$ANDROID_HOME/platform-tools:$ANDROID_HOME/emulator:$PATH"

echo "[+] Stopping peko-agent..."
adb shell "pkill peko-agent" 2>/dev/null || true

echo "[+] Shutting down emulator..."
adb emu kill 2>/dev/null || true

echo "[+] Done"
