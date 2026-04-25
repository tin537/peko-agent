//! Write a rendered RgbaBuffer to a Linux framebuffer device.
//!
//! Lane A use case: agent renders a status overlay via `Canvas`,
//! then blits the resulting RgbaBuffer to `/dev/graphics/fb0` so
//! the user sees it on the panel. SurfaceFlinger is dead in Lane A,
//! so direct fb writes are the actual display path.
//!
//! Lane B caveat: `/dev/graphics/fb0` on sdm845 + many Qualcomm
//! kernels is a stale low-res AOD buffer (Phase 1 measured it as
//! 640x400 on the OnePlus 6T while the real display is 1080x2340).
//! Blitting to it succeeds but the pixels never reach the panel —
//! SurfaceFlinger owns the actual scanout. This is detected via
//! the `verify_dimensions` parameter and surfaced as a warning,
//! not an error.

use crate::Rgba;
use peko_hal::RgbaBuffer;
use std::fs::OpenOptions;
use std::io::{Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

#[derive(Debug, thiserror::Error)]
pub enum BlitError {
    #[error("framebuffer {path} not present")]
    NotPresent { path: PathBuf },
    #[error("io error on {path}: {err}")]
    Io {
        path: PathBuf,
        #[source]
        err: std::io::Error,
    },
    #[error("ioctl {ioctl} failed on {path}: {err}")]
    Ioctl {
        path: PathBuf,
        ioctl: &'static str,
        #[source]
        err: std::io::Error,
    },
    #[error("dimension mismatch: canvas is {cw}x{ch}, framebuffer is {fw}x{fh}")]
    DimensionMismatch {
        cw: u32,
        ch: u32,
        fw: u32,
        fh: u32,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BlitFormat {
    /// 32-bit, 4 bytes per pixel, channels in BGRA order (Android's most
    /// common scanout format).
    #[default]
    Bgra8888,
    /// 32-bit, 4 bytes per pixel, channels in RGBA order. Less common
    /// but seen on some Mali pipelines.
    Rgba8888,
    /// 16-bit, 2 bytes per pixel, RGB565. Some legacy panels and
    /// emulator targets default to this.
    Rgb565,
}

/// Information about a framebuffer device that we read once at the
/// start of a blit. Pulled here rather than via `peko-hal` to keep
/// the renderer crate's surface tight; the framebuffer ABI is small
/// and stable.
#[derive(Debug, Clone, Default)]
pub struct FbInfo {
    pub width: u32,
    pub height: u32,
    pub bytes_per_pixel: u32,
    pub line_length: u32,
    pub format: BlitFormat,
}

#[repr(C)]
#[derive(Debug, Default)]
struct FbVarScreenInfo {
    xres: u32,
    yres: u32,
    xres_virtual: u32,
    yres_virtual: u32,
    xoffset: u32,
    yoffset: u32,
    bits_per_pixel: u32,
    grayscale: u32,
    red: FbBitField,
    green: FbBitField,
    blue: FbBitField,
    transp: FbBitField,
    _padding: [u32; 14],
}

#[repr(C)]
#[derive(Debug, Default, Clone, Copy)]
struct FbBitField {
    offset: u32,
    length: u32,
    msb_right: u32,
}

#[repr(C)]
#[derive(Debug)]
struct FbFixScreenInfo {
    id: [u8; 16],
    smem_start: libc::c_ulong,
    smem_len: u32,
    type_: u32,
    type_aux: u32,
    visual: u32,
    xpanstep: u16,
    ypanstep: u16,
    ywrapstep: u16,
    line_length: u32,
    _padding: [u8; 32],
}

const FBIOGET_VSCREENINFO: u64 = 0x4600;
const FBIOGET_FSCREENINFO: u64 = 0x4602;

pub fn read_fb_info(path: &Path) -> Result<FbInfo, BlitError> {
    if !path.exists() {
        return Err(BlitError::NotPresent { path: path.to_path_buf() });
    }
    let file = OpenOptions::new()
        .read(true)
        .open(path)
        .map_err(|err| BlitError::Io { path: path.to_path_buf(), err })?;
    let fd = std::os::unix::io::AsRawFd::as_raw_fd(&file);

    let mut var: FbVarScreenInfo = unsafe { std::mem::zeroed() };
    if unsafe {
        libc::ioctl(fd, FBIOGET_VSCREENINFO as _, &mut var as *mut _)
    } < 0
    {
        return Err(BlitError::Ioctl {
            path: path.to_path_buf(),
            ioctl: "FBIOGET_VSCREENINFO",
            err: std::io::Error::last_os_error(),
        });
    }
    let mut fix: FbFixScreenInfo = unsafe { std::mem::zeroed() };
    if unsafe {
        libc::ioctl(fd, FBIOGET_FSCREENINFO as _, &mut fix as *mut _)
    } < 0
    {
        return Err(BlitError::Ioctl {
            path: path.to_path_buf(),
            ioctl: "FBIOGET_FSCREENINFO",
            err: std::io::Error::last_os_error(),
        });
    }
    let bpp = var.bits_per_pixel / 8;
    let format = detect_format(&var);
    Ok(FbInfo {
        width: var.xres,
        height: var.yres,
        bytes_per_pixel: bpp,
        line_length: fix.line_length,
        format,
    })
}

/// Decide which packed format a framebuffer reports based on the
/// FBIOGET_VSCREENINFO bitfields. Only handles the three formats we
/// care about — anything else returns Bgra8888 as a safe default that
/// will produce visually-recognisable (if maybe channel-swapped) output.
fn detect_format(var: &FbVarScreenInfo) -> BlitFormat {
    match var.bits_per_pixel {
        16 => BlitFormat::Rgb565,
        32 => {
            // Distinguish RGBA vs BGRA by red channel offset.
            // RGBA: r=0 g=8  b=16 a=24
            // BGRA: r=16 g=8 b=0  a=24
            if var.red.offset == 0 && var.blue.offset >= 16 {
                BlitFormat::Rgba8888
            } else {
                BlitFormat::Bgra8888
            }
        }
        _ => BlitFormat::Bgra8888,
    }
}

/// Convert a single RGBA8888 pixel to the framebuffer's pixel format
/// and write it into `dst` (which must be at least `bytes_per_pixel`
/// bytes long).
fn pack_pixel(rgba: Rgba, fmt: BlitFormat, dst: &mut [u8]) {
    match fmt {
        BlitFormat::Bgra8888 => {
            dst[0] = rgba.2;
            dst[1] = rgba.1;
            dst[2] = rgba.0;
            if dst.len() >= 4 {
                dst[3] = rgba.3;
            }
        }
        BlitFormat::Rgba8888 => {
            dst[0] = rgba.0;
            dst[1] = rgba.1;
            dst[2] = rgba.2;
            if dst.len() >= 4 {
                dst[3] = rgba.3;
            }
        }
        BlitFormat::Rgb565 => {
            let r = (rgba.0 as u16) >> 3;
            let g = (rgba.1 as u16) >> 2;
            let b = (rgba.2 as u16) >> 3;
            let v = (r << 11) | (g << 5) | b;
            dst[0] = (v & 0xff) as u8;
            dst[1] = (v >> 8) as u8;
        }
    }
}

/// Blit `buf` into the framebuffer at `path`. Honours the framebuffer's
/// reported `line_length` (stride) and pixel format. Returns the number
/// of bytes written.
///
/// `verify_dimensions = true` errors out when the canvas doesn't match
/// the framebuffer geometry (catches Lane B sdm845's stale 640x400 fb0).
/// Set to false for partial overlays where you only paint a sub-region.
pub fn blit_to_framebuffer(
    buf: &RgbaBuffer,
    path: &Path,
    verify_dimensions: bool,
) -> Result<usize, BlitError> {
    let info = read_fb_info(path)?;
    if verify_dimensions && (info.width != buf.width || info.height != buf.height) {
        return Err(BlitError::DimensionMismatch {
            cw: buf.width,
            ch: buf.height,
            fw: info.width,
            fh: info.height,
        });
    }
    let mut file = OpenOptions::new()
        .write(true)
        .open(path)
        .map_err(|err| BlitError::Io { path: path.to_path_buf(), err })?;

    let bpp = info.bytes_per_pixel.max(1) as usize;
    let dst_stride = info.line_length as usize;
    let src_stride = buf.width as usize * 4;
    let rows = buf.height.min(info.height) as usize;
    let cols = buf.width.min(info.width) as usize;

    let mut row_buf = vec![0u8; dst_stride.max(cols * bpp)];
    let mut written: usize = 0;

    for y in 0..rows {
        let src_off = y * src_stride;
        for x in 0..cols {
            let p = &buf.data[src_off + x * 4..src_off + x * 4 + 4];
            let pixel = Rgba(p[0], p[1], p[2], p[3]);
            pack_pixel(pixel, info.format, &mut row_buf[x * bpp..x * bpp + bpp]);
        }
        file.seek(SeekFrom::Start((y * dst_stride) as u64))
            .map_err(|err| BlitError::Io { path: path.to_path_buf(), err })?;
        let to_write = &row_buf[..(cols * bpp)];
        file.write_all(to_write)
            .map_err(|err| BlitError::Io { path: path.to_path_buf(), err })?;
        written += to_write.len();
    }
    Ok(written)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pack_pixel_bgra() {
        let mut dst = [0u8; 4];
        pack_pixel(Rgba(10, 20, 30, 40), BlitFormat::Bgra8888, &mut dst);
        assert_eq!(dst, [30, 20, 10, 40]);
    }

    #[test]
    fn pack_pixel_rgba() {
        let mut dst = [0u8; 4];
        pack_pixel(Rgba(10, 20, 30, 40), BlitFormat::Rgba8888, &mut dst);
        assert_eq!(dst, [10, 20, 30, 40]);
    }

    #[test]
    fn pack_pixel_rgb565() {
        let mut dst = [0u8; 2];
        pack_pixel(Rgba(255, 255, 255, 255), BlitFormat::Rgb565, &mut dst);
        // 0xFFFF in little-endian.
        assert_eq!(dst, [0xFF, 0xFF]);
        let mut dst = [0u8; 2];
        pack_pixel(Rgba(0, 0, 0, 0), BlitFormat::Rgb565, &mut dst);
        assert_eq!(dst, [0x00, 0x00]);
        // Pure red: 0xF800 LE = [0x00, 0xF8]
        let mut dst = [0u8; 2];
        pack_pixel(Rgba(255, 0, 0, 0), BlitFormat::Rgb565, &mut dst);
        assert_eq!(dst, [0x00, 0xF8]);
    }

    #[test]
    fn detect_format_uses_red_offset() {
        let mut var = FbVarScreenInfo::default();
        var.bits_per_pixel = 32;
        var.red.offset = 16;
        var.blue.offset = 0;
        assert_eq!(detect_format(&var), BlitFormat::Bgra8888);
        var.red.offset = 0;
        var.blue.offset = 16;
        assert_eq!(detect_format(&var), BlitFormat::Rgba8888);
        var.bits_per_pixel = 16;
        assert_eq!(detect_format(&var), BlitFormat::Rgb565);
    }

    #[test]
    fn read_fb_info_returns_not_present_when_missing() {
        let r = read_fb_info(Path::new("/nonexistent/fb0"));
        match r {
            Err(BlitError::NotPresent { .. }) => {}
            other => panic!("expected NotPresent, got {other:?}"),
        }
    }
}
