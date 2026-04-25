use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::os::unix::io::AsRawFd;
use std::path::{Path, PathBuf};

#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
struct InputEvent {
    tv_sec: libc::time_t,
    tv_usec: libc::suseconds_t,
    type_: u16,
    code: u16,
    value: i32,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
struct AbsInfo {
    value: i32,
    minimum: i32,
    maximum: i32,
    fuzz: i32,
    flat: i32,
    resolution: i32,
}

const EV_SYN: u16 = 0x00;
const EV_KEY: u16 = 0x01;
const EV_ABS: u16 = 0x03;
const SYN_REPORT: u16 = 0x00;
const ABS_MT_SLOT: u16 = 0x2f;
const ABS_MT_TRACKING_ID: u16 = 0x39;
const ABS_MT_POSITION_X: u16 = 0x35;
const ABS_MT_POSITION_Y: u16 = 0x36;
const ABS_MT_PRESSURE: u16 = 0x3a;
const ABS_MT_TOUCH_MAJOR: u16 = 0x30;

const BTN_TOUCH: u16 = 0x14a;
const BTN_TOOL_FINGER: u16 = 0x145;

const KEY_HOME: u16 = 102;
const KEY_HOMEPAGE: u16 = 172;
const KEY_BACK: u16 = 158;
const KEY_POWER: u16 = 116;
const KEY_VOLUMEUP: u16 = 115;
const KEY_VOLUMEDOWN: u16 = 114;
const KEY_ENTER: u16 = 28;

// EVIOCGABS(abs) = _IOR('E', 0x40 + abs, sizeof(struct input_absinfo=24))
//                = (2<<30) | (24<<16) | ('E'<<8) | (0x40 + abs)
const EVIOCGABS_BASE: u64 = 0x8018_4540;
// EVIOCGBIT(EV_KEY, 96) = _IOC(READ, 'E', 0x20 + 0x01, 96)
const EVIOCGBIT_EV_KEY_96: u64 = 0x8060_4521;

/// Decoded form of a kernel `struct input_event` for callers that want
/// to observe input rather than inject it. Timestamp is omitted because
/// the kernel zeroes it on read for non-realtime devices and we never
/// actually need wall-clock timing — the agent loop runs at human speeds.
#[derive(Debug, Clone, Copy)]
pub struct RawInputEvent {
    pub type_: u16,
    pub code: u16,
    pub value: i32,
}

pub struct InputDevice {
    file: File,
    path: PathBuf,
    name: String,
    abs_x: Option<AbsInfo>,
    abs_y: Option<AbsInfo>,
    abs_pressure: Option<AbsInfo>,
    abs_touch_major: Option<AbsInfo>,
    display_size: Option<(i32, i32)>,
    tracking_id_next: i32,
}

impl InputDevice {
    pub fn open(path: &Path) -> anyhow::Result<Self> {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(path)?;

        let fd = file.as_raw_fd();
        let name = Self::read_device_name(fd).unwrap_or_else(|_| "unknown".to_string());

        let abs_x = Self::read_abs_info(fd, ABS_MT_POSITION_X).ok();
        let abs_y = Self::read_abs_info(fd, ABS_MT_POSITION_Y).ok();
        let abs_pressure = Self::read_abs_info(fd, ABS_MT_PRESSURE).ok();
        let abs_touch_major = Self::read_abs_info(fd, ABS_MT_TOUCH_MAJOR).ok();

        Ok(Self {
            file,
            path: path.to_path_buf(),
            name,
            abs_x,
            abs_y,
            abs_pressure,
            abs_touch_major,
            display_size: None,
            tracking_id_next: 1,
        })
    }

    pub fn find_touchscreen() -> anyhow::Result<Self> {
        let input_dir = Path::new("/dev/input");
        if !input_dir.exists() {
            anyhow::bail!("no /dev/input directory found");
        }

        let mut fallback: Option<Self> = None;

        for entry in fs::read_dir(input_dir)? {
            let entry = entry?;
            let path = entry.path();
            let name = path.file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("");

            if !name.starts_with("event") { continue; }

            if let Ok(device) = Self::open(&path) {
                let lower = device.name.to_lowercase();

                // Real hardware touchscreens
                if lower.contains("touch") || lower.contains("ts")
                    || lower.contains("fts") || lower.contains("goodix")
                    || lower.contains("atmel") || lower.contains("sec_touch")
                    || lower.contains("synaptics") || lower.contains("himax")
                    || lower.contains("novatek") || lower.contains("focaltech")
                    || lower.contains("ilitek") || lower.contains("melfas")
                {
                    return Ok(device);
                }

                // Emulator touchscreen devices
                if lower.contains("goldfish") || lower.contains("virtio")
                    || lower.contains("qemu") || lower.contains("ranchu")
                    || lower.contains("generic")
                {
                    if fallback.is_none() {
                        fallback = Some(device);
                    }
                }
            }
        }

        if let Some(device) = fallback {
            return Ok(device);
        }

        let event0 = input_dir.join("event0");
        if event0.exists() {
            return Self::open(&event0);
        }

        anyhow::bail!("no touchscreen device found in /dev/input/")
    }

    pub fn find_keyboard() -> anyhow::Result<Self> {
        let input_dir = Path::new("/dev/input");
        for entry in fs::read_dir(input_dir)? {
            let entry = entry?;
            let path = entry.path();
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if !name.starts_with("event") { continue; }

            if let Ok(device) = Self::open(&path) {
                let lower = device.name.to_lowercase();
                if lower.contains("key") || lower.contains("button")
                    || lower.contains("gpio")
                {
                    return Ok(device);
                }
            }
        }
        anyhow::bail!("no keyboard/button device found")
    }

    /// Walk /dev/input/event* and return the first device whose EV_KEY
    /// capability bitmap has `keycode` set. HOME/BACK/POWER often live on
    /// a different node than the touchscreen (gpio-keys, qpnp_pon, etc).
    pub fn find_device_with_key(keycode: u16) -> anyhow::Result<Self> {
        let input_dir = Path::new("/dev/input");
        for entry in fs::read_dir(input_dir)? {
            let entry = entry?;
            let path = entry.path();
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if !name.starts_with("event") { continue; }

            if let Ok(device) = Self::open(&path) {
                if device.supports_key(keycode) {
                    return Ok(device);
                }
            }
        }
        anyhow::bail!("no input device advertises keycode {}", keycode)
    }

    fn supports_key(&self, keycode: u16) -> bool {
        let mut bits = [0u8; 96];
        let ret = unsafe {
            crate::raw_ioctl(
                self.file.as_raw_fd(),
                EVIOCGBIT_EV_KEY_96,
                bits.as_mut_ptr() as *mut libc::c_void,
            )
        };
        if ret < 0 {
            return false;
        }
        let byte = (keycode / 8) as usize;
        let bit = (keycode % 8) as u32;
        byte < bits.len() && ((bits[byte] >> bit) & 1) == 1
    }

    fn read_device_name(fd: i32) -> anyhow::Result<String> {
        let mut name_buf = [0u8; 256];
        let ret = unsafe {
            crate::raw_ioctl(fd, 0x80FF_4506u64, name_buf.as_mut_ptr() as *mut libc::c_void) // EVIOCGNAME(256)
        };
        if ret < 0 {
            anyhow::bail!("EVIOCGNAME ioctl failed");
        }
        let len = name_buf.iter().position(|&b| b == 0).unwrap_or(name_buf.len());
        Ok(String::from_utf8_lossy(&name_buf[..len]).to_string())
    }

    fn read_abs_info(fd: i32, axis: u16) -> anyhow::Result<AbsInfo> {
        let mut info = AbsInfo::default();
        let req = EVIOCGABS_BASE | axis as u64;
        let ret = unsafe {
            crate::raw_ioctl(fd, req, &mut info as *mut _ as *mut libc::c_void)
        };
        if ret < 0 {
            anyhow::bail!("EVIOCGABS({:#x}) failed", axis);
        }
        if info.maximum <= info.minimum {
            anyhow::bail!("axis {:#x} has degenerate range {}..{}", axis, info.minimum, info.maximum);
        }
        Ok(info)
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Tell this device the current display resolution (pixels). When set,
    /// `inject_tap`/`inject_swipe` rescale incoming (x,y) from display space
    /// into the panel's native ABS_MT_POSITION_{X,Y} range. No-op if unset
    /// or if EVIOCGABS wasn't readable at open.
    pub fn set_display_size(&mut self, width: i32, height: i32) {
        if width > 0 && height > 0 {
            self.display_size = Some((width, height));
        }
    }

    pub fn display_size(&self) -> Option<(i32, i32)> {
        self.display_size
    }

    pub fn has_abs_calibration(&self) -> bool {
        self.abs_x.is_some() && self.abs_y.is_some()
    }

    fn scale(&self, x: i32, y: i32) -> (i32, i32) {
        match (self.display_size, self.abs_x, self.abs_y) {
            (Some((dw, dh)), Some(ax), Some(ay)) if dw > 1 && dh > 1 => {
                let dx = (dw - 1).max(1);
                let dy = (dh - 1).max(1);
                let sx = ax.minimum
                    + ((x as i64 * (ax.maximum - ax.minimum) as i64) / dx as i64) as i32;
                let sy = ay.minimum
                    + ((y as i64 * (ay.maximum - ay.minimum) as i64) / dy as i64) as i32;
                (sx.clamp(ax.minimum, ax.maximum), sy.clamp(ay.minimum, ay.maximum))
            }
            _ => (x, y),
        }
    }

    fn pressure_value(&self) -> i32 {
        self.abs_pressure.map(|a| ((a.maximum - a.minimum) / 4).max(1) + a.minimum).unwrap_or(50)
    }

    fn touch_major_value(&self) -> i32 {
        self.abs_touch_major.map(|a| ((a.maximum - a.minimum) / 16).max(1) + a.minimum).unwrap_or(6)
    }

    fn next_tracking_id(&mut self) -> i32 {
        let id = self.tracking_id_next;
        self.tracking_id_next = self.tracking_id_next.wrapping_add(1);
        // Tracking IDs must be positive; 0 and -1 have special meaning on
        // some drivers (0 = uninitialised, -1 = finger up).
        if self.tracking_id_next <= 0 {
            self.tracking_id_next = 1;
        }
        id
    }

    fn write_event(&mut self, type_: u16, code: u16, value: i32) -> anyhow::Result<()> {
        let event = InputEvent {
            tv_sec: 0,
            tv_usec: 0,
            type_,
            code,
            value,
        };
        let bytes = unsafe {
            std::slice::from_raw_parts(
                &event as *const InputEvent as *const u8,
                std::mem::size_of::<InputEvent>(),
            )
        };
        self.file.write_all(bytes)?;
        self.file.flush()?;
        Ok(())
    }

    fn syn_report(&mut self) -> anyhow::Result<()> {
        self.write_event(EV_SYN, SYN_REPORT, 0)
    }

    pub fn inject_tap(&mut self, x: i32, y: i32) -> anyhow::Result<()> {
        let (sx, sy) = self.scale(x, y);
        let tracking_id = self.next_tracking_id();
        let pressure = self.pressure_value();
        let major = self.touch_major_value();

        // Finger down. ABS_MT_SLOT=0 selects the first contact slot; many
        // Qualcomm/STM drivers silently drop MT events whose slot wasn't
        // selected first. BTN_TOOL_FINGER classifies the contact as a
        // finger so the input dispatcher treats it as a tap, not a stylus.
        self.write_event(EV_ABS, ABS_MT_SLOT, 0)?;
        self.write_event(EV_ABS, ABS_MT_TRACKING_ID, tracking_id)?;
        self.write_event(EV_ABS, ABS_MT_POSITION_X, sx)?;
        self.write_event(EV_ABS, ABS_MT_POSITION_Y, sy)?;
        self.write_event(EV_ABS, ABS_MT_PRESSURE, pressure)?;
        self.write_event(EV_ABS, ABS_MT_TOUCH_MAJOR, major)?;
        self.write_event(EV_KEY, BTN_TOOL_FINGER, 1)?;
        self.write_event(EV_KEY, BTN_TOUCH, 1)?;
        self.syn_report()?;

        std::thread::sleep(std::time::Duration::from_millis(40));

        // Finger up. Tracking ID -1 on the same slot releases the contact.
        self.write_event(EV_ABS, ABS_MT_SLOT, 0)?;
        self.write_event(EV_ABS, ABS_MT_TRACKING_ID, -1)?;
        self.write_event(EV_KEY, BTN_TOUCH, 0)?;
        self.write_event(EV_KEY, BTN_TOOL_FINGER, 0)?;
        self.syn_report()?;
        Ok(())
    }

    pub fn inject_swipe(
        &mut self, x1: i32, y1: i32, x2: i32, y2: i32, duration_ms: u64,
    ) -> anyhow::Result<()> {
        let (sx1, sy1) = self.scale(x1, y1);
        let (sx2, sy2) = self.scale(x2, y2);
        let tracking_id = self.next_tracking_id();
        let pressure = self.pressure_value();
        let major = self.touch_major_value();

        let steps: i32 = 20;
        let step_delay = std::time::Duration::from_millis((duration_ms / steps as u64).max(1));

        self.write_event(EV_ABS, ABS_MT_SLOT, 0)?;
        self.write_event(EV_ABS, ABS_MT_TRACKING_ID, tracking_id)?;
        self.write_event(EV_ABS, ABS_MT_POSITION_X, sx1)?;
        self.write_event(EV_ABS, ABS_MT_POSITION_Y, sy1)?;
        self.write_event(EV_ABS, ABS_MT_PRESSURE, pressure)?;
        self.write_event(EV_ABS, ABS_MT_TOUCH_MAJOR, major)?;
        self.write_event(EV_KEY, BTN_TOOL_FINGER, 1)?;
        self.write_event(EV_KEY, BTN_TOUCH, 1)?;
        self.syn_report()?;

        for i in 1..=steps {
            let t = i as f32 / steps as f32;
            let cx = sx1 + ((sx2 - sx1) as f32 * t) as i32;
            let cy = sy1 + ((sy2 - sy1) as f32 * t) as i32;
            self.write_event(EV_ABS, ABS_MT_SLOT, 0)?;
            self.write_event(EV_ABS, ABS_MT_POSITION_X, cx)?;
            self.write_event(EV_ABS, ABS_MT_POSITION_Y, cy)?;
            self.syn_report()?;
            std::thread::sleep(step_delay);
        }

        self.write_event(EV_ABS, ABS_MT_SLOT, 0)?;
        self.write_event(EV_ABS, ABS_MT_TRACKING_ID, -1)?;
        self.write_event(EV_KEY, BTN_TOUCH, 0)?;
        self.write_event(EV_KEY, BTN_TOOL_FINGER, 0)?;
        self.syn_report()?;
        Ok(())
    }

    pub fn inject_key(&mut self, keycode: u16) -> anyhow::Result<()> {
        self.write_event(EV_KEY, keycode, 1)?; // down
        self.syn_report()?;
        std::thread::sleep(std::time::Duration::from_millis(50));
        self.write_event(EV_KEY, keycode, 0)?; // up
        self.syn_report()?;
        Ok(())
    }

    /// Resolve a symbolic key name to the linux evdev keycode(s) we'll
    /// try. Some Android devices route HOME as KEY_HOMEPAGE (172) instead
    /// of KEY_HOME (102); try both so callers don't need to care.
    /// Wait up to `timeout_ms` for an input event to arrive on this
    /// device. Returns `Ok(Some(event))` when one arrived, `Ok(None)`
    /// on timeout, `Err` on a real read failure. Used for *observation*
    /// — letting the agent see physical taps, key presses, accelerometer
    /// thresholds, etc., not just inject them.
    ///
    /// Implementation: `poll(2)` with `POLLIN`, then a single `read(2)`
    /// of one event. The kernel will block-buffer events; callers that
    /// need full event streams should call this in a loop with a small
    /// timeout.
    pub fn poll_for_event(&mut self, timeout_ms: i32) -> anyhow::Result<Option<RawInputEvent>> {
        use std::io::Read;
        let fd = self.file.as_raw_fd();
        let mut pfd = libc::pollfd {
            fd,
            events: libc::POLLIN,
            revents: 0,
        };
        let ret = unsafe { libc::poll(&mut pfd, 1, timeout_ms) };
        if ret < 0 {
            let err = std::io::Error::last_os_error();
            if err.kind() == std::io::ErrorKind::Interrupted {
                return Ok(None);
            }
            anyhow::bail!("poll failed: {err}");
        }
        if ret == 0 {
            return Ok(None);
        }
        if pfd.revents & libc::POLLIN == 0 {
            return Ok(None);
        }
        let mut buf = [0u8; std::mem::size_of::<InputEvent>()];
        let n = self.file.read(&mut buf)?;
        if n < buf.len() {
            anyhow::bail!("short read from input device: {} of {} bytes", n, buf.len());
        }
        let raw: InputEvent = unsafe { std::ptr::read(buf.as_ptr() as *const InputEvent) };
        Ok(Some(RawInputEvent {
            type_: raw.type_,
            code: raw.code,
            value: raw.value,
        }))
    }

    pub fn key_codes_for_name(name: &str) -> &'static [u16] {
        match name.to_uppercase().as_str() {
            "HOME" => &[KEY_HOMEPAGE, KEY_HOME],
            "BACK" => &[KEY_BACK],
            "POWER" => &[KEY_POWER],
            "VOLUME_UP" | "VOLUMEUP" => &[KEY_VOLUMEUP],
            "VOLUME_DOWN" | "VOLUMEDOWN" => &[KEY_VOLUMEDOWN],
            "ENTER" | "RETURN" => &[KEY_ENTER],
            _ => &[],
        }
    }

    pub fn key_code_for_name(name: &str) -> Option<u16> {
        Self::key_codes_for_name(name).first().copied()
    }
}
