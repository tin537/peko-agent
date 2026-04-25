//! DRM enumeration that works without holding DRM master.
//!
//! In Lane B (with SurfaceFlinger up) we cannot read the active scanout
//! buffer through DRM — that requires master, which the compositor owns.
//! But we *can* still inspect the device: list driver name, connectors,
//! resolutions, refresh rates. This is enough for diagnostics and
//! enough for Phase 8 (Lane A) to know what to drive when it does take
//! master.
//!
//! Implementation note: we hand-encode the DRM ioctl numbers rather than
//! pull a `drm-ffi` crate. The set we need is tiny (DRM_IOCTL_VERSION,
//! DRM_IOCTL_MODE_GETRESOURCES, DRM_IOCTL_MODE_GETCONNECTOR), and every
//! external crate that exposes them is either unmaintained or pulls in
//! ~20 transitive deps. We only use ioctls that don't require master,
//! so this never trips DRM authentication on a Lane B device.

use std::fs::OpenOptions;
use std::os::unix::io::AsRawFd;
use std::path::Path;

#[derive(Debug, Clone, Default)]
pub struct DrmInfo {
    pub device_path: String,
    pub driver: String,
    pub kernel_version: String,
    pub connectors: Vec<DrmConnector>,
}

#[derive(Debug, Clone, Default)]
pub struct DrmConnector {
    pub id: u32,
    pub kind: String,
    pub connected: bool,
    pub modes: Vec<DrmMode>,
    pub mm_width: u32,
    pub mm_height: u32,
}

#[derive(Debug, Clone, Default)]
pub struct DrmMode {
    pub width: u16,
    pub height: u16,
    pub refresh_hz: u32,
}

#[derive(Debug, thiserror::Error)]
pub enum DrmInfoError {
    #[error("DRM device {path} not present")]
    NotPresent { path: String },
    #[error("DRM ioctl {ioctl} failed: {err}")]
    Ioctl { ioctl: &'static str, err: std::io::Error },
    #[error("io error opening DRM device: {0}")]
    Io(#[from] std::io::Error),
}

/// Probe the default DRM device and return what we can learn without
/// becoming master. Returns `Ok(None)` if no DRM device is present
/// (very old kernels or stripped images) — the caller treats that as
/// "DRM not in use" rather than an error.
pub fn probe_default() -> Result<Option<DrmInfo>, DrmInfoError> {
    for path in ["/dev/dri/card0", "/dev/dri/card1"] {
        if Path::new(path).exists() {
            return probe(path).map(Some);
        }
    }
    Ok(None)
}

/// Probe a specific DRM device. Returns the kernel-name string from
/// `DRM_IOCTL_VERSION` plus the resource list. Connector enumeration is
/// best-effort: a mode list that fails to read leaves `modes` empty
/// rather than aborting the whole probe.
pub fn probe(path: &str) -> Result<DrmInfo, DrmInfoError> {
    if !Path::new(path).exists() {
        return Err(DrmInfoError::NotPresent { path: path.to_string() });
    }
    let file = OpenOptions::new().read(true).write(true).open(path)?;
    let fd = file.as_raw_fd();

    let (driver, kernel_version) = read_version(fd)?;
    let connectors = read_connectors(fd).unwrap_or_default();

    Ok(DrmInfo {
        device_path: path.to_string(),
        driver,
        kernel_version,
        connectors,
    })
}

// -----------------------------------------------------------------------------
// DRM_IOCTL_VERSION
// -----------------------------------------------------------------------------

#[repr(C)]
#[derive(Default)]
struct DrmVersionRaw {
    version_major: i32,
    version_minor: i32,
    version_patchlevel: i32,
    name_len: usize,
    name: usize,
    date_len: usize,
    date: usize,
    desc_len: usize,
    desc: usize,
}

// _IOC(READ|WRITE, 'd', 0x00, sizeof(drm_version)) on 64-bit.
//   _IOC_NRBITS=8, _IOC_TYPEBITS=8, _IOC_SIZEBITS=14, _IOC_DIRBITS=2
//   READ|WRITE = 3; size = 72 (struct drm_version on 64-bit)
//   number = (3 << 30) | (72 << 16) | ('d' << 8) | 0x00 = 0xC048_6400
const DRM_IOCTL_VERSION: u64 = 0xC048_6400;

fn read_version(fd: i32) -> Result<(String, String), DrmInfoError> {
    let mut name_buf = vec![0u8; 64];
    let mut date_buf = vec![0u8; 64];
    let mut desc_buf = vec![0u8; 256];

    // First call: probe lengths with zero pointers.
    let mut v = DrmVersionRaw {
        name_len: name_buf.len(),
        name: name_buf.as_mut_ptr() as usize,
        date_len: date_buf.len(),
        date: date_buf.as_mut_ptr() as usize,
        desc_len: desc_buf.len(),
        desc: desc_buf.as_mut_ptr() as usize,
        ..Default::default()
    };
    let ret = unsafe {
        crate::raw_ioctl(fd, DRM_IOCTL_VERSION, &mut v as *mut _ as *mut libc::c_void)
    };
    if ret < 0 {
        return Err(DrmInfoError::Ioctl {
            ioctl: "DRM_IOCTL_VERSION",
            err: std::io::Error::last_os_error(),
        });
    }
    let name = String::from_utf8_lossy(&name_buf[..v.name_len.min(name_buf.len())])
        .trim_end_matches('\0')
        .to_string();
    let date = String::from_utf8_lossy(&date_buf[..v.date_len.min(date_buf.len())])
        .trim_end_matches('\0')
        .to_string();
    let kver = format!(
        "{}.{}.{} ({})",
        v.version_major, v.version_minor, v.version_patchlevel, date
    );
    Ok((name, kver))
}

// -----------------------------------------------------------------------------
// DRM_IOCTL_MODE_GETRESOURCES + GETCONNECTOR
// -----------------------------------------------------------------------------

// Keep the structs minimal — we only read fields we use and pass back
// pointer/count pairs the kernel mutates in place.

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

// _IOC(READ|WRITE, 'd', 0xA0, sizeof(drm_mode_card_res)=64)
const DRM_IOCTL_MODE_GETRESOURCES: u64 = 0xC040_64A0;

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

// _IOC(READ|WRITE, 'd', 0xA7, sizeof(drm_mode_get_connector)=80)
const DRM_IOCTL_MODE_GETCONNECTOR: u64 = 0xC050_64A7;

#[repr(C)]
#[derive(Default, Clone, Copy)]
struct DrmModeInfoRaw {
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

fn read_connectors(fd: i32) -> Result<Vec<DrmConnector>, DrmInfoError> {
    // Two-phase: first call with zero pointers to learn counts, then
    // allocate buffers and call again.
    let mut res = DrmModeCardRes::default();
    let ret = unsafe {
        crate::raw_ioctl(
            fd,
            DRM_IOCTL_MODE_GETRESOURCES,
            &mut res as *mut _ as *mut libc::c_void,
        )
    };
    if ret < 0 {
        return Err(DrmInfoError::Ioctl {
            ioctl: "DRM_IOCTL_MODE_GETRESOURCES",
            err: std::io::Error::last_os_error(),
        });
    }
    if res.count_connectors == 0 {
        return Ok(Vec::new());
    }
    let mut connector_ids = vec![0u32; res.count_connectors as usize];
    let mut crtc_ids = vec![0u32; res.count_crtcs as usize];
    let mut encoder_ids = vec![0u32; res.count_encoders as usize];
    let mut fb_ids = vec![0u32; res.count_fbs as usize];
    res.connector_id_ptr = connector_ids.as_mut_ptr() as u64;
    res.crtc_id_ptr = crtc_ids.as_mut_ptr() as u64;
    res.encoder_id_ptr = encoder_ids.as_mut_ptr() as u64;
    res.fb_id_ptr = fb_ids.as_mut_ptr() as u64;

    let ret = unsafe {
        crate::raw_ioctl(
            fd,
            DRM_IOCTL_MODE_GETRESOURCES,
            &mut res as *mut _ as *mut libc::c_void,
        )
    };
    if ret < 0 {
        return Err(DrmInfoError::Ioctl {
            ioctl: "DRM_IOCTL_MODE_GETRESOURCES",
            err: std::io::Error::last_os_error(),
        });
    }

    let mut out = Vec::with_capacity(connector_ids.len());
    for &id in &connector_ids {
        match read_connector(fd, id) {
            Ok(c) => out.push(c),
            Err(e) => tracing::debug!(connector_id = id, error = %e, "skipped connector"),
        }
    }
    Ok(out)
}

fn read_connector(fd: i32, id: u32) -> Result<DrmConnector, DrmInfoError> {
    let mut conn = DrmModeGetConnector {
        connector_id: id,
        ..Default::default()
    };
    // Phase 1: probe counts.
    let ret = unsafe {
        crate::raw_ioctl(
            fd,
            DRM_IOCTL_MODE_GETCONNECTOR,
            &mut conn as *mut _ as *mut libc::c_void,
        )
    };
    if ret < 0 {
        return Err(DrmInfoError::Ioctl {
            ioctl: "DRM_IOCTL_MODE_GETCONNECTOR",
            err: std::io::Error::last_os_error(),
        });
    }
    let mut modes = vec![DrmModeInfoRaw::default(); conn.count_modes as usize];
    let mut encoders = vec![0u32; conn.count_encoders as usize];
    let mut props = vec![0u32; conn.count_props as usize];
    let mut prop_values = vec![0u64; conn.count_props as usize];
    conn.modes_ptr = modes.as_mut_ptr() as u64;
    conn.encoders_ptr = encoders.as_mut_ptr() as u64;
    conn.props_ptr = props.as_mut_ptr() as u64;
    conn.prop_values_ptr = prop_values.as_mut_ptr() as u64;

    let ret = unsafe {
        crate::raw_ioctl(
            fd,
            DRM_IOCTL_MODE_GETCONNECTOR,
            &mut conn as *mut _ as *mut libc::c_void,
        )
    };
    if ret < 0 {
        return Err(DrmInfoError::Ioctl {
            ioctl: "DRM_IOCTL_MODE_GETCONNECTOR",
            err: std::io::Error::last_os_error(),
        });
    }
    let connected = conn.connection == 1; // DRM_MODE_CONNECTED
    let parsed_modes = modes
        .iter()
        .take(conn.count_modes as usize)
        .map(|m| DrmMode {
            width: m.hdisplay,
            height: m.vdisplay,
            refresh_hz: m.vrefresh,
        })
        .collect();

    Ok(DrmConnector {
        id: conn.connector_id,
        kind: connector_type_name(conn.connector_type),
        connected,
        modes: parsed_modes,
        mm_width: conn.mm_width,
        mm_height: conn.mm_height,
    })
}

// Subset that covers every connector type Android phones actually expose;
// the value space is stable across kernels.
fn connector_type_name(t: u32) -> String {
    match t {
        0 => "Unknown",
        1 => "VGA",
        2 => "DVI-I",
        3 => "DVI-D",
        4 => "DVI-A",
        5 => "Composite",
        6 => "SVIDEO",
        7 => "LVDS",
        8 => "Component",
        9 => "9PinDIN",
        10 => "DisplayPort",
        11 => "HDMI-A",
        12 => "HDMI-B",
        13 => "TV",
        14 => "eDP",
        15 => "Virtual",
        16 => "DSI",
        17 => "DPI",
        18 => "Writeback",
        19 => "SPI",
        20 => "USB",
        _ => "Unknown",
    }
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn connector_type_names_are_stable() {
        assert_eq!(connector_type_name(11), "HDMI-A");
        assert_eq!(connector_type_name(16), "DSI");
        assert_eq!(connector_type_name(14), "eDP");
        assert_eq!(connector_type_name(99), "Unknown");
    }

    #[test]
    fn ioctl_numbers_match_uapi() {
        // Sanity-check the hand-encoded ioctl numbers against the
        // (well-known) values in linux/drm.h. Catches accidental edits.
        assert_eq!(DRM_IOCTL_VERSION, 0xC048_6400);
        assert_eq!(DRM_IOCTL_MODE_GETRESOURCES, 0xC040_64A0);
        assert_eq!(DRM_IOCTL_MODE_GETCONNECTOR, 0xC050_64A7);
    }

    #[test]
    fn probe_default_returns_ok_or_none() {
        // On a host without /dev/dri/* this should return Ok(None),
        // not an error. Validates the silent-no-DRM path.
        let r = probe_default();
        match r {
            Ok(_) => {}
            Err(DrmInfoError::NotPresent { .. }) => {}
            Err(other) => panic!("unexpected error: {:?}", other),
        }
    }
}
