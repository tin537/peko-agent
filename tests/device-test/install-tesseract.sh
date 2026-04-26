#!/usr/bin/env bash
# Install tesseract OCR on the Android device for use by the agent's
# `ocr` tool. Two paths supported:
#
#   1. (recommended) Use Termux. If Termux is already installed,
#      run `pkg install tesseract tesseract-data-eng` inside Termux,
#      then symlink the binary so the agent finds it without
#      knowing the Termux internal path.
#
#   2. (manual) Download a prebuilt static arm64 tesseract binary
#      and tessdata, push to /data/peko/bin/. This script doesn't
#      automate the download because no single canonical static
#      build exists; we point the user at the right place instead.

set -uo pipefail
ADB=${ADB:-adb}

step() { printf "\n\033[1;34m▶ %s\033[0m\n" "$*"; }
ok()   { printf "  \033[1;32m✓\033[0m %s\n" "$*"; }
warn() { printf "  \033[1;33m!\033[0m %s\n" "$*"; }

step "Preflight"
"$ADB" get-state >/dev/null 2>&1 || { echo "no adb device"; exit 1; }

step "Probe existing tesseract"
EXISTING=$("$ADB" shell 'su -c "
  for p in /data/peko/bin/tesseract /data/local/tmp/tesseract \
           /system/bin/tesseract /data/data/com.termux/files/usr/bin/tesseract; do
    if [ -x \"\$p\" ]; then echo \$p; exit 0; fi
  done
  command -v tesseract 2>/dev/null
"' 2>&1 | tr -d '\r')

if [[ -n "$EXISTING" ]]; then
    ok "tesseract already installed at: $EXISTING"
    "$ADB" shell "su -c '$EXISTING --version 2>&1'" | head -3 | sed 's/^/    /'
    exit 0
fi

step "No tesseract found. Recommended install paths"
cat <<'EOF'
  Option 1 — Termux (easiest):
    1. Install Termux from F-Droid: https://f-droid.org/packages/com.termux/
    2. Open Termux, run:
         pkg update
         pkg install tesseract tesseract-data-eng tesseract-data-tha
    3. From your Mac, symlink the Termux binary so the agent can find it:
         adb shell 'su -c "
           mkdir -p /data/peko/bin
           ln -sf /data/data/com.termux/files/usr/bin/tesseract /data/peko/bin/tesseract
           chmod 755 /data/peko/bin/tesseract
         "'

  Option 2 — Magisk module bundle (if you don't want Termux):
    Pull a static arm64 tesseract binary + tessdata onto the
    device. Recommended source (community arm64 build):
      https://github.com/Madeesh-Kannan/aarch64-android-tesseract
    Then:
      adb push tesseract /data/local/tmp/tesseract
      adb push tessdata /data/local/tmp/tessdata
      adb shell 'su -c "
        mkdir -p /data/peko/bin
        cp /data/local/tmp/tesseract /data/peko/bin/tesseract
        chmod 755 /data/peko/bin/tesseract
        mkdir -p /data/peko/tessdata
        cp -r /data/local/tmp/tessdata/* /data/peko/tessdata/
        echo \"export TESSDATA_PREFIX=/data/peko/tessdata\" >> /data/adb/modules/peko_agent/service.sh
      "'

  After either path, verify with:
    adb shell 'su -c "/data/peko/bin/tesseract --version"'
EOF
