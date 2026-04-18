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
use std::io::{Read, Write};
use std::os::unix::fs::OpenOptionsExt;
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
    // 1. Explicit non-"auto" config wins. Empty / "auto" triggers probe.
    if let Some(c) = configured {
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
pub fn probe_modem() -> Option<PathBuf> {
    info!(candidates = PROBE_CANDIDATES.len(), "probing modem paths");
    for path in PROBE_CANDIDATES {
        let p = Path::new(path);
        if !p.exists() {
            continue;
        }
        match send_at_probe(p) {
            Ok(true) => {
                info!(device = %path, "modem auto-detected");
                return Some(PathBuf::from(path));
            }
            Ok(false) => {
                debug!(device = %path, "modem path exists but no OK response");
            }
            Err(e) => {
                debug!(device = %path, error = %e, "modem probe error");
            }
        }
    }
    warn!("no modem responded to AT probe — SMS/Call tools will be disabled");
    None
}

/// Write `AT\r` and return true if the response contains `OK` within
/// PROBE_TIMEOUT_MS. Non-blocking + timeout-bounded so a hung character
/// device doesn't hang peko's startup.
fn send_at_probe(path: &Path) -> std::io::Result<bool> {
    // O_NONBLOCK so a dead device doesn't block; we poll with a timeout.
    const O_NONBLOCK: i32 = 0o4000;
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .custom_flags(O_NONBLOCK)
        .open(path)?;

    // Best-effort: some modems need CRLF, some need just CR.
    let _ = file.write_all(b"AT\r");

    let deadline = Instant::now() + Duration::from_millis(PROBE_TIMEOUT_MS);
    let mut buf = [0u8; 256];
    let mut acc = Vec::with_capacity(256);

    while Instant::now() < deadline {
        match file.read(&mut buf) {
            Ok(0) => std::thread::sleep(Duration::from_millis(10)),
            Ok(n) => {
                acc.extend_from_slice(&buf[..n]);
                if acc.windows(2).any(|w| w == b"OK") {
                    return Ok(true);
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(e) => return Err(e),
        }
    }
    Ok(false)
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
