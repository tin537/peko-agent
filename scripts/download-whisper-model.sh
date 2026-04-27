#!/bin/bash
# Phase 25 — fetch a whisper.cpp model and push it to the OnePlus 6T (or
# any rooted Android device peko-agent runs on).
#
# Usage:
#   ./scripts/download-whisper-model.sh                 # default: ggml-base.bin (~150 MB, multilingual)
#   ./scripts/download-whisper-model.sh small           # ggml-small.bin (~488 MB, better Thai)
#   ./scripts/download-whisper-model.sh tiny.en         # ggml-tiny.en.bin (~75 MB, English only)
#
# Models live on Hugging Face under ggerganov/whisper.cpp. Once pushed,
# the `stt` tool finds the file at /data/peko/models/whisper.bin and
# loads it lazily on first call.

set -euo pipefail

VARIANT="${1:-base}"
MODEL_NAME="ggml-${VARIANT}.bin"
MODEL_URL="https://huggingface.co/ggerganov/whisper.cpp/resolve/main/${MODEL_NAME}"
DEVICE_PATH="/data/peko/models/whisper.bin"
LOCAL_DIR="$(cd "$(dirname "$0")/.." && pwd)/models"
LOCAL_FILE="${LOCAL_DIR}/${MODEL_NAME}"

GREEN='\033[0;32m'; YELLOW='\033[1;33m'; RED='\033[0;31m'; NC='\033[0m'
log()  { echo -e "${GREEN}[+]${NC} $1"; }
warn() { echo -e "${YELLOW}[!]${NC} $1"; }
err()  { echo -e "${RED}[x]${NC} $1"; exit 1; }

mkdir -p "$LOCAL_DIR"

if [ -f "$LOCAL_FILE" ]; then
    log "Already downloaded: $LOCAL_FILE ($(du -h "$LOCAL_FILE" | cut -f1))"
else
    log "Downloading $MODEL_NAME from Hugging Face..."
    log "(this is a one-time ~150 MB / ~488 MB / ~1.5 GB depending on variant)"
    curl -L --fail --progress-bar -o "$LOCAL_FILE.tmp" "$MODEL_URL"
    mv "$LOCAL_FILE.tmp" "$LOCAL_FILE"
    log "Saved to $LOCAL_FILE ($(du -h "$LOCAL_FILE" | cut -f1))"
fi

if ! command -v adb >/dev/null; then
    warn "adb not on PATH; skipping device push. Push manually with:"
    warn "  adb push '$LOCAL_FILE' /data/local/tmp/whisper.bin"
    warn "  adb shell su -c 'mkdir -p /data/peko/models; mv /data/local/tmp/whisper.bin $DEVICE_PATH'"
    exit 0
fi

if ! adb devices | grep -q "device$"; then
    warn "no Android device connected; skipping push."
    exit 0
fi

log "Pushing to device..."
adb push "$LOCAL_FILE" /data/local/tmp/whisper.bin >/dev/null
adb shell "su -c 'mkdir -p /data/peko/models && mv /data/local/tmp/whisper.bin ${DEVICE_PATH} && chmod 644 ${DEVICE_PATH}'"
SIZE_ON_DEVICE=$(adb shell "su -c 'stat -c %s ${DEVICE_PATH}'" 2>/dev/null | tr -d '\r')
log "Installed at ${DEVICE_PATH} (${SIZE_ON_DEVICE} bytes)"

cat <<HINT

Test it from the agent (e.g. via Telegram):

  audio_pcm record { duration_ms: 4000 }
  → returns wav_path

  stt transcribe { wav_path: "/data/peko/audio/<...>.wav" }
  → { text: "...", language: "..." }

Or talk in Thai — whisper detects the language automatically.
HINT
