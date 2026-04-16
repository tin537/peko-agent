use std::fs::{File, OpenOptions};
use std::io::Write;
use std::os::unix::io::AsRawFd;
use std::mem;

const UINPUT_PATH: &str = "/dev/uinput";

const UI_SET_EVBIT: libc::c_ulong = 0x40045564;
const UI_SET_KEYBIT: libc::c_ulong = 0x40045565;
const UI_SET_ABSBIT: libc::c_ulong = 0x40045567;
const UI_DEV_CREATE: libc::c_ulong = 0x5501;
const UI_DEV_DESTROY: libc::c_ulong = 0x5502;
const UI_DEV_SETUP: libc::c_ulong = 0x405C5503;
const UI_ABS_SETUP: libc::c_ulong = 0x40185504;

const EV_SYN: u16 = 0x00;
const EV_KEY: u16 = 0x01;
const EV_ABS: u16 = 0x03;

const SYN_REPORT: u16 = 0x00;
const BTN_TOUCH: u16 = 0x14a;
const ABS_MT_SLOT: u16 = 0x2f;
const ABS_MT_TRACKING_ID: u16 = 0x39;
const ABS_MT_POSITION_X: u16 = 0x35;
const ABS_MT_POSITION_Y: u16 = 0x36;
const ABS_MT_PRESSURE: u16 = 0x3a;
const ABS_MT_TOUCH_MAJOR: u16 = 0x30;
const ABS_X: u16 = 0x00;
const ABS_Y: u16 = 0x01;

#[repr(C)]
struct UinputSetup {
    id: InputId,
    name: [u8; 80],
    ff_effects_max: u32,
}

#[repr(C)]
struct InputId {
    bustype: u16,
    vendor: u16,
    product: u16,
    version: u16,
}

#[repr(C)]
struct UinputAbsSetup {
    code: u16,
    _padding: u16,
    absinfo: AbsInfo,
}

#[repr(C)]
struct AbsInfo {
    value: i32,
    minimum: i32,
    maximum: i32,
    fuzz: i32,
    flat: i32,
    resolution: i32,
}

#[repr(C)]
#[derive(Default)]
struct InputEvent {
    tv_sec: libc::time_t,
    tv_usec: libc::suseconds_t,
    type_: u16,
    code: u16,
    value: i32,
}

pub struct UInputDevice {
    file: File,
    width: i32,
    height: i32,
}

impl UInputDevice {
    pub fn create_touchscreen(name: &str, width: i32, height: i32) -> anyhow::Result<Self> {
        let file = OpenOptions::new()
            .write(true)
            .open(UINPUT_PATH)?;
        let fd = file.as_raw_fd();

        unsafe {
            // Enable event types
            ioctl_val(fd, UI_SET_EVBIT, EV_SYN as libc::c_ulong)?;
            ioctl_val(fd, UI_SET_EVBIT, EV_KEY as libc::c_ulong)?;
            ioctl_val(fd, UI_SET_EVBIT, EV_ABS as libc::c_ulong)?;

            // Enable BTN_TOUCH
            ioctl_val(fd, UI_SET_KEYBIT, BTN_TOUCH as libc::c_ulong)?;

            // Enable absolute axes
            for axis in [
                ABS_X, ABS_Y, ABS_MT_SLOT, ABS_MT_TRACKING_ID,
                ABS_MT_POSITION_X, ABS_MT_POSITION_Y,
                ABS_MT_PRESSURE, ABS_MT_TOUCH_MAJOR,
            ] {
                ioctl_val(fd, UI_SET_ABSBIT, axis as libc::c_ulong)?;
            }

            // Setup absolute axes ranges
            for (code, max) in [
                (ABS_X, width - 1),
                (ABS_Y, height - 1),
                (ABS_MT_POSITION_X, width - 1),
                (ABS_MT_POSITION_Y, height - 1),
                (ABS_MT_SLOT, 9),
                (ABS_MT_TRACKING_ID, 65535),
                (ABS_MT_PRESSURE, 255),
                (ABS_MT_TOUCH_MAJOR, 255),
            ] {
                let abs_setup = UinputAbsSetup {
                    code,
                    _padding: 0,
                    absinfo: AbsInfo {
                        value: 0,
                        minimum: 0,
                        maximum: max,
                        fuzz: 0,
                        flat: 0,
                        resolution: 0,
                    },
                };
                ioctl_ptr(fd, UI_ABS_SETUP, &abs_setup as *const _ as *const libc::c_void)?;
            }

            // Device setup
            let mut setup: UinputSetup = mem::zeroed();
            setup.id = InputId {
                bustype: 0x03, // BUS_USB
                vendor: 0x1234,
                product: 0x5678,
                version: 1,
            };
            let name_bytes = name.as_bytes();
            let len = name_bytes.len().min(79);
            setup.name[..len].copy_from_slice(&name_bytes[..len]);
            ioctl_ptr(fd, UI_DEV_SETUP, &setup as *const _ as *const libc::c_void)?;

            // Create the device
            if crate::raw_ioctl_none(fd, UI_DEV_CREATE) < 0 {
                anyhow::bail!("UI_DEV_CREATE failed: {}", std::io::Error::last_os_error());
            }
        }

        // Give the kernel a moment to register the device
        std::thread::sleep(std::time::Duration::from_millis(100));

        Ok(Self { file, width, height })
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
                mem::size_of::<InputEvent>(),
            )
        };
        self.file.write_all(bytes)?;
        Ok(())
    }

    fn syn(&mut self) -> anyhow::Result<()> {
        self.write_event(EV_SYN, SYN_REPORT, 0)
    }

    pub fn tap(&mut self, x: i32, y: i32) -> anyhow::Result<()> {
        // Finger down
        self.write_event(EV_ABS, ABS_MT_TRACKING_ID, 0)?;
        self.write_event(EV_ABS, ABS_MT_POSITION_X, x)?;
        self.write_event(EV_ABS, ABS_MT_POSITION_Y, y)?;
        self.write_event(EV_ABS, ABS_MT_PRESSURE, 50)?;
        self.write_event(EV_ABS, ABS_MT_TOUCH_MAJOR, 6)?;
        self.write_event(EV_KEY, BTN_TOUCH, 1)?;
        self.syn()?;

        std::thread::sleep(std::time::Duration::from_millis(15));

        // Finger up
        self.write_event(EV_ABS, ABS_MT_TRACKING_ID, -1)?;
        self.write_event(EV_KEY, BTN_TOUCH, 0)?;
        self.syn()?;

        Ok(())
    }

    pub fn swipe(
        &mut self, x1: i32, y1: i32, x2: i32, y2: i32, duration_ms: u64,
    ) -> anyhow::Result<()> {
        let steps: i32 = 30;
        let step_delay = std::time::Duration::from_millis(duration_ms / steps as u64);

        // Finger down
        self.write_event(EV_ABS, ABS_MT_TRACKING_ID, 0)?;
        self.write_event(EV_ABS, ABS_MT_POSITION_X, x1)?;
        self.write_event(EV_ABS, ABS_MT_POSITION_Y, y1)?;
        self.write_event(EV_ABS, ABS_MT_PRESSURE, 50)?;
        self.write_event(EV_KEY, BTN_TOUCH, 1)?;
        self.syn()?;

        for i in 1..=steps {
            let t = i as f32 / steps as f32;
            let cx = x1 + ((x2 - x1) as f32 * t) as i32;
            let cy = y1 + ((y2 - y1) as f32 * t) as i32;
            self.write_event(EV_ABS, ABS_MT_POSITION_X, cx)?;
            self.write_event(EV_ABS, ABS_MT_POSITION_Y, cy)?;
            self.syn()?;
            std::thread::sleep(step_delay);
        }

        // Finger up
        self.write_event(EV_ABS, ABS_MT_TRACKING_ID, -1)?;
        self.write_event(EV_KEY, BTN_TOUCH, 0)?;
        self.syn()?;

        Ok(())
    }

    pub fn long_press(&mut self, x: i32, y: i32, duration_ms: u64) -> anyhow::Result<()> {
        self.write_event(EV_ABS, ABS_MT_TRACKING_ID, 0)?;
        self.write_event(EV_ABS, ABS_MT_POSITION_X, x)?;
        self.write_event(EV_ABS, ABS_MT_POSITION_Y, y)?;
        self.write_event(EV_ABS, ABS_MT_PRESSURE, 50)?;
        self.write_event(EV_KEY, BTN_TOUCH, 1)?;
        self.syn()?;

        std::thread::sleep(std::time::Duration::from_millis(duration_ms));

        self.write_event(EV_ABS, ABS_MT_TRACKING_ID, -1)?;
        self.write_event(EV_KEY, BTN_TOUCH, 0)?;
        self.syn()?;

        Ok(())
    }

    /// Type text by injecting individual key events
    pub fn type_text(&mut self, text: &str) -> anyhow::Result<()> {
        for ch in text.chars() {
            if let Some(keycode) = char_to_keycode(ch) {
                let needs_shift = ch.is_uppercase() || "!@#$%^&*()_+{}|:\"<>?~".contains(ch);
                if needs_shift {
                    self.write_event(EV_KEY, 42, 1)?; // KEY_LEFTSHIFT down
                    self.syn()?;
                }
                self.write_event(EV_KEY, keycode, 1)?;
                self.syn()?;
                std::thread::sleep(std::time::Duration::from_millis(5));
                self.write_event(EV_KEY, keycode, 0)?;
                self.syn()?;
                if needs_shift {
                    self.write_event(EV_KEY, 42, 0)?; // KEY_LEFTSHIFT up
                    self.syn()?;
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
        }
        Ok(())
    }

    pub fn dimensions(&self) -> (i32, i32) {
        (self.width, self.height)
    }
}

impl Drop for UInputDevice {
    fn drop(&mut self) {
        unsafe {
            crate::raw_ioctl_none(self.file.as_raw_fd(), UI_DEV_DESTROY);
        }
    }
}

unsafe fn ioctl_val(fd: i32, request: libc::c_ulong, value: libc::c_ulong) -> anyhow::Result<()> {
    if crate::raw_ioctl_val(fd, request as u64, value) < 0 {
        anyhow::bail!("ioctl 0x{:x} failed: {}", request, std::io::Error::last_os_error());
    }
    Ok(())
}

unsafe fn ioctl_ptr(fd: i32, request: libc::c_ulong, ptr: *const libc::c_void) -> anyhow::Result<()> {
    if crate::raw_ioctl(fd, request as u64, ptr as *mut libc::c_void) < 0 {
        anyhow::bail!("ioctl 0x{:x} failed: {}", request, std::io::Error::last_os_error());
    }
    Ok(())
}

fn char_to_keycode(ch: char) -> Option<u16> {
    match ch.to_lowercase().next().unwrap_or(ch) {
        'a' => Some(30), 'b' => Some(48), 'c' => Some(46), 'd' => Some(32),
        'e' => Some(18), 'f' => Some(33), 'g' => Some(34), 'h' => Some(35),
        'i' => Some(23), 'j' => Some(36), 'k' => Some(37), 'l' => Some(38),
        'm' => Some(50), 'n' => Some(49), 'o' => Some(24), 'p' => Some(25),
        'q' => Some(16), 'r' => Some(19), 's' => Some(31), 't' => Some(20),
        'u' => Some(22), 'v' => Some(47), 'w' => Some(17), 'x' => Some(45),
        'y' => Some(21), 'z' => Some(44),
        '1' | '!' => Some(2), '2' | '@' => Some(3), '3' | '#' => Some(4),
        '4' | '$' => Some(5), '5' | '%' => Some(6), '6' | '^' => Some(7),
        '7' | '&' => Some(8), '8' | '*' => Some(9), '9' | '(' => Some(10),
        '0' | ')' => Some(11),
        ' ' => Some(57),     // KEY_SPACE
        '\n' => Some(28),    // KEY_ENTER
        '\t' => Some(15),    // KEY_TAB
        '-' | '_' => Some(12),
        '=' | '+' => Some(13),
        '[' | '{' => Some(26),
        ']' | '}' => Some(27),
        '\\' | '|' => Some(43),
        ';' | ':' => Some(39),
        '\'' | '"' => Some(40),
        ',' | '<' => Some(51),
        '.' | '>' => Some(52),
        '/' | '?' => Some(53),
        '`' | '~' => Some(41),
        _ => None,
    }
}
