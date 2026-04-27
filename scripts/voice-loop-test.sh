#!/usr/bin/env bash
# Phase 25 — voice loop end-to-end smoke test on the connected device.
# Records 4s of mic, transcribes via offline whisper-cli, then plays
# back the transcript as TTS to verify the full record→stt→tts pipeline.
#
# Usage:
#   ./scripts/voice-loop-test.sh                    # default 4s record
#   ./scripts/voice-loop-test.sh 6                  # 6s record
#
# Prerequisites:
#   - Device connected via adb, rooted with Magisk
#   - peko-agent + PekoOverlay AudioBridge already running
#   - whisper-cli at /system/bin or /data/adb/modules/peko_agent/system/bin
#   - whisper model at /data/peko/models/whisper.bin

set -euo pipefail

DURATION="${1:-4}"
DURATION_MS=$((DURATION * 1000))
ID="vl_$(date +%s)"
IN_DIR=/data/data/com.peko.overlay/files/audio/in
OUT_DIR=/data/data/com.peko.overlay/files/audio/out

GREEN='\033[0;32m'; YELLOW='\033[1;33m'; RED='\033[0;31m'; NC='\033[0m'
log()  { echo -e "${GREEN}[+]${NC} $1"; }
warn() { echo -e "${YELLOW}[!]${NC} $1"; }
err()  { echo -e "${RED}[x]${NC} $1"; exit 1; }

command -v adb >/dev/null || err "adb not on PATH"
adb devices | grep -q "device$" || err "no Android device connected"

resolve_bin() {
    for p in /system/bin/whisper-cli \
             /data/adb/modules/peko_agent/system/bin/whisper-cli \
             /data/local/tmp/whisper-cli; do
        if adb shell "su -c 'test -x $p'" >/dev/null 2>&1; then
            echo "$p"; return 0
        fi
    done
    return 1
}

WBIN=$(resolve_bin) || err "whisper-cli not found on device. Run scripts/build + push."
log "whisper-cli at: $WBIN"

# Confirm model present
adb shell "su -c 'test -f /data/peko/models/whisper.bin'" \
    || err "model missing. Run scripts/download-whisper-model.sh"

# Step 1: record
log "step 1: recording ${DURATION}s of mic..."
adb shell "su -c \"
    rm -f $OUT_DIR/${ID}.* $IN_DIR/${ID}.*;
    printf '%s' '{\\\"action\\\":\\\"record\\\",\\\"duration_ms\\\":${DURATION_MS},\\\"sample_rate\\\":16000,\\\"channels\\\":1}' > $IN_DIR/${ID}.json &&
    chmod 644 $IN_DIR/${ID}.json &&
    touch $IN_DIR/${ID}.start
\""
for i in $(seq 1 $((DURATION + 5))); do
    sleep 1
    if adb shell "su -c 'test -f $OUT_DIR/${ID}.done'" >/dev/null 2>&1; then break; fi
done
adb shell "su -c 'test -f $OUT_DIR/${ID}.wav'" || err "record failed; no output WAV"
log "recorded $(adb shell "su -c 'stat -c %s $OUT_DIR/${ID}.wav'" | tr -d '\r') bytes"

# Step 2: transcribe
log "step 2: transcribing via $WBIN..."
TRANSCRIPT=$(adb shell "su -c '$WBIN -m /data/peko/models/whisper.bin -f $OUT_DIR/${ID}.wav -t 4 -l auto -np -nt 2>/dev/null'" | tr -d '\r' | sed -e 's/^[[:space:]]*//' -e 's/[[:space:]]*$//')
if [ -z "$TRANSCRIPT" ]; then
    warn "empty transcript — silence or whisper detected no speech"
    TRANSCRIPT="(silence)"
fi
log "transcript: $TRANSCRIPT"

# Step 3: TTS back to user
log "step 3: speaking transcript back via TTS..."
TTS_ID="${ID}_tts"
TTS_TEXT_ESCAPED=$(echo "$TRANSCRIPT" | sed 's/"/\\\\\\"/g')
adb shell "su -c \"
    printf '%s' '{\\\"action\\\":\\\"tts\\\",\\\"text\\\":\\\"You said: ${TTS_TEXT_ESCAPED}\\\",\\\"lang\\\":\\\"auto\\\"}' > $IN_DIR/${TTS_ID}.json &&
    chmod 644 $IN_DIR/${TTS_ID}.json &&
    touch $IN_DIR/${TTS_ID}.start
\""
for i in 1 2 3 4 5 6 7 8 9 10; do
    sleep 1
    if adb shell "su -c 'test -f $OUT_DIR/${TTS_ID}.done'" >/dev/null 2>&1; then break; fi
done
TTS_OK=$(adb shell "su -c 'cat $OUT_DIR/${TTS_ID}.json 2>/dev/null'" | tr -d '\r')
if echo "$TTS_OK" | grep -q '"ok":true'; then
    log "TTS played: $TTS_OK"
else
    warn "TTS may have failed: $TTS_OK"
fi

echo
log "voice loop verified end-to-end:"
echo "    record (${DURATION}s) → transcribe → TTS reply"
echo "    transcript: \"$TRANSCRIPT\""
