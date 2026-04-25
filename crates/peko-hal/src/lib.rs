pub mod input;
pub mod framebuffer;
pub mod modem;
pub mod uinput;
pub mod accessibility;
pub mod package_manager;
pub mod display;
pub mod display_info;
pub mod sensors;
pub mod sensors_dumpsys;
pub mod battery;
pub mod light_prox;

/// Portable ioctl request type — `c_ulong` on macOS, `c_int` on Linux/Android.
#[cfg(target_os = "macos")]
pub(crate) type IoctlRequest = libc::c_ulong;
#[cfg(not(target_os = "macos"))]
pub(crate) type IoctlRequest = libc::c_int;

/// Safe-ish ioctl wrapper that handles platform type differences.
pub(crate) unsafe fn raw_ioctl(fd: i32, request: u64, arg: *mut libc::c_void) -> i32 {
    libc::ioctl(fd, request as IoctlRequest, arg)
}

pub(crate) unsafe fn raw_ioctl_val(fd: i32, request: u64, val: libc::c_ulong) -> i32 {
    libc::ioctl(fd, request as IoctlRequest, val)
}

pub(crate) unsafe fn raw_ioctl_none(fd: i32, request: u64) -> i32 {
    libc::ioctl(fd, request as IoctlRequest)
}

pub use input::{InputDevice, RawInputEvent};
pub use framebuffer::FramebufferDevice;
pub use modem::SerialModem;
pub use uinput::UInputDevice;
pub use accessibility::{SurfaceFlingerCapture, UiHierarchy, UiNode, Bounds};
pub use package_manager::PackageManager;
pub use display::{
    auto_capture, CaptureError, DisplayCapture, FbdevCapture, Rotation, ScreencapCapture,
};
pub use display_info::{probe as probe_drm, probe_default as probe_drm_default, DrmConnector,
    DrmInfo, DrmInfoError, DrmMode};
pub use sensors::{IioSensor, ScalarSample, SensorError, SensorKind, Vec3};
pub use sensors_dumpsys::{
    capture as capture_dumpsys_sensorservice, latest_for_type as dumpsys_latest_for_type,
    parse_recent_events as parse_dumpsys_recent_events, parse_sensor_list as parse_dumpsys_sensors,
    DumpsysError, DumpsysReading, DumpsysSensor,
};
pub use battery::{read as read_battery, BatteryError, BatteryHealth, BatteryState, ChargeStatus};
pub use light_prox::{read as read_light_prox, LightProxError,
    Reading as LightProxReading, SensorKind as LightProxKind};

#[derive(Debug, Clone)]
pub struct RgbaBuffer {
    pub data: Vec<u8>,
    pub width: u32,
    pub height: u32,
}
