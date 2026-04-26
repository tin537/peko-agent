#!/usr/bin/env bash
# Install tesseract OCR on the Android device. The recommended path
# bundles tesseract directly into the Magisk module via:
#
#   1. ./scripts/build-tesseract-android.sh   (one-time, ~10 min)
#         cross-compiles libpng + leptonica + cpu_features + tesseract
#         for arm64-android via the NDK, drops outputs into
#         magisk/peko-module/system/{bin,etc/tessdata}/
#
#   2. ./magisk/build-module.sh               (rebuild the .zip)
#
#   3. Flash the new module via Magisk Manager → reboot
#
# After reboot the tesseract binary lives at /system/bin/tesseract
# and the agent's `ocr` tool finds it automatically. The Magisk
# customize.sh sets perms; service.sh exports TESSDATA_PREFIX.
#
# This script is for ad-hoc / pre-Magisk-flash testing — push the
# already-built artifacts straight to /data/peko/bin/ on a live
# device so the agent can use OCR before the next reflash.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
ADB=${ADB:-adb}

step() { printf "\n\033[1;34m▶ %s\033[0m\n" "$*"; }
ok()   { printf "  \033[1;32m✓\033[0m %s\n" "$*"; }
warn() { printf "  \033[1;33m!\033[0m %s\n" "$*"; }

step "Preflight"
"$ADB" get-state >/dev/null 2>&1 || { echo "no adb device"; exit 1; }

BUNDLED_BIN="$REPO_ROOT/magisk/peko-module/system/bin/tesseract"
BUNDLED_TESSDATA="$REPO_ROOT/magisk/peko-module/system/etc/tessdata"

if [[ ! -x "$BUNDLED_BIN" ]] || [[ ! -d "$BUNDLED_TESSDATA" ]]; then
    warn "no bundled tesseract found at $BUNDLED_BIN"
    warn "build it first:  ./scripts/build-tesseract-android.sh"
    exit 1
fi

step "Probe existing tesseract on device"
EXISTING=$("$ADB" shell 'su -c "
  for p in /data/peko/bin/tesseract /data/local/tmp/tesseract \
           /system/bin/tesseract; do
    if [ -x \"\$p\" ]; then echo \$p; exit 0; fi
  done
  command -v tesseract 2>/dev/null
"' 2>&1 | tr -d '\r')
[[ -n "$EXISTING" ]] && ok "found existing: $EXISTING"

step "Push bundled tesseract → /data/peko/bin/"
"$ADB" push "$BUNDLED_BIN" /data/local/tmp/tesseract >/dev/null
"$ADB" push "$BUNDLED_TESSDATA" /data/local/tmp/ >/dev/null
"$ADB" shell "su -c '
  mkdir -p /data/peko/bin /data/peko/tessdata
  cp /data/local/tmp/tesseract /data/peko/bin/tesseract
  chmod 0755 /data/peko/bin/tesseract
  cp -r /data/local/tmp/tessdata/* /data/peko/tessdata/
  chmod 0644 /data/peko/tessdata/*.traineddata
'"
ok "pushed binary + tessdata"

step "Smoke test"
VER=$("$ADB" shell "su -c 'TESSDATA_PREFIX=/data/peko/tessdata /data/peko/bin/tesseract --version 2>&1 | head -1'" 2>&1 | tr -d '\r')
ok "$VER"

LANGS=$("$ADB" shell "su -c 'TESSDATA_PREFIX=/data/peko/tessdata /data/peko/bin/tesseract --list-langs 2>&1 | tail -n +2'" 2>&1 | tr -d '\r' | tr '\n' ' ')
ok "languages available: $LANGS"

step "End-to-end OCR test"
"$ADB" shell 'su -c "input keyevent KEYCODE_WAKEUP && sleep 1 && screencap -p /data/local/tmp/peko-ocr-test.png"'
TEXT=$("$ADB" shell "su -c 'TESSDATA_PREFIX=/data/peko/tessdata /data/peko/bin/tesseract /data/local/tmp/peko-ocr-test.png stdout -l eng --psm 6 2>/dev/null | head -10'" 2>&1)
if [[ -n "$TEXT" ]]; then
    ok "OCR returned text:"
    echo "$TEXT" | sed 's/^/    /'
else
    warn "OCR returned empty result (screen might be dark/locked)"
fi

step "Done"
echo "  Tesseract is now usable by the agent's `ocr` tool."
echo "  For a permanent install: rebuild + flash the Magisk module."
