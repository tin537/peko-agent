//! Hardware auto-probe.
//!
//! Peko's Touch and Screenshot tools already self-detect their devices
//! (see `InputDevice::find_touchscreen` and the screencap framebuffer
//! fallback). This module fills the last gap: the **modem** device used
//! for AT commands (SMS + Call tools).
//!
//! The probe runs once on startup when the config's `hardware.modem_device`
//! is missing or set to `"auto"`. A successful probe is cached to
//! `<data_dir>/detected_hardware.json` so subsequent boots skip the scan.
//!
//! Detection strategy: write `AT\r` to a candidate char device and read up
//! to 500ms of response. If the reply contains `OK`, that's a modem.
//!
//! Why this list, in this order: `/dev/smd11` is the Qualcomm-standard AT
//! primary channel used by OnePlus 6T (fajita), Redmi 10C (fog), Pixels
//! through 6, and most SD6xx/SD8xx devices. Fallbacks cover USB-passthrough
//! modems and older/newer radio layouts.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::io::AsRawFd;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tracing::{info, warn, debug};

const PROBE_TIMEOUT_MS: u64 = 500;
const PROBE_CANDIDATES: &[&str] = &[
    "/dev/smd11",            // Qualcomm AT primary — fajita, fog, most SD devices
    "/dev/smd1",             // Qualcomm AT secondary
    "/dev/ttyUSB2",          // USB modem passthrough (dongles, some tablets)
    "/dev/ttyUSB0",
    "/dev/radio/smd11",      // Newer Qualcomm layout (A13+ some devices)
    "/dev/umts_boot0",       // Samsung Exynos
];

/// Cache file for detected hardware — sits in data_dir so it persists
/// across peko restarts, and gets wiped on factory reset.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DetectedHardware {
    pub modem_device: Option<String>,
    /// ISO8601 timestamp of the last successful probe. Present so future
    /// probe logic can re-probe if this is stale (e.g. after a kernel
    /// upgrade that renames devices).
    pub detected_at: Option<String>,
}

impl DetectedHardware {
    pub fn load(data_dir: &Path) -> Self {
        let path = data_dir.join("detected_hardware.json");
        match fs::read_to_string(&path) {
            Ok(s) => serde_json::from_str(&s).unwrap_or_default(),
            Err(_) => Self::default(),
        }
    }

    pub fn save(&self, data_dir: &Path) -> std::io::Result<()> {
        let path = data_dir.join("detected_hardware.json");
        let tmp = path.with_extension("json.tmp");
        fs::write(&tmp, serde_json::to_string_pretty(self).unwrap_or_default())?;
        fs::rename(&tmp, &path)
    }
}

/// Resolve a modem path: prefer explicit config, then cached, then probe.
/// Writes the cache when a new device is discovered. Returns None if no
/// modem is present — callers should disable SMS/Call tools in that case.
pub fn resolve_modem(
    configured: Option<&str>,
    data_dir: &Path,
) -> Option<PathBuf> {
    // 1a. Explicit "disable" — user knows there's no usable modem (e.g.
    //     OnePlus 6T, where RILD owns the AT channel). Skip both probe
    //     and open attempts so main.rs doesn't even try to register
    //     SMS/Call tools, keeping the boot log clean.
    if let Some(c) = configured {
        let c = c.trim();
        if matches!(c, "none" | "disabled" | "skip" | "off") {
            debug!("modem probe disabled by config");
            return None;
        }
        // 1b. Explicit path wins without probing.
        if !c.is_empty() && c != "auto" {
            return Some(PathBuf::from(c));
        }
    }

    // 2. Cache — skip the probe if a previous run succeeded.
    let mut cache = DetectedHardware::load(data_dir);
    if let Some(ref cached) = cache.modem_device {
        if Path::new(cached).exists() {
            debug!(device = %cached, "modem from cache");
            return Some(PathBuf::from(cached));
        }
        // Cached path vanished — fall through to re-probe.
        warn!(cached, "cached modem device missing, re-probing");
        cache.modem_device = None;
    }

    // 3. Live probe — write AT, read response, match "OK".
    let found = probe_modem();
    if let Some(ref path) = found {
        cache.modem_device = Some(path.to_string_lossy().into_owned());
        cache.detected_at = Some(chrono::Utc::now().to_rfc3339());
        if let Err(e) = cache.save(data_dir) {
            warn!(error = %e, "failed to cache detected hardware");
        }
    }
    found
}

/// Try each candidate in order; return the first that responds to AT.
///
/// Each candidate is probed inside its own watchdog thread with a hard
/// join-timeout. This is belt-and-braces for kernels where *both* read()
/// and poll() ignore O_NONBLOCK and lie about readiness (seen on Qualcomm
/// glink_pkt on sdm845/fajita) — if the probe thread wedges inside
/// `glink_pkt_read`, the main thread still moves on after the timeout
/// and startup proceeds. The wedged thread leaks a single file descriptor
/// until process exit; acceptable cost for a one-time boot probe.
pub fn probe_modem() -> Option<PathBuf> {
    info!(candidates = PROBE_CANDIDATES.len(), "probing modem paths");
    for path in PROBE_CANDIDATES {
        let p = Path::new(path);
        if !p.exists() {
            continue;
        }
        match probe_with_watchdog(p, PROBE_TIMEOUT_MS) {
            Some(true) => {
                info!(device = %path, "modem auto-detected");
                return Some(PathBuf::from(path));
            }
            Some(false) => {
                debug!(device = %path, "modem path exists but no OK response");
            }
            None => {
                debug!(device = %path, "modem probe wedged, moving on");
            }
        }
    }
    warn!("no modem responded to AT probe — SMS/Call tools will be disabled");
    None
}

/// Runs `send_at_probe` inside a detached thread and joins with a hard
/// deadline. Returns:
///   - `Some(true)`  — modem responded with `OK`
///   - `Some(false)` — probe finished in time, no response / error
///   - `None`        — probe thread wedged past the deadline; abandoned
fn probe_with_watchdog(path: &Path, inner_timeout_ms: u64) -> Option<bool> {
    let (tx, rx) = std::sync::mpsc::channel::<std::io::Result<bool>>();
    let probe_path = path.to_path_buf();
    // Detach — we never join this handle so a wedged probe doesn't keep
    // us on the startup critical path.
    std::thread::spawn(move || {
        let _ = tx.send(send_at_probe(&probe_path));
    });
    // Outer deadline > inner deadline by a small margin so the inner loop's
    // own poll timeout usually fires first; the watchdog is only invoked
    // when the kernel ignores O_NONBLOCK entirely.
    match rx.recv_timeout(Duration::from_millis(inner_timeout_ms + 300)) {
        Ok(Ok(b)) => Some(b),
        Ok(Err(_)) => Some(false),
        Err(_) => None,
    }
}

/// Write `AT\r` and return true if the response contains `OK` within
/// PROBE_TIMEOUT_MS.
///
/// Implementation note: the previous version used `OpenOptions::custom_flags
/// (O_NONBLOCK)` + `file.read()` in a sleep-loop. On Qualcomm sdm845/sdm660
/// (OnePlus 6T/fajita, Redmi fog, Pixel 3 series) the glink_pkt char driver
/// **silently ignores O_NONBLOCK on read()**, so the read() syscall enters
/// the kernel and blocks forever inside `glink_pkt_read`, never honouring
/// the loop's time budget. Symptom: peko-agent hangs in uninterruptible
/// sleep after logging "probing modem paths", and `:8080` is never bound.
///
/// Fix: use `libc::poll()` before each read. poll() uses the driver's
/// `.poll` fop (not `.read`) and returns readiness correctly even on the
/// broken glink driver. If poll times out, we bail without ever calling
/// read() on the hung FD. O_NONBLOCK is kept on open() itself as a safety
/// belt in case the driver blocks inside open() too.
fn send_at_probe(path: &Path) -> std::io::Result<bool> {
    const O_NONBLOCK: i32 = 0o4000;
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .custom_flags(O_NONBLOCK)
        .open(path)?;

    // Best-effort: some modems want CR, some CRLF. The write() path on
    // glink is bounded (3-byte payload) and hasn't been observed to hang.
    let _ = (&file).write_all(b"AT\r");

    let fd = file.as_raw_fd();
    let deadline = Instant::now() + Duration::from_millis(PROBE_TIMEOUT_MS);
    let mut buf = [0u8; 256];
    let mut acc = Vec::with_capacity(256);

    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Ok(false);
        }

        let mut pfd = libc::pollfd {
            fd,
            events: libc::POLLIN,
            revents: 0,
        };
        let rc = unsafe { libc::poll(&mut pfd as *mut _, 1, remaining.as_millis() as i32) };
        match rc {
            -1 => {
                let e = std::io::Error::last_os_error();
                if e.kind() == std::io::ErrorKind::Interrupted {
                    continue;
                }
                return Err(e);
            }
            0 => return Ok(false), // timeout — no data
            _ => {
                if pfd.revents & (libc::POLLERR | libc::POLLHUP | libc::POLLNVAL) != 0 {
                    return Ok(false); // device in error/hangup state
                }
                if pfd.revents & libc::POLLIN == 0 {
                    continue; // spurious wake, re-poll with remaining budget
                }
                let n = unsafe {
                    libc::read(fd, buf.as_mut_ptr() as *mut libc::c_void, buf.len())
                };
                if n < 0 {
                    let e = std::io::Error::last_os_error();
                    match e.kind() {
                        std::io::ErrorKind::Interrupted | std::io::ErrorKind::WouldBlock => continue,
                        _ => return Err(e),
                    }
                } else if n == 0 {
                    return Ok(false); // EOF
                } else {
                    acc.extend_from_slice(&buf[..n as usize]);
                    if acc.windows(2).any(|w| w == b"OK") {
                        return Ok(true);
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_respects_explicit_config() {
        let tmp = std::env::temp_dir().join(format!("peko_hw_test_{}", std::process::id()));
        let _ = fs::create_dir_all(&tmp);
        let got = resolve_modem(Some("/dev/specific"), &tmp);
        assert_eq!(got.as_deref(), Some(Path::new("/dev/specific")));
        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn resolve_auto_triggers_probe() {
        let tmp = std::env::temp_dir().join(format!("peko_hw_probe_{}", std::process::id()));
        let _ = fs::create_dir_all(&tmp);
        // "auto" falls through to probe; on a host machine probe returns None,
        // which is the correct behavior.
        let got = resolve_modem(Some("auto"), &tmp);
        assert!(got.is_none());
        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn detected_hardware_roundtrip() {
        let tmp = std::env::temp_dir().join(format!("peko_hw_rt_{}", std::process::id()));
        let _ = fs::create_dir_all(&tmp);
        let mut hw = DetectedHardware::default();
        hw.modem_device = Some("/dev/smd11".to_string());
        hw.detected_at  = Some("2026-04-18T00:00:00Z".to_string());
        hw.save(&tmp).unwrap();
        let reloaded = DetectedHardware::load(&tmp);
        assert_eq!(reloaded.modem_device.as_deref(), Some("/dev/smd11"));
        let _ = fs::remove_dir_all(&tmp);
    }
}
