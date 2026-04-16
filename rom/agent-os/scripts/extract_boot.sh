#!/bin/bash
# extract_boot.sh — Extract kernel and init from a stock boot.img
#
# Usage:
#   ./extract_boot.sh /path/to/boot.img
#
# Extracts:
#   prebuilt/kernel   — Device kernel
#   prebuilt/init     — Android init binary
#   prebuilt/sepolicy  — Stock SELinux policy (starting point)
#
# Requires: unpackbootimg (from AOSP) or magiskboot

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
ROM_DIR="$(dirname "$SCRIPT_DIR")"
OUT="$ROM_DIR/prebuilt"

log() { echo -e "\033[0;32m[+]\033[0m $1"; }
err() { echo -e "\033[0;31m[x]\033[0m $1"; exit 1; }

[ $# -lt 1 ] && err "Usage: $0 <boot.img>"
BOOT_IMG="$1"
[ ! -f "$BOOT_IMG" ] && err "File not found: $BOOT_IMG"

mkdir -p "$OUT"
WORK=$(mktemp -d)

# ─── Try magiskboot first (most common) ──────────────────

if command -v magiskboot &>/dev/null; then
    log "Using magiskboot..."
    cd "$WORK"
    magiskboot unpack "$BOOT_IMG"

    [ -f kernel ] && cp kernel "$OUT/kernel" && log "Extracted kernel"
    if [ -f ramdisk.cpio ]; then
        mkdir ramdisk_extracted
        cd ramdisk_extracted
        cpio -id < ../ramdisk.cpio 2>/dev/null

        [ -f init ] && cp init "$OUT/init" && log "Extracted init"
        [ -f sepolicy ] && cp sepolicy "$OUT/sepolicy" && log "Extracted sepolicy"

        # Also grab stock fstab if present
        for f in fstab.*; do
            [ -f "$f" ] && cp "$f" "$OUT/$f" && log "Extracted $f"
        done
    fi

# ─── Try unpackbootimg (AOSP tool) ───────────────────────

elif command -v unpackbootimg &>/dev/null; then
    log "Using unpackbootimg..."
    cd "$WORK"
    unpackbootimg -i "$BOOT_IMG" -o .

    KERNEL_FILE=$(ls *-kernel 2>/dev/null | head -1)
    RAMDISK_FILE=$(ls *-ramdisk 2>/dev/null | head -1)

    [ -n "$KERNEL_FILE" ] && cp "$KERNEL_FILE" "$OUT/kernel" && log "Extracted kernel"

    if [ -n "$RAMDISK_FILE" ]; then
        mkdir ramdisk_extracted
        cd ramdisk_extracted
        gunzip -c "../$RAMDISK_FILE" 2>/dev/null | cpio -id 2>/dev/null || \
            lz4 -d "../$RAMDISK_FILE" - 2>/dev/null | cpio -id 2>/dev/null || \
            cpio -id < "../$RAMDISK_FILE" 2>/dev/null

        [ -f init ] && cp init "$OUT/init" && log "Extracted init"
        [ -f sepolicy ] && cp sepolicy "$OUT/sepolicy" && log "Extracted sepolicy"
    fi

# ─── Manual extraction ───────────────────────────────────

else
    log "No boot image tools found. Trying manual extraction..."
    log "Install one of: magiskboot, unpackbootimg"
    log ""
    log "Manual steps:"
    log "  1. adb shell su -c dd if=/dev/block/by-name/boot of=/sdcard/boot.img"
    log "  2. adb pull /sdcard/boot.img"
    log "  3. Use https://github.com/nickhilton/android-boot-image-tools"
    err "Cannot extract automatically"
fi

# ─── Cleanup ──────────────────────────────────────────────

rm -rf "$WORK"

echo ""
log "Extracted files:"
ls -lh "$OUT/" 2>/dev/null
echo ""

[ -f "$OUT/kernel" ] && log "kernel: ready" || echo "  WARNING: kernel not extracted"
[ -f "$OUT/init" ] && log "init: ready" || echo "  WARNING: init not extracted"
[ -f "$OUT/sepolicy" ] && log "sepolicy: ready (stock — will need customization)" || echo "  WARNING: sepolicy not extracted"
echo ""
log "Next: run build_rom.sh"
