pub mod input;
pub mod framebuffer;
pub mod modem;
pub mod uinput;
pub mod accessibility;
pub mod package_manager;

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

pub use input::InputDevice;
pub use framebuffer::FramebufferDevice;
pub use modem::SerialModem;
pub use uinput::UInputDevice;
pub use accessibility::{SurfaceFlingerCapture, UiHierarchy, UiNode, Bounds};
pub use package_manager::PackageManager;

#[derive(Debug, Clone)]
pub struct RgbaBuffer {
    pub data: Vec<u8>,
    pub width: u32,
    pub height: u32,
}
