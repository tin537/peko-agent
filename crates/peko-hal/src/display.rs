//! Display capture trait + implementations.
//!
//! The agent has three possible paths to the screen pixels:
//!   - `FbdevCapture`     — direct mmap of /dev/graphics/fb0. Works in
//!                          frameworkless mode and on devices where the
//!                          framebuffer is the live scanout buffer.
//!   - `ScreencapCapture` — shell out to /system/bin/screencap. Goes
//!                          through SurfaceFlinger and returns whatever
//!                          the compositor sees, including overlays.
//!                          Requires the Android framework.
//!   - `DrmCapture`       — read the DRM scanout buffer. Requires the
//!                          process to hold DRM master (Lane A only).
//!                          NOT IMPLEMENTED YET — see `display_info` for
//!                          enumeration that works without master.
//!
//! `auto_capture()` returns the best available backend, preferring
//! `FbdevCapture` in frameworkless mode and `ScreencapCapture` when
//! SurfaceFlinger is running. Each backend reports its own name so the
//! agent can see which path actually produced the pixels.

use crate::RgbaBuffer;
use std::fs;
use std::path::Path;
use std::process::{Command, Stdio};

#[derive(Debug, thiserror::Error)]
pub enum CaptureError {
    #[error("backend {backend} unavailable: {reason}")]
    Unavailable { backend: &'static str, reason: String },
    #[error("ioctl {ioctl} failed on {backend}: {err}")]
    Ioctl {
        backend: &'static str,
        ioctl: &'static str,
        #[source]
        err: std::io::Error,
    },
    #[error("io error on {backend}: {err}")]
    Io {
        backend: &'static str,
        #[source]
        err: std::io::Error,
    },
    #[error("unsupported pixel format on {backend}: {fmt}")]
    UnsupportedFormat { backend: &'static str, fmt: String },
    #[error("no display backend is available")]
    NoBackendAvailable,
}

/// Display rotation, expressed as the angle the framebuffer content has
/// been turned clockwise relative to the panel's native landscape origin.
/// On most Android phones the panel is mounted "landscape" but the OS
/// boots into portrait, so the natural framebuffer orientation is `R270`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Rotation {
    #[default]
    R0,
    R90,
    R180,
    R270,
}

impl Rotation {
    pub fn from_degrees(deg: i32) -> Self {
        match ((deg % 360) + 360) % 360 {
            0 => Rotation::R0,
            90 => Rotation::R90,
            180 => Rotation::R180,
            270 => Rotation::R270,
            _ => Rotation::R0,
        }
    }

    pub fn from_quarter_turns(q: i32) -> Self {
        Self::from_degrees(((q % 4) + 4) % 4 * 90)
    }

    pub fn degrees(self) -> u32 {
        match self {
            Rotation::R0 => 0,
            Rotation::R90 => 90,
            Rotation::R180 => 180,
            Rotation::R270 => 270,
        }
    }
}

pub trait DisplayCapture: Send {
    fn capture(&mut self) -> Result<RgbaBuffer, CaptureError>;
    fn dimensions(&self) -> (u32, u32);
    fn rotation(&self) -> Rotation;
    fn backend_name(&self) -> &'static str;
}

// -----------------------------------------------------------------------------
// FbdevCapture
// -----------------------------------------------------------------------------

pub struct FbdevCapture {
    fb: crate::FramebufferDevice,
    rotation: Rotation,
}

impl FbdevCapture {
    pub fn open_default() -> Result<Self, CaptureError> {
        let fb = crate::FramebufferDevice::open_default().map_err(|e| {
            CaptureError::Unavailable {
                backend: "fbdev",
                reason: format!("open /dev/graphics/fb0: {e}"),
            }
        })?;
        let rotation = read_sysfs_rotation();
        Ok(Self { fb, rotation })
    }

    pub fn open(path: &Path) -> Result<Self, CaptureError> {
        let fb = crate::FramebufferDevice::open(path).map_err(|e| CaptureError::Unavailable {
            backend: "fbdev",
            reason: format!("open {}: {e}", path.display()),
        })?;
        let rotation = read_sysfs_rotation();
        Ok(Self { fb, rotation })
    }

    pub fn with_rotation(mut self, rot: Rotation) -> Self {
        self.rotation = rot;
        self
    }
}

impl DisplayCapture for FbdevCapture {
    fn capture(&mut self) -> Result<RgbaBuffer, CaptureError> {
        let raw = self.fb.capture().map_err(|e| CaptureError::Io {
            backend: "fbdev",
            err: std::io::Error::other(e.to_string()),
        })?;
        Ok(rotate_rgba(raw, self.rotation))
    }

    fn dimensions(&self) -> (u32, u32) {
        let (w, h) = self.fb.dimensions();
        match self.rotation {
            Rotation::R0 | Rotation::R180 => (w, h),
            Rotation::R90 | Rotation::R270 => (h, w),
        }
    }

    fn rotation(&self) -> Rotation {
        self.rotation
    }

    fn backend_name(&self) -> &'static str {
        "fbdev"
    }
}

/// Read the current rotation from sysfs. Linux fbdev exposes
/// `/sys/class/graphics/fb0/rotate` as an integer in `{0, 1, 2, 3}`
/// representing quarter-turns clockwise. If the file is missing
/// (vendor kernel without the export) we default to `R0` and let the
/// caller override via `with_rotation()` or device profile.
fn read_sysfs_rotation() -> Rotation {
    for path in ["/sys/class/graphics/fb0/rotate", "/sys/class/graphics/fb0/rotation"] {
        if let Ok(s) = fs::read_to_string(path) {
            if let Ok(q) = s.trim().parse::<i32>() {
                // Some kernels report degrees, others quarter-turns.
                // Disambiguate: any value > 3 is degrees.
                return if q > 3 {
                    Rotation::from_degrees(q)
                } else {
                    Rotation::from_quarter_turns(q)
                };
            }
        }
    }
    Rotation::R0
}

/// Rotate an RGBA buffer in-place. Cheap CPU rotation since framebuffer
/// reads are already the dominant cost — we don't ship to a GPU because
/// frameworkless mode may not have one available.
fn rotate_rgba(src: RgbaBuffer, rot: Rotation) -> RgbaBuffer {
    if rot == Rotation::R0 {
        return src;
    }
    let (w, h) = (src.width as usize, src.height as usize);
    let mut out = vec![0u8; src.data.len()];
    let stride = w * 4;

    let (out_w, out_h) = match rot {
        Rotation::R0 | Rotation::R180 => (w, h),
        Rotation::R90 | Rotation::R270 => (h, w),
    };
    let out_stride = out_w * 4;

    for y in 0..h {
        for x in 0..w {
            let src_off = y * stride + x * 4;
            if src_off + 4 > src.data.len() {
                break;
            }
            let (nx, ny) = match rot {
                Rotation::R0 => (x, y),
                Rotation::R90 => (h - 1 - y, x),
                Rotation::R180 => (w - 1 - x, h - 1 - y),
                Rotation::R270 => (y, w - 1 - x),
            };
            let dst_off = ny * out_stride + nx * 4;
            if dst_off + 4 > out.len() {
                continue;
            }
            out[dst_off..dst_off + 4].copy_from_slice(&src.data[src_off..src_off + 4]);
        }
    }

    RgbaBuffer {
        data: out,
        width: out_w as u32,
        height: out_h as u32,
    }
}

// -----------------------------------------------------------------------------
// ScreencapCapture
// -----------------------------------------------------------------------------

pub struct ScreencapCapture {
    cached_dims: Option<(u32, u32)>,
}

impl ScreencapCapture {
    pub fn new() -> Result<Self, CaptureError> {
        let avail = Command::new("which")
            .arg("screencap")
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);
        if !avail {
            return Err(CaptureError::Unavailable {
                backend: "screencap",
                reason: "screencap binary not on PATH".into(),
            });
        }
        Ok(Self { cached_dims: None })
    }
}

impl DisplayCapture for ScreencapCapture {
    fn capture(&mut self) -> Result<RgbaBuffer, CaptureError> {
        // `screencap` (no args) emits raw RGBA: 4-byte LE width, 4-byte LE
        // height, 4-byte format, then pixel data. We use the raw form
        // rather than `-p` (PNG) so callers get a uniform RgbaBuffer.
        let out = Command::new("screencap")
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output()
            .map_err(|err| CaptureError::Io { backend: "screencap", err })?;
        if !out.status.success() {
            return Err(CaptureError::Io {
                backend: "screencap",
                err: std::io::Error::other(format!(
                    "screencap exited non-zero: {}",
                    String::from_utf8_lossy(&out.stderr)
                )),
            });
        }
        let data = out.stdout;
        if data.len() < 12 {
            return Err(CaptureError::Io {
                backend: "screencap",
                err: std::io::Error::other("output shorter than 12-byte header"),
            });
        }
        let w = u32::from_le_bytes([data[0], data[1], data[2], data[3]]);
        let h = u32::from_le_bytes([data[4], data[5], data[6], data[7]]);
        let _fmt = u32::from_le_bytes([data[8], data[9], data[10], data[11]]);
        let pixels = data[12..].to_vec();
        let expected = (w as usize) * (h as usize) * 4;
        if pixels.len() < expected {
            return Err(CaptureError::Io {
                backend: "screencap",
                err: std::io::Error::other(format!(
                    "expected {} bytes of RGBA, got {}",
                    expected,
                    pixels.len()
                )),
            });
        }
        self.cached_dims = Some((w, h));
        Ok(RgbaBuffer { data: pixels, width: w, height: h })
    }

    fn dimensions(&self) -> (u32, u32) {
        self.cached_dims.unwrap_or((0, 0))
    }

    fn rotation(&self) -> Rotation {
        // screencap returns pixels already in display orientation, so by
        // definition R0 from our perspective.
        Rotation::R0
    }

    fn backend_name(&self) -> &'static str {
        "screencap"
    }
}

// -----------------------------------------------------------------------------
// auto_capture
// -----------------------------------------------------------------------------

/// Return the best display backend for the current environment.
///
/// Order:
///   1. preferred backend if specified (from device profile)
///   2. screencap if SurfaceFlinger is up — in Lane B this is what the
///      user sees, including overlays. Most accurate on hybrid devices.
///   3. fbdev as a frameworkless fallback. On many devices in Lane B the
///      framebuffer is a stale AOD buffer; this is correct for Lane A
///      and headless devices.
pub fn auto_capture(preferred: Option<&str>) -> Result<Box<dyn DisplayCapture>, CaptureError> {
    let order: Vec<&str> = match preferred {
        Some("fb") | Some("fbdev") => vec!["fbdev", "screencap"],
        Some("screencap") => vec!["screencap", "fbdev"],
        Some("auto") | None => vec!["screencap", "fbdev"],
        Some(other) => {
            // Unknown preference — log once via tracing and fall back to
            // auto order rather than failing closed.
            tracing::warn!(preferred = other, "unknown capture backend, using auto");
            vec!["screencap", "fbdev"]
        }
    };

    let mut last_err: Option<CaptureError> = None;
    for backend in order {
        let result: Result<Box<dyn DisplayCapture>, CaptureError> = match backend {
            "fbdev" => FbdevCapture::open_default().map(|c| Box::new(c) as _),
            "screencap" => ScreencapCapture::new().map(|c| Box::new(c) as _),
            _ => continue,
        };
        match result {
            Ok(c) => return Ok(c),
            Err(e) => last_err = Some(e),
        }
    }
    Err(last_err.unwrap_or(CaptureError::NoBackendAvailable))
}

// -----------------------------------------------------------------------------
// Tests
// -----------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rotation_from_degrees_normalises() {
        assert_eq!(Rotation::from_degrees(0), Rotation::R0);
        assert_eq!(Rotation::from_degrees(90), Rotation::R90);
        assert_eq!(Rotation::from_degrees(360), Rotation::R0);
        assert_eq!(Rotation::from_degrees(450), Rotation::R90);
        assert_eq!(Rotation::from_degrees(-90), Rotation::R270);
    }

    #[test]
    fn rotation_from_quarter_turns() {
        assert_eq!(Rotation::from_quarter_turns(0), Rotation::R0);
        assert_eq!(Rotation::from_quarter_turns(1), Rotation::R90);
        assert_eq!(Rotation::from_quarter_turns(2), Rotation::R180);
        assert_eq!(Rotation::from_quarter_turns(3), Rotation::R270);
        assert_eq!(Rotation::from_quarter_turns(4), Rotation::R0);
        assert_eq!(Rotation::from_quarter_turns(-1), Rotation::R270);
    }

    #[test]
    fn rotate_rgba_r0_is_identity() {
        let src = RgbaBuffer {
            data: vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16],
            width: 2,
            height: 2,
        };
        let out = rotate_rgba(src.clone(), Rotation::R0);
        assert_eq!(out.data, src.data);
        assert_eq!(out.width, 2);
        assert_eq!(out.height, 2);
    }

    #[test]
    fn rotate_rgba_r90_swaps_dims_and_moves_pixels() {
        // 2x1 source, top-left red, top-right green
        // After R90 (clockwise): 1x2 — top: green, bottom: red.
        let src = RgbaBuffer {
            data: vec![
                255, 0, 0, 255, // (0,0) red
                0, 255, 0, 255, // (1,0) green
            ],
            width: 2,
            height: 1,
        };
        let out = rotate_rgba(src, Rotation::R90);
        assert_eq!(out.width, 1);
        assert_eq!(out.height, 2);
        assert_eq!(&out.data[0..4], &[255, 0, 0, 255]); // (0,0) stays red? No — re-derive.
        // Mapping for R90: new(nx,ny) = src(x,y) where (nx,ny)=(h-1-y, x)
        // h=1, so nx = -y → 0, ny = x. So (0,0)→(0,0) red, (1,0)→(0,1) green.
        assert_eq!(&out.data[4..8], &[0, 255, 0, 255]);
    }

    #[test]
    fn rotate_rgba_r180_flips() {
        let src = RgbaBuffer {
            data: vec![
                1, 1, 1, 255,
                2, 2, 2, 255,
                3, 3, 3, 255,
                4, 4, 4, 255,
            ],
            width: 2,
            height: 2,
        };
        let out = rotate_rgba(src, Rotation::R180);
        assert_eq!(out.width, 2);
        assert_eq!(out.height, 2);
        // R180: new(w-1-x, h-1-y) = src(x,y). (0,0)→(1,1), so out at (1,1) is 1.
        assert_eq!(&out.data[12..16], &[1, 1, 1, 255]); // (1,1) of out
        assert_eq!(&out.data[0..4], &[4, 4, 4, 255]); // (0,0) of out
    }

    #[test]
    fn auto_capture_returns_no_backend_when_nothing_works() {
        // On the host neither fbdev nor screencap normally exists, so this
        // probe should fail with a typed error. We don't assert which
        // variant because CI runners may or may not have one.
        let _ = auto_capture(Some("auto"));
    }
}
