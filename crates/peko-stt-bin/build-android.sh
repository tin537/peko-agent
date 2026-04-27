#!/usr/bin/env bash
# Cross-compile whisper-cli for Android aarch64 via the NDK. Pattern
# mirrors crates/peko-llm-daemon/build-android.sh — same NDK, same
# CMake quirks, same outputs structure.
set -euo pipefail

NDK="${ANDROID_NDK_HOME:-$HOME/Library/Android/sdk/ndk/30.0.14904198}"
ABI="${ANDROID_ABI:-arm64-v8a}"
API="${ANDROID_API:-31}"
BUILD_TYPE="${BUILD_TYPE:-Release}"
BUILD_DIR="${BUILD_DIR:-build-android-$ABI}"
JOBS="${JOBS:-$(sysctl -n hw.ncpu 2>/dev/null || nproc)}"

if [ -d "$NDK/toolchains/llvm/prebuilt/darwin-x86_64" ]; then
    NDK_HOST="darwin-x86_64"
elif [ -d "$NDK/toolchains/llvm/prebuilt/linux-x86_64" ]; then
    NDK_HOST="linux-x86_64"
else
    echo "error: could not detect NDK host platform under $NDK/toolchains/llvm/prebuilt/" >&2
    exit 1
fi

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

export PATH="$NDK/toolchains/llvm/prebuilt/$NDK_HOST/bin:$PATH"
# Use Android SDK's CMake (bundles a working Ninja) over Homebrew's.
if [ -x "$HOME/Library/Android/sdk/cmake/4.1.2/bin/cmake" ]; then
    export PATH="$HOME/Library/Android/sdk/cmake/4.1.2/bin:$PATH"
fi

echo "→ NDK:        $NDK"
echo "→ ABI:        $ABI"
echo "→ API:        $API"
echo "→ build dir:  $BUILD_DIR"
echo "→ build type: $BUILD_TYPE"
echo

cmake -S . -B "$BUILD_DIR" \
    -DCMAKE_TOOLCHAIN_FILE="$NDK/build/cmake/android.toolchain.cmake" \
    -DANDROID_ABI="$ABI" \
    -DANDROID_PLATFORM="android-$API" \
    -DANDROID_STL=c++_static \
    -DCMAKE_BUILD_TYPE="$BUILD_TYPE" \
    -DCMAKE_INSTALL_PREFIX="$BUILD_DIR/out"

cmake --build "$BUILD_DIR" -j "$JOBS"
cmake --install "$BUILD_DIR" 2>/dev/null || true

# whisper-cli ends up in build-*/bin or build-*/<example_dir>; surface it.
BIN=$(find "$BUILD_DIR" -maxdepth 6 -name whisper-cli -type f -perm +111 2>/dev/null | head -1)
if [ -z "$BIN" ]; then
    BIN=$(find "$BUILD_DIR" -maxdepth 6 -name whisper-cli -type f 2>/dev/null | head -1)
fi
if [ -n "$BIN" ]; then
    cp "$BIN" "$BUILD_DIR/whisper-cli"
    echo
    echo "✓ built: $BUILD_DIR/whisper-cli"
    ls -lh "$BUILD_DIR/whisper-cli"
    file "$BUILD_DIR/whisper-cli" 2>/dev/null || true
else
    echo "✗ whisper-cli binary not found in $BUILD_DIR" >&2
    exit 1
fi
