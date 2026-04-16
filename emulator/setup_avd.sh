#!/bin/bash
# setup_avd.sh — Create and configure the Peko test AVD
#
# Auto-detects host architecture (Apple Silicon → arm64, Intel → x86_64)
# and installs the appropriate system image.
#
# Usage:
#   ./emulator/setup_avd.sh                  # Auto-detect arch
#   ./emulator/setup_avd.sh --api 34         # Specific API level
#   ./emulator/setup_avd.sh --arch x86_64    # Force architecture

set -euo pipefail

export JAVA_HOME="/Applications/Android Studio.app/Contents/jbr/Contents/Home"
export ANDROID_HOME=~/Library/Android/sdk
export PATH="$JAVA_HOME/bin:$ANDROID_HOME/cmdline-tools/latest/bin:$ANDROID_HOME/platform-tools:$ANDROID_HOME/emulator:$PATH"

AVD_NAME="peko_test"
API_LEVEL="34"
ARCH=""

GREEN='\033[0;32m'
RED='\033[0;31m'
NC='\033[0m'
log() { echo -e "${GREEN}[+]${NC} $1"; }
err() { echo -e "${RED}[x]${NC} $1"; exit 1; }

# Parse args
while [[ $# -gt 0 ]]; do
    case $1 in
        --api) API_LEVEL="$2"; shift 2 ;;
        --arch) ARCH="$2"; shift 2 ;;
        *) shift ;;
    esac
done

# Auto-detect host architecture
if [ -z "$ARCH" ]; then
    HOST_ARCH=$(uname -m)
    case "$HOST_ARCH" in
        arm64|aarch64) ARCH="arm64-v8a" ;;
        x86_64)        ARCH="x86_64" ;;
        *)             err "Unknown host arch: $HOST_ARCH" ;;
    esac
    log "Host architecture: $HOST_ARCH → emulator ABI: $ARCH"
fi

# Determine system image — prefer google_apis (rootable), NOT playstore
SYS_IMAGE="system-images;android-${API_LEVEL};google_apis;${ARCH}"
log "System image: $SYS_IMAGE"

# Install system image if missing
if ! sdkmanager --list_installed 2>/dev/null | grep -q "android-${API_LEVEL}.*google_apis.*${ARCH}"; then
    log "Installing system image..."
    echo "y" | sdkmanager --install "$SYS_IMAGE" "platform-tools" "platforms;android-${API_LEVEL}" 2>&1 | tail -5
    if [ $? -ne 0 ]; then
        log "google_apis image not available for API $API_LEVEL/$ARCH, trying default..."
        SYS_IMAGE="system-images;android-${API_LEVEL};default;${ARCH}"
        echo "y" | sdkmanager --install "$SYS_IMAGE" "platform-tools" "platforms;android-${API_LEVEL}" 2>&1 | tail -5
    fi
fi

# Delete existing AVD if present
if avdmanager list avd 2>/dev/null | grep -q "$AVD_NAME"; then
    log "Deleting existing AVD '$AVD_NAME'..."
    avdmanager delete avd -n "$AVD_NAME"
fi

# Create AVD
log "Creating AVD '$AVD_NAME' (API $API_LEVEL, $ARCH)..."
echo "no" | avdmanager create avd \
    -n "$AVD_NAME" \
    -k "$SYS_IMAGE" \
    -d "pixel_4" \
    --force

# Configure AVD for agent testing
AVD_DIR="$HOME/.android/avd/${AVD_NAME}.avd"
cat >> "$AVD_DIR/config.ini" << 'EOF'
hw.ramSize=2048
hw.keyboard=yes
hw.lcd.density=420
hw.lcd.width=1080
hw.lcd.height=2340
disk.dataPartition.size=4G
vm.heapSize=256
hw.gpu.enabled=yes
hw.gpu.mode=auto
hw.camera.back=none
hw.camera.front=none
hw.audioInput=no
hw.audioOutput=no
hw.sensors.proximity=no
hw.sensors.magnetic_field=no
hw.sensors.orientation=no
hw.sensors.temperature=no
hw.sensors.light=no
hw.sensors.pressure=no
hw.sensors.humidity=no
EOF

log "AVD '$AVD_NAME' created! (API $API_LEVEL, $ARCH)"
echo ""
echo "Next steps:"
echo "  1. Start emulator:  ./emulator/start.sh"
echo "  2. Deploy & test:   ./emulator/deploy_test.sh"
echo ""
echo "Or manually:"
echo "  emulator -avd $AVD_NAME -writable-system -no-snapshot -gpu host"
