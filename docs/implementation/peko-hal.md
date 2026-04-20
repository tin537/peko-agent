# peko-hal

> Hardware Abstraction Layer — safe Rust wrappers around kernel ioctls.

---

## Purpose

`peko-hal` sits between [[peko-tools-android|Android tools]] and the Linux kernel. It provides typed, safe Rust interfaces to raw kernel device nodes, converting unsafe `ioctl`/`read`/`write` syscalls into ergonomic Rust APIs.

This crate uses the `nix` crate for safe ioctl wrappers and `libc` for low-level types.

## Components

### InputDevice

Wraps `/dev/input/event*` for input event injection and reading.

```rust
pub struct InputDevice {
    fd: OwnedFd,
    name: String,
    device_path: PathBuf,
}

impl InputDevice {
    /// Scan /dev/input/event* and find the touchscreen
    pub fn find_touchscreen() -> Result<Self>;

    /// Find keyboard/button device
    pub fn find_keyboard() -> Result<Self>;

    /// Inject a single input_event
    pub fn write_event(&self, type_: u16, code: u16, value: i32) -> Result<()>;

    /// Inject a tap gesture at (x, y)
    pub fn inject_tap(&self, x: i32, y: i32) -> Result<()>;

    /// Inject a swipe from (x1,y1) to (x2,y2) over duration_ms
    pub fn inject_swipe(&self, x1: i32, y1: i32, x2: i32, y2: i32,
                        duration_ms: u64) -> Result<()>;
}
```

Device identification uses `ioctl(fd, EVIOCGNAME)` to read the device name string.

See [[../knowledge/Touch-Input-System]] for the full evdev protocol.

### Framebuffer

Reads screen content from the legacy framebuffer interface:

```rust
pub struct Framebuffer {
    fd: OwnedFd,
    mmap: *mut u8,
    width: u32,
    height: u32,
    stride: u32,        // bytes per line
    bits_per_pixel: u32,
    pixel_format: PixelFormat,
}

impl Framebuffer {
    /// Open /dev/graphics/fb0
    pub fn open() -> Result<Self>;

    /// Capture current screen as RGBA buffer
    pub fn capture(&self) -> Result<RgbaBuffer>;

    /// Get display dimensions
    pub fn dimensions(&self) -> (u32, u32);
}
```

Uses two ioctls:
- `FBIOGET_VSCREENINFO` — variable screen info (resolution, bits per pixel, color offsets)
- `FBIOGET_FSCREENINFO` — fixed screen info (line length, memory size)

Then `mmap`s the framebuffer memory for zero-copy pixel reads.

See [[../knowledge/Screen-Capture]] for details.

### DrmDisplay

Modern alternative to Framebuffer for devices that have deprecated `fb0`:

```rust
pub struct DrmDisplay {
    fd: OwnedFd,
    connector_id: u32,
    crtc_id: u32,
    width: u32,
    height: u32,
}

impl DrmDisplay {
    /// Open /dev/dri/card0
    pub fn open() -> Result<Self>;

    /// Capture current screen
    pub fn capture(&self) -> Result<RgbaBuffer>;
}
```

Uses DRM/KMS ioctls to enumerate connectors, find the active CRTC, and create a dumb buffer for screen capture.

See [[../knowledge/Screen-Capture]] for the DRM protocol.

### SerialModem

Manages the serial connection to the cellular modem:

```rust
pub struct SerialModem {
    fd: OwnedFd,
    device_path: PathBuf,
}

impl SerialModem {
    /// Open modem serial device (auto-detect or explicit path)
    pub fn open() -> Result<Self>;

    /// Send an AT command and read the response
    pub fn send_command(&self, cmd: &str, timeout_ms: u64) -> Result<String>;

    /// Read unsolicited result codes (RING, etc.)
    pub fn read_urc(&self, timeout_ms: u64) -> Result<Option<String>>;
}
```

Configures `termios`: 115200 baud, 8N1, no hardware flow control.

See [[../knowledge/Telephony-AT-Commands]] for the AT command reference.

### UInputDevice

Creates virtual input devices via the kernel's uinput subsystem:

```rust
pub struct UInputDevice {
    fd: OwnedFd,
}

impl UInputDevice {
    /// Create a virtual touchscreen device
    pub fn create_touchscreen(name: &str, width: i32, height: i32) -> Result<Self>;

    /// Inject events through the virtual device
    pub fn write_event(&self, type_: u16, code: u16, value: i32) -> Result<()>;
}
```

Opens `/dev/uinput`, configures device capabilities via ioctls (`UI_SET_EVBIT`, `UI_SET_ABSBIT`, etc.), then creates the device with `UI_DEV_CREATE`. This is an alternative to writing directly to physical device nodes.

## Safety Model

All ioctl calls go through `nix` crate wrappers where possible, which encode the ioctl number and argument type in the Rust type system:

```rust
// nix ioctl macro generates a type-safe wrapper
nix::ioctl_read!(eviocgname, b'E', 0x06, [u8; 256]);

// Usage
let mut name_buf = [0u8; 256];
unsafe { eviocgname(fd, &mut name_buf)? };
```

File descriptors use `OwnedFd` for automatic cleanup. Memory-mapped regions are wrapped in a struct with `Drop` to ensure `munmap` is called.

## Dependencies

```toml
[dependencies]
nix = { version = "0.29", features = ["ioctl", "fs", "term"] }
libc = "0.2"
tracing = "0.1"
```

Minimal — this crate deliberately avoids pulling in async runtime or serialization dependencies.

## Related

- [[peko-tools-android]] — Uses HAL to implement tools
- [[../knowledge/Touch-Input-System]] — evdev protocol details
- [[../knowledge/Screen-Capture]] — Framebuffer and DRM details
- [[../knowledge/Telephony-AT-Commands]] — AT command protocol
- [[../architecture/Crate-Map]] — Dependency position (leaf crate)

---

#implementation #hal #hardware #kernel
