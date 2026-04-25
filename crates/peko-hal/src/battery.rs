//! Read battery state from `/sys/class/power_supply/battery/`.
//!
//! Every Android kernel exposes the same field set under power_supply
//! (it's a Linux kernel ABI). Filenames are stable, units are
//! standardised: voltage in µV, current in µA, temp in 0.1 °C,
//! capacity in % (0..100). We never shell out to `dumpsys battery` —
//! that needs framework, which is exactly what we're trying to avoid.

use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, thiserror::Error)]
pub enum BatteryError {
    #[error("battery node not found at {path}")]
    NotFound { path: PathBuf },
    #[error("read {path}: {err}")]
    Io {
        path: PathBuf,
        #[source]
        err: std::io::Error,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChargeStatus {
    Charging,
    Discharging,
    NotCharging,
    Full,
    Unknown,
}

impl ChargeStatus {
    fn parse(s: &str) -> Self {
        match s.trim() {
            "Charging" => ChargeStatus::Charging,
            "Discharging" => ChargeStatus::Discharging,
            "Not charging" => ChargeStatus::NotCharging,
            "Full" => ChargeStatus::Full,
            _ => ChargeStatus::Unknown,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            ChargeStatus::Charging => "Charging",
            ChargeStatus::Discharging => "Discharging",
            ChargeStatus::NotCharging => "Not charging",
            ChargeStatus::Full => "Full",
            ChargeStatus::Unknown => "Unknown",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BatteryHealth {
    Good,
    Overheat,
    Dead,
    Cold,
    OverVoltage,
    UnspecifiedFailure,
    Unknown,
}

impl BatteryHealth {
    fn parse(s: &str) -> Self {
        match s.trim() {
            "Good" => BatteryHealth::Good,
            "Overheat" => BatteryHealth::Overheat,
            "Dead" => BatteryHealth::Dead,
            "Cold" => BatteryHealth::Cold,
            "Over voltage" => BatteryHealth::OverVoltage,
            "Unspecified failure" => BatteryHealth::UnspecifiedFailure,
            _ => BatteryHealth::Unknown,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            BatteryHealth::Good => "Good",
            BatteryHealth::Overheat => "Overheat",
            BatteryHealth::Dead => "Dead",
            BatteryHealth::Cold => "Cold",
            BatteryHealth::OverVoltage => "Over voltage",
            BatteryHealth::UnspecifiedFailure => "Unspecified failure",
            BatteryHealth::Unknown => "Unknown",
        }
    }
}

#[derive(Debug, Clone)]
pub struct BatteryState {
    pub capacity_pct: Option<u8>,
    pub status: ChargeStatus,
    pub health: BatteryHealth,
    /// Battery voltage in volts (read as µV from sysfs and converted).
    pub voltage_v: Option<f64>,
    /// Instantaneous current in mA (read as µA, converted, sign preserved).
    /// Negative = discharging on most kernels.
    pub current_ma: Option<f64>,
    /// Battery temperature in degrees Celsius (read as deci-°C, divided).
    pub temperature_c: Option<f64>,
    pub technology: Option<String>,
    pub present: Option<bool>,
}

const DEFAULT_PATH: &str = "/sys/class/power_supply/battery";

pub fn read() -> Result<BatteryState, BatteryError> {
    read_from(Path::new(DEFAULT_PATH))
}

pub fn read_from(dir: &Path) -> Result<BatteryState, BatteryError> {
    if !dir.exists() {
        return Err(BatteryError::NotFound { path: dir.to_path_buf() });
    }

    Ok(BatteryState {
        capacity_pct: read_int(dir, "capacity").ok().and_then(|v| u8::try_from(v).ok()),
        status: read_str(dir, "status")
            .ok()
            .map(|s| ChargeStatus::parse(&s))
            .unwrap_or(ChargeStatus::Unknown),
        health: read_str(dir, "health")
            .ok()
            .map(|s| BatteryHealth::parse(&s))
            .unwrap_or(BatteryHealth::Unknown),
        voltage_v: read_int(dir, "voltage_now").ok().map(|uv| uv as f64 / 1_000_000.0),
        current_ma: read_int(dir, "current_now").ok().map(|ua| ua as f64 / 1_000.0),
        temperature_c: read_int(dir, "temp").ok().map(|deci| deci as f64 / 10.0),
        technology: read_str(dir, "technology").ok().filter(|s| !s.is_empty()),
        present: read_int(dir, "present").ok().map(|v| v != 0),
    })
}

fn read_str(dir: &Path, name: &str) -> Result<String, BatteryError> {
    let path = dir.join(name);
    fs::read_to_string(&path)
        .map(|s| s.trim().to_string())
        .map_err(|err| BatteryError::Io { path, err })
}

fn read_int(dir: &Path, name: &str) -> Result<i64, BatteryError> {
    let s = read_str(dir, name)?;
    s.trim().parse().map_err(|_| BatteryError::Io {
        path: dir.join(name),
        err: std::io::Error::new(std::io::ErrorKind::InvalidData, format!("not an int: {s}")),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn tmp() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("peko_battery_test_{}", rand_id()));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn rand_id() -> u64 {
        // Avoid pulling in `rand` for one usage. Process id + nanos is
        // unique enough across the whole `cargo test` run.
        let pid = std::process::id() as u64;
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.subsec_nanos() as u64)
            .unwrap_or(0);
        pid * 1_000_000_000 + nanos
    }

    #[test]
    fn parses_full_battery_node() {
        let d = tmp();
        fs::write(d.join("capacity"), "78\n").unwrap();
        fs::write(d.join("status"), "Charging\n").unwrap();
        fs::write(d.join("health"), "Good\n").unwrap();
        fs::write(d.join("voltage_now"), "4123000\n").unwrap();
        fs::write(d.join("current_now"), "-512000\n").unwrap();
        fs::write(d.join("temp"), "278\n").unwrap();
        fs::write(d.join("technology"), "Li-ion\n").unwrap();
        fs::write(d.join("present"), "1\n").unwrap();

        let s = read_from(&d).unwrap();
        assert_eq!(s.capacity_pct, Some(78));
        assert_eq!(s.status, ChargeStatus::Charging);
        assert_eq!(s.health, BatteryHealth::Good);
        assert!((s.voltage_v.unwrap() - 4.123).abs() < 1e-6);
        assert!((s.current_ma.unwrap() + 512.0).abs() < 1e-6);
        assert!((s.temperature_c.unwrap() - 27.8).abs() < 1e-6);
        assert_eq!(s.technology.as_deref(), Some("Li-ion"));
        assert_eq!(s.present, Some(true));
        fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn missing_node_is_not_found() {
        let bogus = std::env::temp_dir().join(format!("peko_no_batt_{}", rand_id()));
        let r = read_from(&bogus);
        match r {
            Err(BatteryError::NotFound { .. }) => {}
            other => panic!("expected NotFound, got {:?}", other),
        }
    }

    #[test]
    fn partial_node_uses_unknowns_for_missing_fields() {
        let d = tmp();
        fs::write(d.join("capacity"), "42\n").unwrap();
        // No status, health, voltage etc.
        let s = read_from(&d).unwrap();
        assert_eq!(s.capacity_pct, Some(42));
        assert_eq!(s.status, ChargeStatus::Unknown);
        assert_eq!(s.health, BatteryHealth::Unknown);
        assert!(s.voltage_v.is_none());
        assert!(s.current_ma.is_none());
        fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn status_parser_covers_known_values() {
        assert_eq!(ChargeStatus::parse("Charging"), ChargeStatus::Charging);
        assert_eq!(ChargeStatus::parse("Discharging"), ChargeStatus::Discharging);
        assert_eq!(ChargeStatus::parse("Full"), ChargeStatus::Full);
        assert_eq!(ChargeStatus::parse("Not charging"), ChargeStatus::NotCharging);
        assert_eq!(ChargeStatus::parse("Bogus"), ChargeStatus::Unknown);
    }
}
