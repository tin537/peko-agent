#!/usr/bin/env bash
# Cross-compile peko-llm-daemon for Android aarch64 via the NDK.
# Includes llama.cpp + Vulkan backend (Adreno 610 / Mali / etc).
set -euo pipefail

# ── Config (override via env) ───────────────────────────────────
NDK="${ANDROID_NDK_HOME:-$HOME/Library/Android/sdk/ndk/30.0.14904198}"
ABI="${ANDROID_ABI:-arm64-v8a}"
API="${ANDROID_API:-31}"
BUILD_TYPE="${BUILD_TYPE:-Release}"
BUILD_DIR="${BUILD_DIR:-build-android-$ABI}"
VULKAN="${VULKAN:-ON}"   # set VULKAN=OFF for CPU-only build
JOBS="${JOBS:-$(sysctl -n hw.ncpu 2>/dev/null || nproc)}"

# ── Detect NDK host ─────────────────────────────────────────────
if [ -d "$NDK/toolchains/llvm/prebuilt/darwin-x86_64" ]; then
    NDK_HOST="darwin-x86_64"
elif [ -d "$NDK/toolchains/llvm/prebuilt/linux-x86_64" ]; then
    NDK_HOST="linux-x86_64"
else
    echo "error: could not detect NDK host platform under $NDK/toolchains/llvm/prebuilt/" >&2
    exit 1
fi

# glslc lives in NDK's shader-tools, not the toolchain bin dir
GLSLC="$NDK/shader-tools/$NDK_HOST/glslc"
if [ "$VULKAN" = "ON" ] && [ ! -x "$GLSLC" ]; then
    # Fallback — older NDKs put it in toolchain/llvm bin
    GLSLC="$NDK/toolchains/llvm/prebuilt/$NDK_HOST/bin/glslc"
fi
if [ "$VULKAN" = "ON" ] && [ ! -x "$GLSLC" ]; then
    echo "warning: glslc not found (tried shader-tools and toolchain/bin)" >&2
    echo "         install via 'sdkmanager \"ndk;VERSION\"' or build glslc from source" >&2
fi

# ── Sanity checks ───────────────────────────────────────────────
if [ ! -d "$NDK" ]; then
    echo "error: NDK not found at $NDK" >&2
    exit 1
fi

if [ ! -f "$NDK/build/cmake/android.toolchain.cmake" ]; then
    echo "error: NDK toolchain file missing" >&2
    exit 1
fi

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
cd "$SCRIPT_DIR"

# Put CMake + glslc on PATH (used by FetchContent'd llama.cpp during shader compile)
export PATH="$NDK/toolchains/llvm/prebuilt/$NDK_HOST/bin:$NDK/shader-tools/$NDK_HOST:$PATH"
# Homebrew CMake often outranks Android SDK's; both are fine but pick consistent one.
if [ -x "$HOME/Library/Android/sdk/cmake/4.1.2/bin/cmake" ]; then
    export PATH="$HOME/Library/Android/sdk/cmake/4.1.2/bin:$PATH"
fi

echo "→ NDK:        $NDK"
echo "→ NDK host:   $NDK_HOST"
echo "→ ABI:        $ABI"
echo "→ API:        $API"
echo "→ Vulkan:     $VULKAN"
echo "→ glslc:      $GLSLC"
echo "→ build dir:  $BUILD_DIR"
echo "→ build type: $BUILD_TYPE"
echo

# ── Configure ───────────────────────────────────────────────────
cmake -S . -B "$BUILD_DIR" \
    -DCMAKE_TOOLCHAIN_FILE="$NDK/build/cmake/android.toolchain.cmake" \
    -DANDROID_ABI="$ABI" \
    -DANDROID_PLATFORM="android-$API" \
    -DANDROID_STL=c++_static \
    -DCMAKE_BUILD_TYPE="$BUILD_TYPE" \
    -DCMAKE_INSTALL_PREFIX="$BUILD_DIR/out" \
    -DPEKO_VULKAN="$VULKAN" \
    -DVulkan_GLSLC_EXECUTABLE="$GLSLC"

# ── Build ───────────────────────────────────────────────────────
# VERBOSE=1 makes CMake echo every compile + link command. Without it,
# a failed target just prints `make: *** Error 2` with no context, which
# burned an hour of CI debugging. Cost: ~30% larger log files. Accept.
cmake --build "$BUILD_DIR" -j "$JOBS" --verbose

# ── Report ──────────────────────────────────────────────────────
BIN="$BUILD_DIR/peko-llm-daemon"
if [ -f "$BIN" ]; then
    echo
    echo "✓ built: $BIN"
    ls -lh "$BIN"
    file "$BIN" 2>/dev/null || true
else
    echo "✗ binary not found at $BIN" >&2
    exit 1
fi
