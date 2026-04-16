# Touch Input System

> Injecting touch events via the Linux evdev interface.

---

## How Android Touch Works (Normally)

```
Touchscreen hardware → Kernel driver → /dev/input/eventN
  → InputReader (SystemServer) → InputDispatcher → WindowManager → App
```

Peko Agent **short-circuits** this by writing directly to `/dev/input/eventN`, skipping everything after the kernel driver.

## The evdev Protocol

Linux's input subsystem represents all input as `input_event` structs:

```c
struct input_event {
    struct timeval time;  // Timestamp
    __u16 type;           // Event type (EV_ABS, EV_KEY, EV_SYN)
    __u16 code;           // Event code (ABS_MT_POSITION_X, KEY_HOME, etc.)
    __s32 value;          // Event value (coordinate, pressed/released)
};
```

In Rust (via `libc` or `nix`):

```rust
#[repr(C)]
pub struct InputEvent {
    pub time: libc::timeval,
    pub type_: u16,
    pub code: u16,
    pub value: i32,
}
```

## Device Discovery

At startup, [[../implementation/peko-hal|peko-hal]] scans `/dev/input/event*`:

```rust
for entry in std::fs::read_dir("/dev/input")? {
    let path = entry?.path();
    let fd = open(&path, O_RDWR)?;
    let mut name = [0u8; 256];
    ioctl(fd, EVIOCGNAME, &mut name)?;

    let name_str = CStr::from_ptr(name.as_ptr()).to_str()?;
    // Touchscreens typically have names like:
    // "sec_touchscreen", "fts_ts", "atmel_mxt_ts", "goodix_ts"
    if is_touchscreen(name_str) {
        return Ok(InputDevice { fd, name: name_str, path });
    }
}
```

Also uses `ioctl(EVIOCGBIT)` to check for `EV_ABS` + `ABS_MT_POSITION_X` capabilities as a more reliable identification method.

## Tap Gesture

A tap is: finger down → sync → finger up → sync

```rust
pub fn inject_tap(&self, x: i32, y: i32) -> Result<()> {
    let now = current_timeval();

    // Finger down
    self.write_event(EV_ABS, ABS_MT_TRACKING_ID, 0)?;   // Assign tracking ID
    self.write_event(EV_ABS, ABS_MT_POSITION_X, x)?;     // X coordinate
    self.write_event(EV_ABS, ABS_MT_POSITION_Y, y)?;     // Y coordinate
    self.write_event(EV_ABS, ABS_MT_PRESSURE, 50)?;      // Pressure
    self.write_event(EV_ABS, ABS_MT_TOUCH_MAJOR, 5)?;    // Contact area
    self.write_event(EV_SYN, SYN_REPORT, 0)?;            // Sync (commit events)

    // Brief delay (10ms simulates real finger)
    tokio::time::sleep(Duration::from_millis(10)).await;

    // Finger up
    self.write_event(EV_ABS, ABS_MT_TRACKING_ID, -1)?;   // Release tracking
    self.write_event(EV_SYN, SYN_REPORT, 0)?;            // Sync
    Ok(())
}
```

## Swipe Gesture

A swipe is: down → series of moves → up

```rust
pub fn inject_swipe(&self, x1: i32, y1: i32, x2: i32, y2: i32,
                    duration_ms: u64) -> Result<()> {
    let steps = 20;
    let step_delay = Duration::from_millis(duration_ms / steps);

    // Finger down at start
    self.write_event(EV_ABS, ABS_MT_TRACKING_ID, 0)?;
    self.write_event(EV_ABS, ABS_MT_POSITION_X, x1)?;
    self.write_event(EV_ABS, ABS_MT_POSITION_Y, y1)?;
    self.write_event(EV_ABS, ABS_MT_PRESSURE, 50)?;
    self.write_event(EV_SYN, SYN_REPORT, 0)?;

    // Interpolated moves
    for i in 1..=steps {
        let t = i as f32 / steps as f32;
        let cx = x1 + ((x2 - x1) as f32 * t) as i32;
        let cy = y1 + ((y2 - y1) as f32 * t) as i32;

        self.write_event(EV_ABS, ABS_MT_POSITION_X, cx)?;
        self.write_event(EV_ABS, ABS_MT_POSITION_Y, cy)?;
        self.write_event(EV_SYN, SYN_REPORT, 0)?;

        tokio::time::sleep(step_delay).await;
    }

    // Finger up
    self.write_event(EV_ABS, ABS_MT_TRACKING_ID, -1)?;
    self.write_event(EV_SYN, SYN_REPORT, 0)?;
    Ok(())
}
```

## Multi-Touch Protocol

Android touchscreens use the **Linux Multi-Touch Protocol B** (slot-based):

| Code | Purpose |
|---|---|
| `ABS_MT_SLOT` | Which finger (0, 1, 2...) |
| `ABS_MT_TRACKING_ID` | Finger lifecycle (-1 = lift) |
| `ABS_MT_POSITION_X` | X coordinate |
| `ABS_MT_POSITION_Y` | Y coordinate |
| `ABS_MT_PRESSURE` | Pressure (0-255) |
| `ABS_MT_TOUCH_MAJOR` | Contact area major axis |

For single-finger agent operations, slot 0 and tracking ID 0 are sufficient.

## Coordinate Systems

The evdev coordinate space may differ from screen pixels:

```rust
// Query the touchscreen's coordinate range
let mut abs_info = input_absinfo::default();
ioctl(fd, EVIOCGABS(ABS_MT_POSITION_X), &mut abs_info)?;
// abs_info.minimum = 0
// abs_info.maximum = 1079 (for 1080px width)
// abs_info.resolution = pixels per mm (optional)
```

If the evdev range matches screen resolution (common on Android), coordinates map 1:1. Otherwise, scale:

```
evdev_x = screen_x * evdev_max_x / screen_width
```

## UInput Alternative

Instead of writing to the physical touchscreen device, create a **virtual device** via `/dev/uinput`:

```rust
let fd = open("/dev/uinput", O_WRONLY)?;
ioctl(fd, UI_SET_EVBIT, EV_ABS)?;
ioctl(fd, UI_SET_ABSBIT, ABS_MT_POSITION_X)?;
ioctl(fd, UI_SET_ABSBIT, ABS_MT_POSITION_Y)?;
// ... configure all capabilities
ioctl(fd, UI_DEV_CREATE)?;
// Now write events to this fd
```

Advantages:
- No need to find the physical device node
- Works even if multiple touchscreen devices exist
- More predictable coordinate mapping

See [[../implementation/peko-hal]] for the `UInputDevice` implementation.

## Related

- [[../implementation/peko-hal]] — InputDevice and UInputDevice structs
- [[../implementation/peko-tools-android]] — TouchTool, KeyEventTool
- [[Linux-Kernel-Interfaces]] — Overview of all kernel interfaces
- [[SELinux-Policy]] — Permission for `/dev/input/*` access

---

#knowledge #touch #input #evdev #kernel
