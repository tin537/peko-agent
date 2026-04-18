#!/usr/bin/env bash
# build.sh — LineageOS-for-fajita build driver with peko overlay.
#
# One-time setup (host machine: Ubuntu 22.04 recommended, 200GB free,
# 16GB+ RAM, 32GB swap):
#
#   ./rom/lineage-fajita/build.sh --init
#
# Subsequent builds:
#
#   ./rom/lineage-fajita/build.sh          # full build
#   ./rom/lineage-fajita/build.sh --sync   # repo sync before build
#   ./rom/lineage-fajita/build.sh --clean  # `make clean` first
#
# Outputs: $LINEAGE_ROOT/out/target/product/fajita/lineage-21.0-*-fajita.zip
# Flash with: adb sideload <zip>  (from LOS recovery)

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

# ─── Configurable ───────────────────────────────────────────
LINEAGE_ROOT="${LINEAGE_ROOT:-$HOME/lineage}"
LINEAGE_BRANCH="${LINEAGE_BRANCH:-lineage-21.0}"
DEVICE="fajita"
JOBS="${JOBS:-$(nproc 2>/dev/null || sysctl -n hw.ncpu)}"

# ─── Parse args ─────────────────────────────────────────────
DO_INIT=false ; DO_SYNC=false ; DO_CLEAN=false
for a in "$@"; do
    case "$a" in
        --init)  DO_INIT=true  ;;
        --sync)  DO_SYNC=true  ;;
        --clean) DO_CLEAN=true ;;
        *) echo "unknown: $a"; exit 1 ;;
    esac
done

# ─── Init: repo + local manifest + first sync ──────────────
if [ "$DO_INIT" = true ]; then
    mkdir -p "$LINEAGE_ROOT"
    cd "$LINEAGE_ROOT"
    [ -d .repo ] || repo init -u https://github.com/LineageOS/android.git -b "$LINEAGE_BRANCH"

    # Drop our manifest so peko + TheMuppets vendor tree get synced too
    mkdir -p .repo/local_manifests
    cp "$SCRIPT_DIR/local_manifest.xml" .repo/local_manifests/peko.xml
    # Rewrite the placeholder path so the local peko source is used
    # instead of a remote GitHub fetch. This is what makes `device/peko/common`
    # point at THIS repo, so edits in peko-agent/ reach the AOSP build.
    mkdir -p device/peko
    ln -sfn "$PROJECT_ROOT" device/peko/common

    repo sync -j"$JOBS" --force-sync --no-clone-bundle --no-tags
    echo "[+] init complete. Next: ./build.sh"
    exit 0
fi

# ─── Normal build ──────────────────────────────────────────
cd "$LINEAGE_ROOT"

if [ "$DO_SYNC" = true ]; then
    repo sync -j"$JOBS" --force-sync --no-clone-bundle --no-tags
fi

# Sanity checks
[ -d .repo ]                     || { echo "not initialized — run --init"; exit 1; }
[ -d device/oneplus/fajita ]     || { echo "fajita device tree missing"; exit 1; }
[ -L device/peko/common ] || [ -d device/peko/common ] || { echo "peko overlay missing"; exit 1; }

# Cross-compile peko-agent + peko-llm-daemon for aarch64-linux-android
# and drop the binaries into the prebuilt slot the Android.bp expects.
echo "[+] Building peko-agent (Cargo)"
(cd "$PROJECT_ROOT" && cargo build --target aarch64-linux-android --release)
cp "$PROJECT_ROOT/target/aarch64-linux-android/release/peko-agent" \
   "$PROJECT_ROOT/rom/layout/peko-agent"

echo "[+] Building peko-llm-daemon (CMake)"
(cd "$PROJECT_ROOT/crates/peko-llm-daemon" && ./build-android.sh)
cp "$PROJECT_ROOT/crates/peko-llm-daemon/build-android-arm64-v8a/peko-llm-daemon" \
   "$PROJECT_ROOT/rom/layout/peko-llm-daemon" 2>/dev/null || \
   echo "[!] daemon binary not found — skipping (ROM build will miss it)"

# Append peko overlay to the LOS fajita makefile exactly once
LOS_MK="$LINEAGE_ROOT/device/oneplus/fajita/lineage_fajita.mk"
OVERLAY_LINE='$(call inherit-product, device/peko/common/rom/lineage-fajita/peko_overlay.mk)'
grep -qxF "$OVERLAY_LINE" "$LOS_MK" || echo "$OVERLAY_LINE" >> "$LOS_MK"

source build/envsetup.sh
breakfast lineage_fajita_peko-userdebug || lunch lineage_fajita-userdebug

if [ "$DO_CLEAN" = true ]; then
    mka clean
fi

mka bacon -j"$JOBS"

OUT_DIR="$LINEAGE_ROOT/out/target/product/fajita"
ls -lh "$OUT_DIR"/lineage-*.zip 2>/dev/null || true

cat <<EOF

[+] Build complete.
    ROM zip:    $OUT_DIR/lineage-21.0-*-fajita.zip

Flash (from LineageOS Recovery):
    adb sideload $OUT_DIR/lineage-21.0-*-fajita.zip

First boot will take ~5 min (dex2oat runs against "speed" filter).
Web UI: adb forward tcp:8080 tcp:8080  →  http://localhost:8080
EOF
