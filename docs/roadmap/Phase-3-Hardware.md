# Phase 3: Hardware

> HAL — wrapping kernel device interfaces in safe Rust.

---

## Goal

Safe Rust wrappers for all kernel interfaces Peko Agent needs. Tested on a real rooted Android device.

## Prerequisites

- Rooted Android device (see [[Device-Requirements]])
- [[../knowledge/Cross-Compilation|NDK toolchain]] configured
- ADB access to device

## Tasks

### 3.1 Cross-compilation Setup

- [ ] Add `aarch64-linux-android` target to Rust toolchain
- [ ] Configure `.cargo/config.toml` with NDK linker
- [ ] Verify a minimal "hello world" binary runs on device via ADB
- [ ] Set up `[profile.release]` optimizations (LTO, size optimization)

See [[../knowledge/Cross-Compilation]] for details.

### 3.2 InputDevice (evdev)

- [ ] Implement device enumeration: scan `/dev/input/event*`
- [ ] Implement `EVIOCGNAME` ioctl for device identification
- [ ] Implement `EVIOCGBIT` / `EVIOCGABS` for capability detection
- [ ] Implement `write_event()` for raw event injection
- [ ] Implement `inject_tap()` — full tap gesture with proper MT protocol
- [ ] Implement `inject_swipe()` — interpolated move sequence
- [ ] Test: push binary to device, inject a tap, verify it registers

See [[../knowledge/Touch-Input-System]] for the evdev protocol.

### 3.3 Framebuffer

- [ ] Implement `FBIOGET_VSCREENINFO` and `FBIOGET_FSCREENINFO` ioctls
- [ ] Implement `mmap()` of framebuffer memory
- [ ] Implement pixel format detection (RGBA, BGRA, etc.)
- [ ] Implement `capture()` → raw RGBA buffer
- [ ] Handle stride/padding in row reads
- [ ] Test: capture screen, save as PNG, verify visually via ADB pull

See [[../knowledge/Screen-Capture]] for framebuffer details.

### 3.4 DrmDisplay (fallback)

- [ ] Implement DRM resource enumeration (connectors, CRTCs)
- [ ] Find active connector + CRTC
- [ ] Implement dumb buffer creation + mapping for capture
- [ ] Test on a device that lacks `/dev/graphics/fb0`

### 3.5 SerialModem

- [ ] Implement modem device discovery (scan `/sys/class/tty/`)
- [ ] Implement `termios` configuration (115200, 8N1)
- [ ] Implement `send_command()` with timeout
- [ ] Implement URC (unsolicited result code) reading
- [ ] Test: send `AT` command, receive `OK`
- [ ] Test: query signal strength with `AT+CSQ`

See [[../knowledge/Telephony-AT-Commands]] for the AT protocol.

### 3.6 UInputDevice

- [ ] Implement virtual device creation via `/dev/uinput`
- [ ] Configure touchscreen capabilities
- [ ] Implement event injection through virtual device
- [ ] Test as alternative to direct evdev writes

## Testing Approach

Each HAL component gets a standalone test binary:

```bash
# Build test binary for Android
cargo build --target aarch64-linux-android --release --example test_input

# Push and run on device
adb push target/aarch64-linux-android/release/examples/test_input /data/local/tmp/
adb shell su -c /data/local/tmp/test_input
```

Test binaries should be simple: one test per binary, clear stdout output, exit code 0 on success.

## Device Access Notes

All hardware access requires root. On a rooted device:

```bash
adb shell su -c "ls -la /dev/input/"
adb shell su -c "cat /proc/bus/input/devices"  # List all input devices
adb shell su -c "ls -la /dev/graphics/"
adb shell su -c "ls -la /dev/tty*"
```

## Definition of Done

Three standalone test binaries that, when run on a rooted device:
1. **test_input**: Finds touchscreen, injects a tap → visible screen tap
2. **test_framebuffer**: Captures screen → saves valid PNG
3. **test_modem**: Sends AT command → receives OK response

## Output Artifacts

- `crates/peko-hal/` — fully functional on Android
- Test binaries for each HAL component
- Documentation of device-specific quirks discovered

## Related

- [[Phase-2-Transport]] — Previous phase
- [[Phase-4-Tools]] — Next phase (uses HAL)
- [[../implementation/peko-hal]] — HAL crate design
- [[Device-Requirements]] — Hardware needs
- [[../knowledge/Cross-Compilation]] — Build setup

---

#roadmap #phase-3 #hardware #hal
