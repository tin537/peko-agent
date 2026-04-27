#!/usr/bin/env bash
# Build tesseract for arm64-android via the NDK and stage it into the
# Magisk module's system/bin/ + system/etc/tessdata/ directories.
#
# Why bundle into the Magisk module:
#   - One reflash gives the user a working `ocr` tool, no Termux,
#     no manual `adb push`, no PATH gymnastics.
#   - Updates land naturally with every module rebuild.
#   - The agent's discover_tesseract() already probes /system/bin
#     first, so this Just Works after install + reboot.
#
# Build strategy:
#   - Build leptonica statically with NDK toolchain.
#   - Build tesseract statically against that leptonica.
#   - Strip + copy outputs into magisk/peko-module/system/.
#   - Fetch English + Thai tessdata (the "fast" variant — smaller,
#     accuracy gap is negligible for screen text).
#
# Re-running is idempotent: existing build dirs are reused, only the
# final binaries get re-copied.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
WORK="$REPO_ROOT/build/tesseract-android"
mkdir -p "$WORK"
cd "$WORK"

# ----------------------------------------------------------------------
# NDK + toolchain
# ----------------------------------------------------------------------
NDK="${ANDROID_NDK_HOME:-${ANDROID_NDK_ROOT:-}}"
if [ -z "$NDK" ]; then
    if ls "$HOME/Library/Android/sdk/ndk" >/dev/null 2>&1; then
        NDK="$HOME/Library/Android/sdk/ndk/$(ls -1 "$HOME/Library/Android/sdk/ndk" | sort -V | tail -1)"
    fi
fi
if [ -z "$NDK" ] || [ ! -d "$NDK" ]; then
    echo "ANDROID_NDK_HOME not set and no NDK at ~/Library/Android/sdk/ndk/" >&2
    exit 1
fi
echo "[+] NDK at $NDK"

ABI=arm64-v8a
API=24
TOOLCHAIN_FILE="$NDK/build/cmake/android.toolchain.cmake"

# ----------------------------------------------------------------------
# Dependency: libpng (required for leptonica to read PNG, which is what
# `screencap -p` produces). zlib comes from the NDK sysroot.
# ----------------------------------------------------------------------
PNG_VER=1.6.43
if [ ! -d "libpng-$PNG_VER" ]; then
    echo "[+] Fetching libpng $PNG_VER"
    curl -fsSL "https://download.sourceforge.net/libpng/libpng-$PNG_VER.tar.gz" \
        | tar xz
fi
PNG_DIR="$WORK/libpng-$PNG_VER"
PNG_INSTALL="$WORK/libpng-install"

if [ ! -f "$PNG_INSTALL/lib/libpng16.a" ] && [ ! -f "$PNG_INSTALL/lib/libpng.a" ]; then
    echo "[+] Configuring libpng"
    rm -rf "$PNG_DIR/build"
    cmake -S "$PNG_DIR" -B "$PNG_DIR/build" \
        -DCMAKE_POLICY_VERSION_MINIMUM=3.5 \
        -DCMAKE_TOOLCHAIN_FILE="$TOOLCHAIN_FILE" \
        -DANDROID_ABI="$ABI" \
        -DANDROID_PLATFORM="android-$API" \
        -DCMAKE_BUILD_TYPE=Release \
        -DCMAKE_INSTALL_PREFIX="$PNG_INSTALL" \
        -DPNG_SHARED=OFF \
        -DPNG_STATIC=ON \
        -DPNG_TESTS=OFF \
        -DPNG_TOOLS=OFF
    echo "[+] Building libpng"
    cmake --build "$PNG_DIR/build" --parallel
    cmake --install "$PNG_DIR/build"
fi
echo "[+] libpng installed at $PNG_INSTALL"

# ----------------------------------------------------------------------
# Source: leptonica
# ----------------------------------------------------------------------
LEPT_VER=1.84.1
if [ ! -d "leptonica-$LEPT_VER" ]; then
    echo "[+] Fetching leptonica $LEPT_VER"
    curl -fsSL "https://github.com/DanBloomberg/leptonica/releases/download/$LEPT_VER/leptonica-$LEPT_VER.tar.gz" \
        | tar xz
fi
LEPT_DIR="$WORK/leptonica-$LEPT_VER"
LEPT_INSTALL="$WORK/leptonica-install"

if [ ! -x "$LEPT_INSTALL/lib/libleptonica.a" ] && [ ! -f "$LEPT_INSTALL/lib/libleptonica.a" ]; then
    echo "[+] Configuring leptonica"
    rm -rf "$LEPT_DIR/build"
    # CMAKE_POLICY_VERSION_MINIMUM is needed because leptonica 1.84.x +
    # tesseract 5.x have CMakeLists with `cmake_minimum_required(VERSION
    # 3.0)` which CMake 4.x dropped support for. Tells CMake to apply
    # 3.5 compatibility shims rather than refuse outright.
    # PNG + zlib are required so leptonica can read screencap output.
    # zlib comes from the NDK sysroot (-lz). PNG comes from the libpng
    # we built above. Other format libs stay disabled — JPEG/TIFF/WebP
    # aren't what `screencap -p` produces, and adding them grows the
    # binary by ~1 MB each.
    rm -rf "$LEPT_DIR/build"
    cmake -S "$LEPT_DIR" -B "$LEPT_DIR/build" \
        -DCMAKE_POLICY_VERSION_MINIMUM=3.5 \
        -DCMAKE_TOOLCHAIN_FILE="$TOOLCHAIN_FILE" \
        -DANDROID_ABI="$ABI" \
        -DANDROID_PLATFORM="android-$API" \
        -DCMAKE_BUILD_TYPE=Release \
        -DCMAKE_INSTALL_PREFIX="$LEPT_INSTALL" \
        -DCMAKE_PREFIX_PATH="$PNG_INSTALL" \
        -DCMAKE_FIND_ROOT_PATH_MODE_PACKAGE=BOTH \
        -DCMAKE_FIND_ROOT_PATH_MODE_INCLUDE=BOTH \
        -DCMAKE_FIND_ROOT_PATH_MODE_LIBRARY=BOTH \
        -DBUILD_SHARED_LIBS=OFF \
        -DBUILD_PROG=OFF \
        -DSW_BUILD=OFF \
        -DENABLE_GIF=OFF \
        -DENABLE_JPEG=OFF \
        -DENABLE_TIFF=OFF \
        -DENABLE_WEBP=OFF \
        -DENABLE_OPENJPEG=OFF \
        -DENABLE_PNG=ON \
        -DENABLE_ZLIB=ON
    echo "[+] Building leptonica (this takes ~3 min)"
    cmake --build "$LEPT_DIR/build" --parallel
    cmake --install "$LEPT_DIR/build"
fi
echo "[+] leptonica installed at $LEPT_INSTALL"

# ----------------------------------------------------------------------
# Dependency: cpu_features (required by tesseract on Android, the NDK
# ships sources only as Android.mk, no CMake config — we build the
# upstream Google project ourselves).
# ----------------------------------------------------------------------
CPUF_VER=v0.10.1
if [ ! -d "cpu_features-${CPUF_VER#v}" ]; then
    echo "[+] Fetching cpu_features $CPUF_VER"
    curl -fsSL "https://github.com/google/cpu_features/archive/refs/tags/$CPUF_VER.tar.gz" \
        | tar xz
fi
CPUF_DIR="$WORK/cpu_features-${CPUF_VER#v}"
CPUF_INSTALL="$WORK/cpu_features-install"

if [ ! -f "$CPUF_INSTALL/lib/cmake/CpuFeatures/CpuFeaturesConfig.cmake" ] && [ ! -f "$CPUF_INSTALL/lib/cmake/CpuFeatures/cpu_featuresConfig.cmake" ]; then
    echo "[+] Configuring cpu_features"
    rm -rf "$CPUF_DIR/build"
    cmake -S "$CPUF_DIR" -B "$CPUF_DIR/build" \
        -DCMAKE_POLICY_VERSION_MINIMUM=3.5 \
        -DCMAKE_TOOLCHAIN_FILE="$TOOLCHAIN_FILE" \
        -DANDROID_ABI="$ABI" \
        -DANDROID_PLATFORM="android-$API" \
        -DCMAKE_BUILD_TYPE=Release \
        -DCMAKE_INSTALL_PREFIX="$CPUF_INSTALL" \
        -DBUILD_TESTING=OFF \
        -DBUILD_SHARED_LIBS=OFF
    echo "[+] Building cpu_features"
    cmake --build "$CPUF_DIR/build" --parallel
    cmake --install "$CPUF_DIR/build"
fi
echo "[+] cpu_features installed at $CPUF_INSTALL"

# ----------------------------------------------------------------------
# Source: tesseract
# ----------------------------------------------------------------------
TESS_VER=5.4.1
if [ ! -d "tesseract-$TESS_VER" ]; then
    echo "[+] Fetching tesseract $TESS_VER"
    curl -fsSL "https://github.com/tesseract-ocr/tesseract/archive/refs/tags/$TESS_VER.tar.gz" \
        | tar xz
fi
TESS_DIR="$WORK/tesseract-$TESS_VER"
TESS_INSTALL="$WORK/tesseract-install"

if [ ! -f "$TESS_INSTALL/bin/tesseract" ]; then
    echo "[+] Configuring tesseract"
    rm -rf "$TESS_DIR/build"
    cmake -S "$TESS_DIR" -B "$TESS_DIR/build" \
        -DCMAKE_POLICY_VERSION_MINIMUM=3.5 \
        -DCMAKE_TOOLCHAIN_FILE="$TOOLCHAIN_FILE" \
        -DANDROID_ABI="$ABI" \
        -DANDROID_PLATFORM="android-$API" \
        -DCMAKE_BUILD_TYPE=Release \
        -DCMAKE_INSTALL_PREFIX="$TESS_INSTALL" \
        -DLeptonica_DIR="$LEPT_INSTALL/lib/cmake/leptonica" \
        -DCpuFeaturesNdkCompat_DIR="$CPUF_INSTALL/lib/cmake/CpuFeaturesNdkCompat" \
        -DCMAKE_PREFIX_PATH="$LEPT_INSTALL;$CPUF_INSTALL" \
        -DCMAKE_FIND_ROOT_PATH_MODE_PACKAGE=BOTH \
        -DCMAKE_FIND_ROOT_PATH_MODE_INCLUDE=BOTH \
        -DCMAKE_FIND_ROOT_PATH_MODE_LIBRARY=BOTH \
        -DLEPT_TIFF_RESULT=1 \
        -DBUILD_SHARED_LIBS=OFF \
        -DBUILD_TRAINING_TOOLS=OFF \
        -DDISABLE_TIFF=ON \
        -DDISABLE_ARCHIVE=ON \
        -DDISABLE_CURL=ON \
        -DGRAPHICS_DISABLED=ON \
        -DOPENMP_BUILD=OFF
    echo "[+] Building tesseract (this takes ~5 min)"
    cmake --build "$TESS_DIR/build" --parallel
    cmake --install "$TESS_DIR/build"
fi
TESS_BIN="$TESS_INSTALL/bin/tesseract"
[ -f "$TESS_BIN" ] || { echo "ERROR: tesseract binary missing at $TESS_BIN"; exit 2; }
echo "[+] tesseract built at $TESS_BIN"

# Strip to shrink the binary (typically 20MB → 5-8MB).
STRIP="$NDK/toolchains/llvm/prebuilt/$(uname -s | tr A-Z a-z)-x86_64/bin/llvm-strip"
[ -x "$STRIP" ] || STRIP="$NDK/toolchains/llvm/prebuilt/darwin-x86_64/bin/llvm-strip"
if [ -x "$STRIP" ]; then
    echo "[+] Stripping debug symbols"
    "$STRIP" --strip-unneeded "$TESS_BIN"
fi

# ----------------------------------------------------------------------
# Tessdata
# ----------------------------------------------------------------------
TESSDATA_DIR="$WORK/tessdata"
mkdir -p "$TESSDATA_DIR"
for lang in eng tha; do
    out="$TESSDATA_DIR/$lang.traineddata"
    if [ ! -f "$out" ]; then
        echo "[+] Fetching $lang tessdata (fast)"
        curl -fsSL -o "$out" \
            "https://github.com/tesseract-ocr/tessdata_fast/raw/main/$lang.traineddata"
    fi
done

# ----------------------------------------------------------------------
# Stage into Magisk module
# ----------------------------------------------------------------------
MAGISK_BIN="$REPO_ROOT/magisk/peko-module/system/bin"
MAGISK_TESSDATA="$REPO_ROOT/magisk/peko-module/system/etc/tessdata"
mkdir -p "$MAGISK_BIN" "$MAGISK_TESSDATA"

cp -v "$TESS_BIN" "$MAGISK_BIN/tesseract"
chmod 0755 "$MAGISK_BIN/tesseract"
for lang in eng tha; do
    cp -v "$TESSDATA_DIR/$lang.traineddata" "$MAGISK_TESSDATA/$lang.traineddata"
done

echo
echo "[done] Bundled into:"
echo "  $MAGISK_BIN/tesseract"
ls -la "$MAGISK_BIN/tesseract"
echo "  $MAGISK_TESSDATA/"
ls -la "$MAGISK_TESSDATA/"
echo
echo "Next: rebuild the Magisk module (./magisk/build-module.sh) and flash."
