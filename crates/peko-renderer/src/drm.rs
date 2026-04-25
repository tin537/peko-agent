//! DRM master + dumb buffer + scanout for Lane A on Qualcomm SoCs.
//!
//! On sdm845-class devices the framebuffer node `/dev/graphics/fb0`
//! is a phantom AOD plane (Phase 8 finding); the live scanout lives
//! exclusively in DRM. To paint pixels on the panel without
//! SurfaceFlinger we need the full DRM modesetting dance:
//!
//!   1. open `/dev/dri/card0`
//!   2. `DRM_IOCTL_SET_MASTER`           — claim the device
//!   3. `DRM_IOCTL_MODE_GETRESOURCES`    — enumerate crtcs/connectors
//!   4. pick the first connected connector + its preferred mode
//!   5. `DRM_IOCTL_MODE_CREATE_DUMB`     — kernel allocates a buffer
//!   6. `DRM_IOCTL_MODE_MAP_DUMB`        — get the mmap offset
//!   7. mmap(2)                           — write our pixels in
//!   8. `DRM_IOCTL_MODE_ADDFB2`          — register as a framebuffer
//!   9. `DRM_IOCTL_MODE_SETCRTC`         — scan out on the connector
//!  10. (hold for the desired duration)
//!  11. restore previous CRTC, RMFB, DESTROY_DUMB, DROP_MASTER, close
//!
//! `SET_MASTER` fails with EBUSY when SurfaceFlinger holds master,
//! which is the entire framework. So this code is only meaningful in
//! Lane A (frameworkless boot) or with SurfaceFlinger stopped — the
//! latter is unsafe on sdm845 (Phase 8 finding), so verification of
//! the paint path is gated behind explicit operator opt-in.
//!
//! Zero external deps beyond libc. Hand-encoded ioctl numbers + struct
//! layouts so we never silently fall out of sync with vendor headers.

use crate::Rgba;
use peko_hal::RgbaBuffer;
use std::fs::OpenOptions;
use std::os::unix::io::AsRawFd;
use std::path::{Path, PathBuf};

#[derive(Debug, thiserror::Error)]
pub enum DrmBlitError {
    #[error("DRM device {path} not present")]
    NotPresent { path: PathBuf },
    #[error("io error opening {path}: {err}")]
    Open {
        path: PathBuf,
        #[source]
        err: std::io::Error,
    },
    #[error("ioctl {ioctl} failed: {err}")]
    Ioctl {
        ioctl: &'static str,
        #[source]
        err: std::io::Error,
    },
    #[error("no connected DRM connector with a usable mode")]
    NoConnector,
    #[error("no encoder/CRTC reachable for the chosen connector")]
    NoCrtc,
    #[error("mmap failed: {0}")]
    Mmap(std::io::Error),
    #[error("dimension mismatch: canvas is {cw}x{ch}, mode is {mw}x{mh}")]
    DimensionMismatch {
        cw: u32,
        ch: u32,
        mw: u32,
        mh: u32,
    },
}

// -----------------------------------------------------------------------------
// ioctl numbers — encoded once, sanity-checked in unit tests.
// -----------------------------------------------------------------------------
//
// Linux ioctl encoding: (dir << 30) | (size << 16) | (type << 8) | nr
//   dir: 0 = none, 1 = write, 2 = read, 3 = read/write
//   type: 'd' = 0x64 for DRM
//
// The constants below match the values in linux/drm.h + linux/drm_mode.h
// at the time of writing. They have not changed in any kernel release
// since the dumb-buffer ABI stabilised in 2010.

const DRM_IOCTL_SET_MASTER: u64 = 0x0000_641e; // _IO('d', 0x1e)
const DRM_IOCTL_DROP_MASTER: u64 = 0x0000_641f; // _IO('d', 0x1f)

// MODE_GETRESOURCES is also defined in display_info.rs; we re-declare
// here because that module's constant is private. Kept in lockstep.
const DRM_IOCTL_MODE_GETRESOURCES: u64 = 0xC040_64A0;
const DRM_IOCTL_MODE_GETCONNECTOR: u64 = 0xC050_64A7;
const DRM_IOCTL_MODE_GETENCODER: u64 = 0xC014_64A6;

// _IOWR('d', 0xb2, sizeof(drm_mode_create_dumb)=32)
const DRM_IOCTL_MODE_CREATE_DUMB: u64 = 0xC020_64B2;
// _IOWR('d', 0xb3, sizeof(drm_mode_map_dumb)=16)
const DRM_IOCTL_MODE_MAP_DUMB: u64 = 0xC010_64B3;
// _IOWR('d', 0xb4, sizeof(drm_mode_destroy_dumb)=4)
const DRM_IOCTL_MODE_DESTROY_DUMB: u64 = 0xC004_64B4;
// _IOWR('d', 0xb8, sizeof(drm_mode_fb_cmd2)=104)
const DRM_IOCTL_MODE_ADDFB2: u64 = 0xC068_64B8;
// _IOWR('d', 0xaf, sizeof(uint)=4)
const DRM_IOCTL_MODE_RMFB: u64 = 0xC004_64AF;
// _IOWR('d', 0xa2, sizeof(drm_mode_crtc)=104)
const DRM_IOCTL_MODE_SETCRTC: u64 = 0xC068_64A2;

// DRM_FORMAT_* values — fourcc-style packed into a u32 little-endian.
// XR24 = "XR24" = 'X','R','2','4' = 0x34325258 — XRGB8888, alpha ignored.
const DRM_FORMAT_XRGB8888: u32 = 0x3432_5258;

// -----------------------------------------------------------------------------
// Struct layouts — must match Linux uAPI exactly.
// -----------------------------------------------------------------------------

#[repr(C)]
#[derive(Default)]
struct DrmModeCardRes {
    fb_id_ptr: u64,
    crtc_id_ptr: u64,
    connector_id_ptr: u64,
    encoder_id_ptr: u64,
    count_fbs: u32,
    count_crtcs: u32,
    count_connectors: u32,
    count_encoders: u32,
    min_width: u32,
    max_width: u32,
    min_height: u32,
    max_height: u32,
}

#[repr(C)]
#[derive(Default)]
struct DrmModeGetConnector {
    encoders_ptr: u64,
    modes_ptr: u64,
    props_ptr: u64,
    prop_values_ptr: u64,
    count_modes: u32,
    count_props: u32,
    count_encoders: u32,
    encoder_id: u32,
    connector_id: u32,
    connector_type: u32,
    connector_type_id: u32,
    connection: u32,
    mm_width: u32,
    mm_height: u32,
    subpixel: u32,
    pad: u32,
}

#[repr(C)]
#[derive(Default, Clone, Copy)]
struct DrmModeInfo {
    clock: u32,
    hdisplay: u16,
    hsync_start: u16,
    hsync_end: u16,
    htotal: u16,
    hskew: u16,
    vdisplay: u16,
    vsync_start: u16,
    vsync_end: u16,
    vtotal: u16,
    vscan: u16,
    vrefresh: u32,
    flags: u32,
    type_: u32,
    name: [u8; 32],
}

#[repr(C)]
#[derive(Default)]
struct DrmModeGetEncoder {
    encoder_id: u32,
    encoder_type: u32,
    crtc_id: u32,
    possible_crtcs: u32,
    possible_clones: u32,
}

#[repr(C)]
#[derive(Default)]
struct DrmModeCreateDumb {
    height: u32,
    width: u32,
    bpp: u32,
    flags: u32,
    handle: u32,
    pitch: u32,
    size: u64,
}

#[repr(C)]
#[derive(Default)]
struct DrmModeMapDumb {
    handle: u32,
    pad: u32,
    offset: u64,
}

#[repr(C)]
#[derive(Default)]
struct DrmModeDestroyDumb {
    handle: u32,
}

#[repr(C)]
struct DrmModeFbCmd2 {
    fb_id: u32,
    width: u32,
    height: u32,
    pixel_format: u32,
    flags: u32,
    handles: [u32; 4],
    pitches: [u32; 4],
    offsets: [u32; 4],
    modifier: [u64; 4],
}

#[repr(C)]
struct DrmModeCrtc {
    set_connectors_ptr: u64,
    count_connectors: u32,
    crtc_id: u32,
    fb_id: u32,
    x: u32,
    y: u32,
    gamma_size: u32,
    mode_valid: u32,
    mode: DrmModeInfo,
}

// -----------------------------------------------------------------------------
// Public types
// -----------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct DrmTargetInfo {
    pub device_path: PathBuf,
    pub connector_id: u32,
    pub encoder_id: u32,
    pub crtc_id: u32,
    pub width: u32,
    pub height: u32,
    pub refresh_hz: u32,
    pub mode_name: String,
}

/// Enumerate the first connected connector with a usable mode, plus
/// the encoder / CRTC pair that drives it. Does NOT acquire master,
/// safe to call while SurfaceFlinger is running.
pub fn pick_target(path: &Path) -> Result<DrmTargetInfo, DrmBlitError> {
    if !path.exists() {
        return Err(DrmBlitError::NotPresent { path: path.to_path_buf() });
    }
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .map_err(|err| DrmBlitError::Open { path: path.to_path_buf(), err })?;
    let fd = file.as_raw_fd();

    // 1) Get resource counts.
    let mut res = DrmModeCardRes::default();
    ioctl(fd, DRM_IOCTL_MODE_GETRESOURCES, &mut res, "MODE_GETRESOURCES")?;
    if res.count_connectors == 0 || res.count_crtcs == 0 || res.count_encoders == 0 {
        return Err(DrmBlitError::NoConnector);
    }
    let mut connector_ids = vec![0u32; res.count_connectors as usize];
    let mut encoder_ids = vec![0u32; res.count_encoders as usize];
    let mut crtc_ids = vec![0u32; res.count_crtcs as usize];
    let mut fb_ids = vec![0u32; res.count_fbs as usize];
    res.connector_id_ptr = connector_ids.as_mut_ptr() as u64;
    res.encoder_id_ptr = encoder_ids.as_mut_ptr() as u64;
    res.crtc_id_ptr = crtc_ids.as_mut_ptr() as u64;
    res.fb_id_ptr = fb_ids.as_mut_ptr() as u64;
    ioctl(fd, DRM_IOCTL_MODE_GETRESOURCES, &mut res, "MODE_GETRESOURCES (2)")?;

    // 2) Find the first connected connector with at least one mode.
    for &conn_id in &connector_ids {
        let mut conn = DrmModeGetConnector {
            connector_id: conn_id,
            ..Default::default()
        };
        if ioctl(fd, DRM_IOCTL_MODE_GETCONNECTOR, &mut conn, "MODE_GETCONNECTOR (probe)").is_err() {
            continue;
        }
        if conn.connection != 1 || conn.count_modes == 0 || conn.count_encoders == 0 {
            continue;
        }
        let mut modes = vec![DrmModeInfo::default(); conn.count_modes as usize];
        let mut encoders = vec![0u32; conn.count_encoders as usize];
        let mut props = vec![0u32; conn.count_props as usize];
        let mut propv = vec![0u64; conn.count_props as usize];
        conn.modes_ptr = modes.as_mut_ptr() as u64;
        conn.encoders_ptr = encoders.as_mut_ptr() as u64;
        conn.props_ptr = props.as_mut_ptr() as u64;
        conn.prop_values_ptr = propv.as_mut_ptr() as u64;
        if ioctl(fd, DRM_IOCTL_MODE_GETCONNECTOR, &mut conn, "MODE_GETCONNECTOR (full)").is_err() {
            continue;
        }
        // Pick the first encoder that has a CRTC reachable.
        for &enc_id in &encoders {
            let mut enc = DrmModeGetEncoder {
                encoder_id: enc_id,
                ..Default::default()
            };
            if ioctl(fd, DRM_IOCTL_MODE_GETENCODER, &mut enc, "MODE_GETENCODER").is_err() {
                continue;
            }
            // possible_crtcs is a bitmask of indices into res.crtc_id_ptr.
            let mut chosen_crtc = enc.crtc_id;
            if chosen_crtc == 0 {
                for (i, &cid) in crtc_ids.iter().enumerate() {
                    if (enc.possible_crtcs >> i) & 1 == 1 {
                        chosen_crtc = cid;
                        break;
                    }
                }
            }
            if chosen_crtc == 0 {
                continue;
            }
            let preferred = &modes[0];
            let name = std::str::from_utf8(&preferred.name)
                .unwrap_or("?")
                .trim_end_matches('\0')
                .to_string();
            return Ok(DrmTargetInfo {
                device_path: path.to_path_buf(),
                connector_id: conn.connector_id,
                encoder_id: enc.encoder_id,
                crtc_id: chosen_crtc,
                width: preferred.hdisplay as u32,
                height: preferred.vdisplay as u32,
                refresh_hz: preferred.vrefresh,
                mode_name: name,
            });
        }
    }

    Err(DrmBlitError::NoConnector)
}

/// Acquire DRM master, allocate a dumb buffer matching the connector's
/// mode, write the canvas into it, register as a framebuffer, scan it
/// out for `hold_ms` milliseconds, then tear everything down.
///
/// Returns `Ok(bytes_written)` on success.
///
/// **Hard prerequisite:** SurfaceFlinger must NOT hold DRM master.
/// Otherwise `SET_MASTER` returns EBUSY immediately. Run from a Lane A
/// boot, or — at your own risk — with SurfaceFlinger stopped.
pub fn paint_to_panel(
    target: &DrmTargetInfo,
    canvas: &RgbaBuffer,
    hold_ms: u64,
) -> Result<usize, DrmBlitError> {
    if canvas.width != target.width || canvas.height != target.height {
        return Err(DrmBlitError::DimensionMismatch {
            cw: canvas.width,
            ch: canvas.height,
            mw: target.width,
            mh: target.height,
        });
    }

    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(&target.device_path)
        .map_err(|err| DrmBlitError::Open { path: target.device_path.clone(), err })?;
    let fd = file.as_raw_fd();

    // 1) Acquire master. EBUSY here means SurfaceFlinger holds it.
    if unsafe { libc::ioctl(fd, DRM_IOCTL_SET_MASTER as _) } < 0 {
        return Err(DrmBlitError::Ioctl {
            ioctl: "SET_MASTER",
            err: std::io::Error::last_os_error(),
        });
    }

    // Drop master + free resources on every exit path.
    let mut guard = MasterGuard::new(fd);

    // 2) Re-fetch the connector to copy its preferred mode struct verbatim.
    let mut conn = DrmModeGetConnector {
        connector_id: target.connector_id,
        ..Default::default()
    };
    ioctl(fd, DRM_IOCTL_MODE_GETCONNECTOR, &mut conn, "MODE_GETCONNECTOR")?;
    if conn.count_modes == 0 {
        return Err(DrmBlitError::NoConnector);
    }
    let mut modes = vec![DrmModeInfo::default(); conn.count_modes as usize];
    let mut encs = vec![0u32; conn.count_encoders as usize];
    let mut props = vec![0u32; conn.count_props as usize];
    let mut propv = vec![0u64; conn.count_props as usize];
    conn.modes_ptr = modes.as_mut_ptr() as u64;
    conn.encoders_ptr = encs.as_mut_ptr() as u64;
    conn.props_ptr = props.as_mut_ptr() as u64;
    conn.prop_values_ptr = propv.as_mut_ptr() as u64;
    ioctl(fd, DRM_IOCTL_MODE_GETCONNECTOR, &mut conn, "MODE_GETCONNECTOR (full)")?;
    let mode = modes[0];

    // 3) Create a dumb buffer at 32 bpp (XRGB8888).
    let mut dumb = DrmModeCreateDumb {
        width: target.width,
        height: target.height,
        bpp: 32,
        ..Default::default()
    };
    ioctl(fd, DRM_IOCTL_MODE_CREATE_DUMB, &mut dumb, "MODE_CREATE_DUMB")?;
    guard.dumb_handle = Some(dumb.handle);

    // 4) Map it for CPU writes.
    let mut map = DrmModeMapDumb {
        handle: dumb.handle,
        ..Default::default()
    };
    ioctl(fd, DRM_IOCTL_MODE_MAP_DUMB, &mut map, "MODE_MAP_DUMB")?;
    let map_size = dumb.size as usize;
    let mmap_ptr = unsafe {
        libc::mmap(
            std::ptr::null_mut(),
            map_size,
            libc::PROT_READ | libc::PROT_WRITE,
            libc::MAP_SHARED,
            fd,
            map.offset as libc::off_t,
        )
    };
    if mmap_ptr == libc::MAP_FAILED {
        return Err(DrmBlitError::Mmap(std::io::Error::last_os_error()));
    }
    guard.mmap = Some((mmap_ptr as *mut u8, map_size));

    // 5) Write our canvas into the buffer, packing RGBA8888 → XRGB8888
    //    (X channel ignored, R/G/B preserved). Honour the kernel-reported
    //    pitch (stride may be padded for hardware alignment).
    let pitch = dumb.pitch as usize;
    let dst_slice = unsafe { std::slice::from_raw_parts_mut(mmap_ptr as *mut u8, map_size) };
    let mut written = 0usize;
    for y in 0..target.height as usize {
        let row_base = y * pitch;
        for x in 0..target.width as usize {
            let src_off = (y * target.width as usize + x) * 4;
            if src_off + 4 > canvas.data.len() {
                break;
            }
            let r = canvas.data[src_off];
            let g = canvas.data[src_off + 1];
            let b = canvas.data[src_off + 2];
            let dst_off = row_base + x * 4;
            if dst_off + 4 > map_size {
                break;
            }
            // XRGB8888 little-endian = [B, G, R, X]
            dst_slice[dst_off] = b;
            dst_slice[dst_off + 1] = g;
            dst_slice[dst_off + 2] = r;
            dst_slice[dst_off + 3] = 0xFF;
            written += 4;
        }
    }

    // 6) Register as a framebuffer.
    let mut fb = DrmModeFbCmd2 {
        fb_id: 0,
        width: target.width,
        height: target.height,
        pixel_format: DRM_FORMAT_XRGB8888,
        flags: 0,
        handles: [dumb.handle, 0, 0, 0],
        pitches: [dumb.pitch, 0, 0, 0],
        offsets: [0; 4],
        modifier: [0; 4],
    };
    ioctl(fd, DRM_IOCTL_MODE_ADDFB2, &mut fb, "MODE_ADDFB2")?;
    guard.fb_id = Some(fb.fb_id);

    // 7) Scan it out on the CRTC.
    let mut connectors = [target.connector_id];
    let mut setcrtc = DrmModeCrtc {
        set_connectors_ptr: connectors.as_mut_ptr() as u64,
        count_connectors: 1,
        crtc_id: target.crtc_id,
        fb_id: fb.fb_id,
        x: 0,
        y: 0,
        gamma_size: 0,
        mode_valid: 1,
        mode,
    };
    ioctl(fd, DRM_IOCTL_MODE_SETCRTC, &mut setcrtc, "MODE_SETCRTC")?;

    // 8) Hold for the requested duration.
    if hold_ms > 0 {
        std::thread::sleep(std::time::Duration::from_millis(hold_ms));
    }

    // Drop happens via guard: tears down setcrtc -> rmfb -> destroy_dumb -> munmap -> drop_master.
    drop(guard);
    drop(file);
    Ok(written)
}

// -----------------------------------------------------------------------------
// RAII guard ensuring we always restore the device.
// -----------------------------------------------------------------------------

struct MasterGuard {
    fd: i32,
    fb_id: Option<u32>,
    dumb_handle: Option<u32>,
    mmap: Option<(*mut u8, usize)>,
}

impl MasterGuard {
    fn new(fd: i32) -> Self {
        Self { fd, fb_id: None, dumb_handle: None, mmap: None }
    }
}

impl Drop for MasterGuard {
    fn drop(&mut self) {
        if let Some((ptr, len)) = self.mmap.take() {
            unsafe {
                libc::munmap(ptr as *mut libc::c_void, len);
            }
        }
        if let Some(fb_id) = self.fb_id.take() {
            let mut id = fb_id;
            unsafe {
                libc::ioctl(self.fd, DRM_IOCTL_MODE_RMFB as _, &mut id);
            }
        }
        if let Some(handle) = self.dumb_handle.take() {
            let mut destroy = DrmModeDestroyDumb { handle };
            unsafe {
                libc::ioctl(
                    self.fd,
                    DRM_IOCTL_MODE_DESTROY_DUMB as _,
                    &mut destroy as *mut _,
                );
            }
        }
        unsafe {
            libc::ioctl(self.fd, DRM_IOCTL_DROP_MASTER as _);
        }
    }
}

fn ioctl<T>(
    fd: i32,
    request: u64,
    arg: *mut T,
    label: &'static str,
) -> Result<(), DrmBlitError> {
    let r = unsafe { libc::ioctl(fd, request as _, arg as *mut libc::c_void) };
    if r < 0 {
        return Err(DrmBlitError::Ioctl {
            ioctl: label,
            err: std::io::Error::last_os_error(),
        });
    }
    Ok(())
}

// Force `Rgba` into the symbol table so this module can be cross-checked
// from peko-renderer's lib.rs without warnings about unused imports.
#[allow(dead_code)]
fn _color_size_check() -> std::mem::ManuallyDrop<Rgba> {
    std::mem::ManuallyDrop::new(Rgba::BLACK)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ioctl_numbers_match_uapi() {
        // Hand-derived from linux/drm.h + linux/drm_mode.h. Catch typos.
        assert_eq!(DRM_IOCTL_SET_MASTER, 0x0000_641e);
        assert_eq!(DRM_IOCTL_DROP_MASTER, 0x0000_641f);
        assert_eq!(DRM_IOCTL_MODE_CREATE_DUMB, 0xC020_64B2);
        assert_eq!(DRM_IOCTL_MODE_MAP_DUMB, 0xC010_64B3);
        assert_eq!(DRM_IOCTL_MODE_DESTROY_DUMB, 0xC004_64B4);
        assert_eq!(DRM_IOCTL_MODE_ADDFB2, 0xC068_64B8);
        assert_eq!(DRM_IOCTL_MODE_RMFB, 0xC004_64AF);
        assert_eq!(DRM_IOCTL_MODE_SETCRTC, 0xC068_64A2);
    }

    #[test]
    fn struct_sizes_match_uapi() {
        // These sizes are baked into the ioctl numbers above (the size
        // field in _IOWR). If a struct grows, the ioctl number changes
        // and the kernel will return -ENOTTY. Catch that locally.
        assert_eq!(std::mem::size_of::<DrmModeCreateDumb>(), 32);
        assert_eq!(std::mem::size_of::<DrmModeMapDumb>(), 16);
        assert_eq!(std::mem::size_of::<DrmModeDestroyDumb>(), 4);
        assert_eq!(std::mem::size_of::<DrmModeFbCmd2>(), 104);
        assert_eq!(std::mem::size_of::<DrmModeCrtc>(), 104);
        assert_eq!(std::mem::size_of::<DrmModeInfo>(), 68);
    }

    #[test]
    fn drm_format_xrgb8888_packs_xr24() {
        // "XR24" little-endian 'X','R','2','4' → 0x34325258.
        assert_eq!(DRM_FORMAT_XRGB8888, 0x3432_5258);
        let bytes = DRM_FORMAT_XRGB8888.to_le_bytes();
        assert_eq!(&bytes, b"XR24");
    }

    #[test]
    fn pick_target_returns_not_present_on_missing_device() {
        let r = pick_target(Path::new("/nonexistent/dri/card0"));
        match r {
            Err(DrmBlitError::NotPresent { .. }) => {}
            other => panic!("expected NotPresent, got {other:?}"),
        }
    }
}
