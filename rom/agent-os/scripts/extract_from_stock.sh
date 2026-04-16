#!/bin/bash
# extract_from_stock.sh — Extract minimal required files from a stock Android ROM
#
# Usage:
#   ./extract_from_stock.sh /path/to/stock-system.img
#   ./extract_from_stock.sh /path/to/mounted/system
#
# Extracts only what peko-agent needs:
#   - libc, libm, libdl, liblog (dynamic libs)
#   - linker64 (dynamic linker)
#   - sh, toybox (shell + coreutils)
#   - screencap (optional, for screenshots)
#   - ueventd, logd, netd (essential daemons)
#   - wpa_supplicant (WiFi)

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
ROM_DIR="$(dirname "$SCRIPT_DIR")"
OUT_DIR="$ROM_DIR/system"

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m'

log()  { echo -e "${GREEN}[+]${NC} $1"; }
warn() { echo -e "${YELLOW}[!]${NC} $1"; }
err()  { echo -e "${RED}[x]${NC} $1"; exit 1; }

if [ $# -lt 1 ]; then
    echo "Usage: $0 <stock-system-image-or-mount-point>"
    exit 1
fi

SOURCE="$1"
MOUNT_POINT=""
NEEDS_UNMOUNT=false

# If it's an image file, mount it
if [ -f "$SOURCE" ]; then
    MOUNT_POINT=$(mktemp -d)
    log "Mounting $SOURCE at $MOUNT_POINT..."

    # Try simg2img first (sparse image)
    if command -v simg2img &>/dev/null; then
        RAW_IMG=$(mktemp)
        simg2img "$SOURCE" "$RAW_IMG" 2>/dev/null || cp "$SOURCE" "$RAW_IMG"
        SOURCE="$RAW_IMG"
    fi

    if [[ "$(uname)" == "Linux" ]]; then
        sudo mount -o ro,loop "$SOURCE" "$MOUNT_POINT"
    else
        # macOS — needs ext4fuse or similar
        if command -v ext4fuse &>/dev/null; then
            ext4fuse "$SOURCE" "$MOUNT_POINT" -o allow_other
        else
            err "On macOS, install ext4fuse: brew install ext4fuse"
        fi
    fi
    NEEDS_UNMOUNT=true
elif [ -d "$SOURCE" ]; then
    MOUNT_POINT="$SOURCE"
else
    err "Source not found: $SOURCE"
fi

# ─── Extract files ────────────────────────────────────────

log "Extracting minimal system files..."

mkdir -p "$OUT_DIR/bin" "$OUT_DIR/lib64" "$OUT_DIR/etc/peko"

# Essential binaries
BINS=(
    "bin/sh"
    "bin/toybox"
    "bin/linker64"
    "bin/ueventd"
    "bin/logd"
    "bin/logcat"
)

# Optional binaries
OPT_BINS=(
    "bin/netd"
    "bin/screencap"
    "bin/ip"
    "bin/ping"
    "bin/wpa_supplicant"
)

# Essential libraries
LIBS=(
    "lib64/libc.so"
    "lib64/libm.so"
    "lib64/libdl.so"
    "lib64/liblog.so"
    "lib64/libcutils.so"
    "lib64/libutils.so"
    "lib64/libbase.so"
    "lib64/libz.so"
    "lib64/libcrypto.so"
    "lib64/libssl.so"
    "lib64/ld-android.so"
)

# Optional libraries (for screencap, netd)
OPT_LIBS=(
    "lib64/libui.so"
    "lib64/libgui.so"
    "lib64/libbinder.so"
    "lib64/libEGL.so"
    "lib64/libGLESv2.so"
    "lib64/libnetd_client.so"
    "lib64/libnl.so"
)

extract_file() {
    local src="$MOUNT_POINT/$1"
    local dst_dir="$OUT_DIR/$(dirname "$1")"
    local name="$(basename "$1")"

    if [ -f "$src" ]; then
        mkdir -p "$dst_dir"
        cp "$src" "$dst_dir/$name"
        log "  $1"
        return 0
    fi
    return 1
}

echo ""
log "Essential binaries:"
for bin in "${BINS[@]}"; do
    extract_file "$bin" || err "Missing required: $bin"
done

echo ""
log "Optional binaries:"
for bin in "${OPT_BINS[@]}"; do
    extract_file "$bin" || warn "  Missing optional: $bin"
done

echo ""
log "Essential libraries:"
for lib in "${LIBS[@]}"; do
    extract_file "$lib" || warn "  Missing: $lib (may be in APEX)"
done

echo ""
log "Optional libraries:"
for lib in "${OPT_LIBS[@]}"; do
    extract_file "$lib" 2>/dev/null || true
done

# ─── Create symlinks (toybox provides most coreutils) ─────

log "Creating toybox symlinks..."
TOYBOX_CMDS="cat chmod chown cp date df du echo env find grep head id ifconfig
             kill ln ls mkdir mount mv ping ps rm rmdir sed sleep sort stat
             tail tar touch umount uname wc which"

for cmd in $TOYBOX_CMDS; do
    ln -sf toybox "$OUT_DIR/bin/$cmd" 2>/dev/null || true
done

# ─── Cleanup ──────────────────────────────────────────────

if [ "$NEEDS_UNMOUNT" = true ]; then
    log "Unmounting..."
    if [[ "$(uname)" == "Linux" ]]; then
        sudo umount "$MOUNT_POINT"
    else
        umount "$MOUNT_POINT" 2>/dev/null || true
    fi
    rmdir "$MOUNT_POINT" 2>/dev/null || true
fi

# ─── Stats ────────────────────────────────────────────────

echo ""
TOTAL_SIZE=$(du -sh "$OUT_DIR" | cut -f1)
FILE_COUNT=$(find "$OUT_DIR" -type f | wc -l)
log "Extracted $FILE_COUNT files, total size: $TOTAL_SIZE"
log "Output: $OUT_DIR"
echo ""
log "Next: run build_rom.sh to create flashable images"
