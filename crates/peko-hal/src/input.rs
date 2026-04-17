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

const KEY_HOME: u16 = 102;
const KEY_BACK: u16 = 158;
const KEY_POWER: u16 = 116;
const KEY_VOLUMEUP: u16 = 115;
const KEY_VOLUMEDOWN: u16 = 114;
const KEY_ENTER: u16 = 28;

pub struct InputDevice {
    file: File,
    path: PathBuf,
    name: String,
}

impl InputDevice {
    pub fn open(path: &Path) -> anyhow::Result<Self> {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(path)?;

        let name = Self::read_device_name(file.as_raw_fd())
            .unwrap_or_else(|_| "unknown".to_string());

        Ok(Self {
            file,
            path: path.to_path_buf(),
            name,
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
                    // Check if this device supports ABS_MT (multitouch) via ioctl
                    // For emulators, prefer the first matching device
                    if fallback.is_none() {
                        fallback = Some(device);
                    }
                }
            }
        }

        if let Some(device) = fallback {
            return Ok(device);
        }

        // Last resort: try event0 (common on emulators)
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

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn path(&self) -> &Path {
        &self.path
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
        // Finger down
        self.write_event(EV_ABS, ABS_MT_TRACKING_ID, 0)?;
        self.write_event(EV_ABS, ABS_MT_POSITION_X, x)?;
        self.write_event(EV_ABS, ABS_MT_POSITION_Y, y)?;
        self.write_event(EV_ABS, ABS_MT_PRESSURE, 50)?;
        self.write_event(EV_ABS, ABS_MT_TOUCH_MAJOR, 6)?;
        self.write_event(EV_KEY, BTN_TOUCH, 1)?;
        self.syn_report()?;

        std::thread::sleep(std::time::Duration::from_millis(20));

        // Finger up
        self.write_event(EV_ABS, ABS_MT_TRACKING_ID, -1)?;
        self.write_event(EV_KEY, BTN_TOUCH, 0)?;
        self.syn_report()?;
        Ok(())
    }

    pub fn inject_swipe(
        &mut self, x1: i32, y1: i32, x2: i32, y2: i32, duration_ms: u64,
    ) -> anyhow::Result<()> {
        let steps: i32 = 20;
        let step_delay = std::time::Duration::from_millis(duration_ms / steps as u64);

        // Finger down
        self.write_event(EV_ABS, ABS_MT_TRACKING_ID, 0)?;
        self.write_event(EV_ABS, ABS_MT_POSITION_X, x1)?;
        self.write_event(EV_ABS, ABS_MT_POSITION_Y, y1)?;
        self.write_event(EV_ABS, ABS_MT_PRESSURE, 50)?;
        self.write_event(EV_ABS, ABS_MT_TOUCH_MAJOR, 6)?;
        self.write_event(EV_KEY, BTN_TOUCH, 1)?;
        self.syn_report()?;

        for i in 1..=steps {
            let t = i as f32 / steps as f32;
            let cx = x1 + ((x2 - x1) as f32 * t) as i32;
            let cy = y1 + ((y2 - y1) as f32 * t) as i32;
            self.write_event(EV_ABS, ABS_MT_POSITION_X, cx)?;
            self.write_event(EV_ABS, ABS_MT_POSITION_Y, cy)?;
            self.syn_report()?;
            std::thread::sleep(step_delay);
        }

        // Finger up
        self.write_event(EV_ABS, ABS_MT_TRACKING_ID, -1)?;
        self.write_event(EV_KEY, BTN_TOUCH, 0)?;
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

    pub fn key_code_for_name(name: &str) -> Option<u16> {
        match name.to_uppercase().as_str() {
            "HOME" => Some(KEY_HOME),
            "BACK" => Some(KEY_BACK),
            "POWER" => Some(KEY_POWER),
            "VOLUME_UP" | "VOLUMEUP" => Some(KEY_VOLUMEUP),
            "VOLUME_DOWN" | "VOLUMEDOWN" => Some(KEY_VOLUMEDOWN),
            "ENTER" | "RETURN" => Some(KEY_ENTER),
            _ => None,
        }
    }
}
