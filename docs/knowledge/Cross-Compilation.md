# Cross-Compilation

> Building Rust for Android's aarch64-linux-android target.

---

## Toolchain Setup

### Prerequisites

1. **Rust toolchain** with the Android target:
   ```bash
   rustup target add aarch64-linux-android
   ```

2. **Android NDK** (r25+ recommended):
   ```bash
   # Via Android Studio SDK Manager, or:
   wget https://dl.google.com/android/repository/android-ndk-r25c-linux-x86_64.zip
   ```

3. **Set NDK path**:
   ```bash
   export ANDROID_NDK_HOME=/path/to/android-ndk-r25c
   export PATH="$ANDROID_NDK_HOME/toolchains/llvm/prebuilt/linux-x86_64/bin:$PATH"
   ```

### Cargo Configuration

`.cargo/config.toml` in the workspace root:

```toml
[target.aarch64-linux-android]
linker = "aarch64-linux-android31-clang"
rustflags = ["-C", "link-arg=-landroid", "-C", "link-arg=-llog"]

[target.armv7-linux-androideabi]
linker = "armv7a-linux-androideabi31-clang"
rustflags = ["-C", "link-arg=-landroid", "-C", "link-arg=-llog"]
```

The `31` in the linker name is the Android API level (Android 12). Use the minimum API level your target devices support.

## Build Commands

```bash
# Build for Android arm64
cargo build --target aarch64-linux-android --release

# The binary is at:
# target/aarch64-linux-android/release/peko-agent
```

### Build optimizations

```toml
# Cargo.toml [profile.release]
[profile.release]
opt-level = "z"      # Optimize for size
lto = true           # Link-time optimization
codegen-units = 1    # Better optimization, slower build
strip = true         # Strip debug symbols
panic = "abort"      # Smaller binary (no unwind tables)
```

## Dependency Considerations

| Dependency | Android notes |
|---|---|
| `reqwest` | Use `rustls-tls` feature, NOT `native-tls` (avoids OpenSSL) |
| `rusqlite` | Use `bundled` feature (compiles SQLite from source) |
| `nix` | Works out of the box — wraps libc |
| `tokio` | Works out of the box |
| `image` | Works out of the box |

### Avoiding OpenSSL

OpenSSL is the most common cross-compilation headache. Peko Agent avoids it entirely:

```toml
[dependencies]
reqwest = { version = "0.12", default-features = false, features = [
    "rustls-tls",   # Use rustls instead of OpenSSL
    "stream",
    "json",
] }
```

`rustls` is a pure-Rust TLS implementation that cross-compiles trivially.

## Deploying to Device

### Via ADB (development)

```bash
# Push binary
adb push target/aarch64-linux-android/release/peko-agent /data/local/tmp/

# Make executable
adb shell chmod 755 /data/local/tmp/peko-agent

# Test run (as root)
adb shell su -c /data/local/tmp/peko-agent --config /data/peko/config.toml
```

### Via system image (production)

For proper [[../architecture/Boot-Sequence|init.rc integration]]:

1. Mount system partition read-write: `adb shell mount -o remount,rw /system`
2. Copy binary: `adb push peko-agent /system/bin/`
3. Copy init script: `adb push peko-agent.rc /system/etc/init/`
4. Copy SELinux policy files
5. Set labels: `adb shell restorecon /system/bin/peko-agent`
6. Reboot

### Via custom ROM (ideal)

Integrate into AOSP build:
1. Add binary to `PRODUCT_PACKAGES`
2. Include `.rc` file in device makefile
3. Add [[SELinux-Policy|SELinux policy]] to `BOARD_SEPOLICY_DIRS`
4. Build ROM: `make -j$(nproc)`

## Binary Size Budget

Target: **< 15 MB**

| Component | Estimated size |
|---|---|
| Core agent logic | ~500 KB |
| reqwest + rustls | ~1.5 MB |
| SQLite (bundled) | ~1.5 MB |
| tokio runtime | ~1 MB |
| image (PNG only) | ~300 KB |
| nix + libc | ~100 KB |
| Other | ~500 KB |
| **Total (stripped, LTO)** | **~5-8 MB** |

Well within the 15 MB budget.

## Testing Locally

Before deploying to a device, test on desktop:

```bash
# Build for host (macOS/Linux)
cargo build --release

# Run with mock tools
cargo test

# Build for Android
cargo build --target aarch64-linux-android --release
```

The platform-agnostic crates ([[../implementation/peko-core|peko-core]], [[../implementation/peko-transport|peko-transport]], [[../implementation/peko-config|peko-config]]) can be fully tested on the host.

## Related

- [[Rust-On-Android]] — Rust's role in AOSP
- [[../architecture/Crate-Map]] — What gets compiled
- [[../roadmap/Phase-5-Integration]] — Integration build phase
- [[../roadmap/Phase-6-Android-Deploy]] — Deployment steps

---

#knowledge #build #cross-compilation #ndk
