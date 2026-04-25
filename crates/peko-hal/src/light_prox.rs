//! Read light + proximity sensors.
//!
//! These two are special on Android: they're rarely IIO devices.
//! Vendors expose them as input devices that emit `EV_ABS` events on
//! axis `ABS_DISTANCE` (proximity, "near"/"far") or `ABS_MISC`
//! (illuminance, lux). Reading them means polling the input device
//! and grabbing the latest absolute value.
//!
//! Some kernels also offer a sysfs reading at
//! `/sys/class/sensors/<name>/raw_data`. We probe that as a fallback
//! because polling input subscribes to a stream — fine for "wait for
//! prox change", awkward for "what's the current value right now".

use crate::input::InputDevice;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const EV_ABS: u16 = 0x03;
const ABS_DISTANCE: u16 = 0x19;
const ABS_MISC: u16 = 0x28;

#[derive(Debug, thiserror::Error)]
pub enum LightProxError {
    #[error("no {kind} sensor found via input or sysfs")]
    NotFound { kind: &'static str },
    #[error("io reading {path}: {err}")]
    Io {
        path: PathBuf,
        #[source]
        err: std::io::Error,
    },
    #[error("read timed out waiting for sensor event")]
    Timeout,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SensorKind {
    Light,
    Proximity,
}

impl SensorKind {
    fn label(self) -> &'static str {
        match self {
            SensorKind::Light => "light",
            SensorKind::Proximity => "proximity",
        }
    }

    fn name_match(self, lower: &str) -> bool {
        match self {
            SensorKind::Light => {
                lower.contains("light") || lower.contains("als") || lower.contains("lux")
            }
            SensorKind::Proximity => lower.contains("prox"),
        }
    }

    fn abs_axis(self) -> u16 {
        match self {
            SensorKind::Light => ABS_MISC,
            SensorKind::Proximity => ABS_DISTANCE,
        }
    }

    fn sysfs_unit(self) -> &'static str {
        match self {
            SensorKind::Light => "lux",
            SensorKind::Proximity => "cm",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Reading {
    pub kind: SensorKind,
    pub value: f64,
    pub unit: &'static str,
    pub source: PathBuf,
}

/// Read the current value. Tries sysfs first (instant snapshot), falls
/// back to input event polling (waits for the next sample).
pub fn read(kind: SensorKind, timeout: Duration) -> Result<Reading, LightProxError> {
    if let Ok(r) = read_via_sysfs(kind) {
        return Ok(r);
    }
    read_via_input(kind, timeout)
}

fn read_via_sysfs(kind: SensorKind) -> Result<Reading, LightProxError> {
    let class = Path::new("/sys/class/sensors");
    if !class.exists() {
        return Err(LightProxError::NotFound { kind: kind.label() });
    }
    let entries = fs::read_dir(class).map_err(|err| LightProxError::Io {
        path: class.to_path_buf(),
        err,
    })?;
    for entry in entries.flatten() {
        let dir = entry.path();
        let name = fs::read_to_string(dir.join("name")).unwrap_or_default();
        if !kind.name_match(&name.to_lowercase()) {
            continue;
        }
        for cand in ["raw_data", "data", "value"] {
            let p = dir.join(cand);
            if let Ok(s) = fs::read_to_string(&p) {
                if let Some(first) = s.split_whitespace().next() {
                    if let Ok(v) = first.parse::<f64>() {
                        return Ok(Reading {
                            kind,
                            value: v,
                            unit: kind.sysfs_unit(),
                            source: p,
                        });
                    }
                }
            }
        }
    }
    Err(LightProxError::NotFound { kind: kind.label() })
}

fn read_via_input(kind: SensorKind, timeout: Duration) -> Result<Reading, LightProxError> {
    let mut device = find_input_for(kind)?;
    let path = device.path().to_path_buf();
    let deadline = Instant::now() + timeout;
    loop {
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .unwrap_or(Duration::ZERO);
        let timeout_ms = remaining.as_millis().min(60_000) as i32;
        if timeout_ms <= 0 {
            return Err(LightProxError::Timeout);
        }
        let event = device
            .poll_for_event(timeout_ms)
            .map_err(|err| LightProxError::Io {
                path: path.clone(),
                err: std::io::Error::other(err.to_string()),
            })?;
        let Some(ev) = event else {
            continue;
        };
        if ev.type_ == EV_ABS && ev.code == kind.abs_axis() {
            return Ok(Reading {
                kind,
                value: ev.value as f64,
                unit: kind.sysfs_unit(),
                source: path,
            });
        }
        if Instant::now() >= deadline {
            return Err(LightProxError::Timeout);
        }
    }
}

fn find_input_for(kind: SensorKind) -> Result<InputDevice, LightProxError> {
    let dir = Path::new("/sys/class/input");
    let entries = fs::read_dir(dir).map_err(|err| LightProxError::Io {
        path: dir.to_path_buf(),
        err,
    })?;
    for entry in entries.flatten() {
        let path = entry.path();
        let name = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
        if !name.starts_with("event") {
            continue;
        }
        let dev_name = fs::read_to_string(path.join("device/name")).unwrap_or_default();
        if !kind.name_match(&dev_name.to_lowercase()) {
            continue;
        }
        let dev_path = PathBuf::from("/dev/input").join(name);
        if let Ok(dev) = InputDevice::open(&dev_path) {
            return Ok(dev);
        }
    }
    Err(LightProxError::NotFound { kind: kind.label() })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn name_match_distinguishes_kinds() {
        assert!(SensorKind::Light.name_match("apds9960 als"));
        assert!(SensorKind::Light.name_match("stk3a5x light sensor"));
        assert!(SensorKind::Proximity.name_match("stk3a5x proximity"));
        assert!(!SensorKind::Light.name_match("stk3a5x proximity"));
        assert!(!SensorKind::Proximity.name_match("apds9960 als"));
    }

    #[test]
    fn abs_axes_are_correct() {
        assert_eq!(SensorKind::Light.abs_axis(), ABS_MISC);
        assert_eq!(SensorKind::Proximity.abs_axis(), ABS_DISTANCE);
    }

    #[test]
    fn read_via_sysfs_handles_missing_subsystem() {
        // On hosts without /sys/class/sensors, this should be NotFound,
        // not panic.
        let _ = read_via_sysfs(SensorKind::Light);
    }
}
