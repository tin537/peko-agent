#!/usr/bin/env bash
# Deploy daemon + test with real GGUF model on the Android emulator.
# Assumes:
#   - `cargo build --release --target aarch64-linux-android` completed
#   - `./build-android.sh` completed (produces build-android-arm64-v8a/peko-llm-daemon)
#   - Qwen3 1.7B GGUF already pushed to /data/local/tmp/peko/models/
set -euo pipefail

export PATH="$HOME/Library/Android/sdk/platform-tools:$PATH"

SERIAL="${ANDROID_SERIAL:-emulator-5554}"
DAEMON_BIN="build-android-arm64-v8a/peko-llm-daemon"
REMOTE_DIR="/data/local/tmp/peko"
MODEL="$REMOTE_DIR/models/qwen3-1.7b-q4_k_m.gguf"
SOCKET="@peko-llm"

cd "$(dirname "$0")"

if [ ! -f "$DAEMON_BIN" ]; then
    echo "error: $DAEMON_BIN not found — run ./build-android.sh first" >&2
    exit 1
fi

echo "→ killing old daemon + agent..."
adb -s "$SERIAL" shell "pkill -9 -f peko-llm-daemon; pkill -9 -f peko-agent" 2>&1 || true
sleep 1

echo "→ pushing daemon binary..."
adb -s "$SERIAL" push "$DAEMON_BIN" "$REMOTE_DIR/peko-llm-daemon"
adb -s "$SERIAL" shell "chmod 755 $REMOTE_DIR/peko-llm-daemon"

echo "→ verifying model exists on device..."
if ! adb -s "$SERIAL" shell "[ -f $MODEL ] && echo ok" 2>&1 | grep -q ok; then
    echo "error: model not found at $MODEL on device" >&2
    echo "push it first:" >&2
    echo "  adb push models/qwen3-1.7b-q4_k_m.gguf $MODEL" >&2
    exit 1
fi

echo "→ starting daemon on $SOCKET..."
adb -s "$SERIAL" shell "cd $REMOTE_DIR && ./peko-llm-daemon --model $MODEL --socket '$SOCKET' --template qwen --context 2048 --max-tokens 256 > $REMOTE_DIR/daemon.log 2>&1 &" &
disown 2>/dev/null
sleep 3

PID=$(adb -s "$SERIAL" shell "pidof peko-llm-daemon" | tr -d '\r\n')
if [ -z "$PID" ]; then
    echo "error: daemon not running — log:"
    adb -s "$SERIAL" shell "cat $REMOTE_DIR/daemon.log"
    exit 1
fi

echo "→ daemon PID: $PID"
echo "→ daemon startup log:"
adb -s "$SERIAL" shell "tail -20 $REMOTE_DIR/daemon.log"

echo ""
echo "→ forwarding tcp:9900 → $SOCKET for host testing..."
adb -s "$SERIAL" forward --remove-all 2>&1 || true
adb -s "$SERIAL" forward tcp:9900 "localabstract:${SOCKET#@}"

echo ""
echo "→ /health:"
curl -s -m 30 http://127.0.0.1:9900/health
echo ""

echo ""
echo "→ /v1/chat/completions streaming test (Hi):"
START=$(date +%s)
curl -s -N -m 180 -X POST http://127.0.0.1:9900/v1/chat/completions \
    -H "Content-Type: application/json" \
    -d '{"model":"qwen","messages":[{"role":"user","content":"Hi"}],"stream":true,"max_tokens":64}' \
    | head -c 3000
END=$(date +%s)
echo ""
echo "→ took $((END-START))s"

echo ""
echo "→ daemon log tail:"
adb -s "$SERIAL" shell "tail -15 $REMOTE_DIR/daemon.log"
