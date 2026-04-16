#!/bin/bash
# build_rom.sh — Build Peko Agent-OS ROM images
#
# Usage:
#   ./build_rom.sh                          # Build all images
#   ./build_rom.sh --device pixel4a         # Build for specific device
#   ./build_rom.sh --skip-build             # Use existing peko-agent binary
#
# Produces:
#   out/boot.img      — Kernel + ramdisk
#   out/system.img    — System partition (peko-agent + minimal tools)
#   out/flash_all.sh  — Flash script
#
# Prerequisites:
#   1. Device kernel at prebuilt/kernel (or prebuilt/Image.gz)
#   2. Stock system files extracted (run extract_from_stock.sh first)
#   3. Rust toolchain with aarch64-linux-android target

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
ROM_DIR="$(dirname "$SCRIPT_DIR")"
PROJECT_DIR="$(dirname "$(dirname "$ROM_DIR")")"
OUT_DIR="$ROM_DIR/out"

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
CYAN='\033[0;36m'
NC='\033[0m'

log()     { echo -e "${GREEN}[+]${NC} $1"; }
warn()    { echo -e "${YELLOW}[!]${NC} $1"; }
err()     { echo -e "${RED}[x]${NC} $1"; exit 1; }
section() { echo -e "\n${CYAN}═══ $1 ═══${NC}"; }

SKIP_BUILD=false
DEVICE="generic"
for arg in "$@"; do
    case $arg in
        --skip-build) SKIP_BUILD=true ;;
        --device=*) DEVICE="${arg#*=}" ;;
        --device) shift; DEVICE="${1:-generic}" ;;
    esac
done

mkdir -p "$OUT_DIR"

# ═══════════════════════════════════════════════════════════
section "1. Build peko-agent"
# ═══════════════════════════════════════════════════════════

PEKO_BIN="$PROJECT_DIR/target/aarch64-linux-android/release/peko-agent"

if [ "$SKIP_BUILD" = false ]; then
    log "Building peko-agent for aarch64-linux-android..."
    cd "$PROJECT_DIR"

    if ! rustup target list --installed | grep -q aarch64-linux-android; then
        log "Adding Android target..."
        rustup target add aarch64-linux-android
    fi

    cargo build --target aarch64-linux-android --release 2>&1 | tail -3
    cd "$ROM_DIR"
fi

if [ ! -f "$PEKO_BIN" ]; then
    warn "peko-agent binary not found at $PEKO_BIN"
    warn "Using placeholder — set up NDK and rebuild, or use --skip-build with existing binary"
    # Create a placeholder script for testing the ROM structure
    mkdir -p "$(dirname "$PEKO_BIN")"
    echo '#!/system/bin/sh' > "$PEKO_BIN"
    echo 'echo "placeholder — build with NDK for real binary"' >> "$PEKO_BIN"
    chmod +x "$PEKO_BIN"
fi

BIN_SIZE=$(du -h "$PEKO_BIN" | cut -f1)
log "Binary: $PEKO_BIN ($BIN_SIZE)"

# ═══════════════════════════════════════════════════════════
section "2. Build system image"
# ═══════════════════════════════════════════════════════════

SYSTEM_DIR="$OUT_DIR/system_root"
rm -rf "$SYSTEM_DIR"
mkdir -p "$SYSTEM_DIR/system/bin"
mkdir -p "$SYSTEM_DIR/system/lib64"
mkdir -p "$SYSTEM_DIR/system/etc/init"
mkdir -p "$SYSTEM_DIR/system/etc/peko"
mkdir -p "$SYSTEM_DIR/system/etc/selinux"

# Copy peko-agent binary
cp "$PEKO_BIN" "$SYSTEM_DIR/system/bin/peko-agent"
chmod 755 "$SYSTEM_DIR/system/bin/peko-agent"

# Copy init scripts
cp "$ROM_DIR/ramdisk/init.peko.rc" "$SYSTEM_DIR/system/etc/init/peko-agent.rc"

# Copy extracted system files (binaries + libs)
if [ -d "$ROM_DIR/system/bin" ]; then
    log "Copying extracted system binaries..."
    cp -a "$ROM_DIR/system/bin/"* "$SYSTEM_DIR/system/bin/" 2>/dev/null || true
fi
if [ -d "$ROM_DIR/system/lib64" ]; then
    log "Copying extracted system libraries..."
    cp -a "$ROM_DIR/system/lib64/"* "$SYSTEM_DIR/system/lib64/" 2>/dev/null || true
fi

# Copy default config
cp "$PROJECT_DIR/config/config.example.toml" "$SYSTEM_DIR/system/etc/peko/config.toml"

# Copy SELinux policy files
cp "$ROM_DIR/../sepolicy/"* "$SYSTEM_DIR/system/etc/selinux/" 2>/dev/null || true

# Calculate size
SYSTEM_SIZE=$(du -sm "$SYSTEM_DIR" | cut -f1)
log "System directory: ${SYSTEM_SIZE}MB"

# Build ext4 image
SYSTEM_IMG="$OUT_DIR/system.img"
IMAGE_SIZE_MB=$((SYSTEM_SIZE + 20)) # 20MB headroom

if command -v make_ext4fs &>/dev/null; then
    log "Building system.img with make_ext4fs..."
    make_ext4fs -l "${IMAGE_SIZE_MB}M" -a system "$SYSTEM_IMG" "$SYSTEM_DIR/system"
elif command -v mke2fs &>/dev/null; then
    log "Building system.img with mke2fs..."
    dd if=/dev/zero of="$SYSTEM_IMG" bs=1M count="$IMAGE_SIZE_MB" 2>/dev/null
    mke2fs -t ext4 -d "$SYSTEM_DIR/system" -L system "$SYSTEM_IMG" 2>/dev/null
    # Resize to minimum
    resize2fs -M "$SYSTEM_IMG" 2>/dev/null || true
else
    warn "No ext4 tool found — creating tar archive instead"
    tar -cf "$OUT_DIR/system.tar" -C "$SYSTEM_DIR" system
    log "Created system.tar (flash manually)"
fi

[ -f "$SYSTEM_IMG" ] && log "system.img: $(du -h "$SYSTEM_IMG" | cut -f1)"

# ═══════════════════════════════════════════════════════════
section "3. Build ramdisk"
# ═══════════════════════════════════════════════════════════

RAMDISK_DIR="$OUT_DIR/ramdisk_root"
rm -rf "$RAMDISK_DIR"
mkdir -p "$RAMDISK_DIR"

# Copy ramdisk contents
cp "$ROM_DIR/ramdisk/init.rc" "$RAMDISK_DIR/"
cp "$ROM_DIR/ramdisk/init.peko.rc" "$RAMDISK_DIR/"
cp "$ROM_DIR/ramdisk/default.prop" "$RAMDISK_DIR/"

# Create required directories
mkdir -p "$RAMDISK_DIR"/{dev,proc,sys,system,data,vendor,mnt,tmp}
mkdir -p "$RAMDISK_DIR/dev/socket"
mkdir -p "$RAMDISK_DIR/sbin"

# init binary (needs to be extracted from stock boot.img or AOSP)
if [ -f "$ROM_DIR/prebuilt/init" ]; then
    cp "$ROM_DIR/prebuilt/init" "$RAMDISK_DIR/init"
    chmod 750 "$RAMDISK_DIR/init"
else
    warn "No init binary at prebuilt/init — ramdisk incomplete"
    warn "Extract init from stock boot.img: unpackbootimg + cpio extract"
fi

# SELinux policy (compiled sepolicy)
if [ -f "$ROM_DIR/prebuilt/sepolicy" ]; then
    cp "$ROM_DIR/prebuilt/sepolicy" "$RAMDISK_DIR/sepolicy"
else
    warn "No compiled sepolicy at prebuilt/sepolicy"
    warn "Compile with: checkpolicy or secilc from AOSP"
fi

# Build cpio archive
RAMDISK_IMG="$OUT_DIR/ramdisk.img"
cd "$RAMDISK_DIR"
find . | cpio -o -H newc 2>/dev/null | gzip > "$RAMDISK_IMG"
cd "$ROM_DIR"

log "ramdisk.img: $(du -h "$RAMDISK_IMG" | cut -f1)"

# ═══════════════════════════════════════════════════════════
section "4. Build boot image"
# ═══════════════════════════════════════════════════════════

KERNEL="$ROM_DIR/prebuilt/kernel"
BOOT_IMG="$OUT_DIR/boot.img"

if [ ! -f "$KERNEL" ]; then
    # Try alternative names
    for name in Image.gz Image zImage vmlinux; do
        if [ -f "$ROM_DIR/prebuilt/$name" ]; then
            KERNEL="$ROM_DIR/prebuilt/$name"
            break
        fi
    done
fi

if [ -f "$KERNEL" ]; then
    if command -v mkbootimg &>/dev/null; then
        log "Building boot.img with mkbootimg..."
        mkbootimg \
            --kernel "$KERNEL" \
            --ramdisk "$RAMDISK_IMG" \
            --cmdline "console=ttyMSM0,115200n8 androidboot.console=ttyMSM0 androidboot.selinux=permissive" \
            --base 0x00000000 \
            --kernel_offset 0x00008000 \
            --ramdisk_offset 0x01000000 \
            --os_version 14.0.0 \
            --os_patch_level 2024-01 \
            --output "$BOOT_IMG"
        log "boot.img: $(du -h "$BOOT_IMG" | cut -f1)"
    else
        warn "mkbootimg not found — install from AOSP tools or Android SDK"
        warn "Ramdisk built at $RAMDISK_IMG — combine with kernel manually"
    fi
else
    warn "No kernel found at prebuilt/kernel"
    warn "Place your device's kernel there and re-run"
fi

# ═══════════════════════════════════════════════════════════
section "5. Generate flash script"
# ═══════════════════════════════════════════════════════════

cat > "$OUT_DIR/flash.sh" << 'FLASH_EOF'
#!/bin/bash
# flash.sh — Flash Peko Agent-OS to device
#
# WARNING: This will ERASE your current Android installation!
#          Back up your data first.

set -euo pipefail

DIR="$(cd "$(dirname "$0")" && pwd)"

echo "=== Peko Agent-OS Flasher ==="
echo ""
echo "WARNING: This will replace your Android system."
echo "The device will boot directly into the Peko agent."
echo ""
read -p "Continue? (y/N) " -n 1 -r
echo ""
[[ $REPLY =~ ^[Yy]$ ]] || exit 1

# Reboot to bootloader
echo "[+] Rebooting to bootloader..."
adb reboot bootloader
sleep 5

# Flash boot image
if [ -f "$DIR/boot.img" ]; then
    echo "[+] Flashing boot.img..."
    fastboot flash boot "$DIR/boot.img"
fi

# Flash system image
if [ -f "$DIR/system.img" ]; then
    echo "[+] Flashing system.img..."
    fastboot flash system "$DIR/system.img"
fi

# Wipe data (optional — preserves agent config if skip)
read -p "Wipe data partition? (y/N) " -n 1 -r
echo ""
if [[ $REPLY =~ ^[Yy]$ ]]; then
    echo "[+] Wiping data..."
    fastboot erase userdata
fi

echo "[+] Rebooting..."
fastboot reboot

echo ""
echo "=== Flash complete ==="
echo "Device will boot into Peko Agent-OS."
echo "Web UI: http://<device-ip>:8080"
echo "ADB:    adb shell"
FLASH_EOF

chmod +x "$OUT_DIR/flash.sh"

# ═══════════════════════════════════════════════════════════
section "Build Summary"
# ═══════════════════════════════════════════════════════════

echo ""
echo "Output directory: $OUT_DIR"
echo ""
ls -lh "$OUT_DIR/" 2>/dev/null | grep -v "^total" | grep -v "_root"
echo ""

TOTAL=$(du -sh "$OUT_DIR" --exclude="*_root" 2>/dev/null | cut -f1 || du -sh "$OUT_DIR" | cut -f1)
log "Total ROM size: $TOTAL"
echo ""
log "Flash with: $OUT_DIR/flash.sh"
echo ""

echo "Missing components (if any):"
[ ! -f "$ROM_DIR/prebuilt/kernel" ] && warn "  kernel — place at prebuilt/kernel"
[ ! -f "$ROM_DIR/prebuilt/init" ]   && warn "  init binary — extract from stock boot.img"
[ ! -f "$ROM_DIR/prebuilt/sepolicy" ] && warn "  sepolicy — compile from rom/sepolicy/"
[ ! -d "$ROM_DIR/system/lib64" ]    && warn "  system libs — run extract_from_stock.sh"
echo ""
