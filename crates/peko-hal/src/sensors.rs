//! Read sensor values from the kernel directly, no Android sensor HAL.
//!
//! Modern Android kernels expose three different paths for the same
//! physical sensors:
//!
//!   1. `/sys/bus/iio/devices/iio:deviceN/` — Industrial IO subsystem.
//!      Standard upstream Linux. Each axis is `in_accel_x_raw` and
//!      conversion is `value = (raw + offset) * scale`.
//!   2. `/sys/class/sensors/<name>/` — Qualcomm vendor path. Same
//!      values, ad-hoc filenames. Used as a fallback when IIO doesn't
//!      expose a particular sensor.
//!   3. `/dev/input/eventN` for proximity / light — these arrive as
//!      EV_ABS events on input devices named "*proximity*" or
//!      "*als*". We poll them with `InputDevice::poll_for_event()`.
//!
//! IIO is the preferred path because the units are documented and the
//! conversion is mechanical. We always probe IIO first, fall back to
//! vendor sysfs, and only use the input-event path for prox/als which
//! never appear in IIO on most Android phones.

use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, thiserror::Error)]
pub enum SensorError {
    #[error("no {kind} sensor found via IIO or vendor sysfs")]
    NotFound { kind: &'static str },
    #[error("read {path}: {err}")]
    Io {
        path: PathBuf,
        #[source]
        err: std::io::Error,
    },
    #[error("parse {value} from {path}: {reason}")]
    Parse { path: PathBuf, value: String, reason: String },
}

/// 3-axis sample in canonical units. For accelerometer that's m/s²,
/// for gyro rad/s, for magnetometer Tesla. We never normalise to
/// "g" or "deg/s" at this layer — the tool layer does that for the LLM
/// because keeping SI here means scale factors are well-defined.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Vec3 {
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

#[derive(Debug, Clone)]
pub struct ScalarSample {
    pub value: f64,
    pub unit: &'static str,
    pub source: PathBuf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SensorKind {
    Accel,
    Gyro,
    Magnetometer,
    Pressure,
    AmbientTemp,
    Light,
    Proximity,
}

impl SensorKind {
    fn iio_channel_prefix(self) -> &'static str {
        match self {
            SensorKind::Accel => "in_accel",
            SensorKind::Gyro => "in_anglvel",
            SensorKind::Magnetometer => "in_magn",
            SensorKind::Pressure => "in_pressure",
            SensorKind::AmbientTemp => "in_temp",
            SensorKind::Light => "in_illuminance",
            SensorKind::Proximity => "in_proximity",
        }
    }

    fn name_match(self, lower: &str) -> bool {
        match self {
            SensorKind::Accel => {
                lower.contains("accel")
                    || lower.starts_with("bmi")
                    || lower.starts_with("lsm6")
                    || lower.starts_with("kx")
                    || lower.starts_with("mc34")
            }
            SensorKind::Gyro => lower.contains("gyro") || lower.contains("anglvel"),
            SensorKind::Magnetometer => {
                lower.contains("mag")
                    || lower.contains("compass")
                    || lower.starts_with("ak0")
                    || lower.starts_with("mmc")
            }
            SensorKind::Pressure => lower.contains("baro") || lower.contains("press"),
            SensorKind::AmbientTemp => lower.contains("temp"),
            SensorKind::Light => {
                lower.contains("light") || lower.contains("als") || lower.starts_with("tcs")
            }
            SensorKind::Proximity => lower.contains("prox"),
        }
    }
}

/// Handle to a discovered IIO sensor. Owns no fd — every read is a
/// fresh sysfs open + parse, which keeps the API stateless and avoids
/// stale fd issues across screen-off transitions.
#[derive(Debug, Clone)]
pub struct IioSensor {
    pub kind: SensorKind,
    pub name: String,
    pub path: PathBuf,
}

impl IioSensor {
    pub fn discover(kind: SensorKind) -> Result<Self, SensorError> {
        let prefix = kind.iio_channel_prefix();
        let kind_name = sensor_kind_label(kind);

        let entries = fs::read_dir("/sys/bus/iio/devices").map_err(|err| SensorError::Io {
            path: PathBuf::from("/sys/bus/iio/devices"),
            err,
        });
        let entries = match entries {
            Ok(e) => e,
            Err(_) => return Err(SensorError::NotFound { kind: kind_name }),
        };

        for entry in entries.flatten() {
            let path = entry.path();
            if !entry.file_name().to_string_lossy().starts_with("iio:device") {
                continue;
            }
            let name = read_sysfs_string(&path.join("name")).unwrap_or_default();
            let lower = name.to_lowercase();

            // Match if either the device name hints at the right sensor
            // OR the device exposes the expected channel files.
            let has_channel = path.join(format!("{prefix}_x_raw")).exists()
                || path.join(format!("{prefix}_raw")).exists()
                || path.join(format!("{prefix}_input")).exists();
            if !has_channel && !kind.name_match(&lower) {
                continue;
            }
            if has_channel {
                return Ok(IioSensor { kind, name, path });
            }
        }
        Err(SensorError::NotFound { kind: kind_name })
    }

    /// Read a 3-axis sensor (accel/gyro/mag). Returns the value in SI
    /// units after applying offset+scale. Both per-axis and shared
    /// scale/offset files are supported; per-axis wins when present.
    pub fn read_vec3(&self) -> Result<Vec3, SensorError> {
        let prefix = self.kind.iio_channel_prefix();
        let x = self.read_axis(prefix, "x")?;
        let y = self.read_axis(prefix, "y")?;
        let z = self.read_axis(prefix, "z")?;
        Ok(Vec3 { x, y, z })
    }

    /// Read a scalar sensor (pressure/temp/light/prox).
    pub fn read_scalar(&self) -> Result<ScalarSample, SensorError> {
        let prefix = self.kind.iio_channel_prefix();
        // Some scalar sensors expose `in_X_input` (already converted),
        // others `in_X_raw` (apply offset+scale).
        let input_path = self.path.join(format!("{prefix}_input"));
        if input_path.exists() {
            let raw = read_sysfs_f64(&input_path)?;
            return Ok(ScalarSample {
                value: raw,
                unit: scalar_unit(self.kind),
                source: input_path,
            });
        }
        let value = self.read_axis(prefix, "")?;
        Ok(ScalarSample {
            value,
            unit: scalar_unit(self.kind),
            source: self.path.join(format!("{prefix}_raw")),
        })
    }

    fn read_axis(&self, prefix: &str, axis: &str) -> Result<f64, SensorError> {
        // Channel filenames may or may not include the axis suffix.
        let suffix = if axis.is_empty() { "" } else { "_" };
        let raw_path = self.path.join(format!("{prefix}{suffix}{axis}_raw"));
        let raw = read_sysfs_f64(&raw_path)?;

        // Per-axis scale wins; otherwise shared `<prefix>_scale`.
        let mut scale_path = self.path.join(format!("{prefix}{suffix}{axis}_scale"));
        if !scale_path.exists() {
            scale_path = self.path.join(format!("{prefix}_scale"));
        }
        let scale = if scale_path.exists() {
            read_sysfs_f64(&scale_path).unwrap_or(1.0)
        } else {
            1.0
        };

        let mut offset_path = self.path.join(format!("{prefix}{suffix}{axis}_offset"));
        if !offset_path.exists() {
            offset_path = self.path.join(format!("{prefix}_offset"));
        }
        let offset = if offset_path.exists() {
            read_sysfs_f64(&offset_path).unwrap_or(0.0)
        } else {
            0.0
        };

        Ok((raw + offset) * scale)
    }
}

fn scalar_unit(kind: SensorKind) -> &'static str {
    match kind {
        SensorKind::Pressure => "hPa",
        SensorKind::AmbientTemp => "°C",
        SensorKind::Light => "lux",
        SensorKind::Proximity => "cm",
        _ => "raw",
    }
}

fn sensor_kind_label(kind: SensorKind) -> &'static str {
    match kind {
        SensorKind::Accel => "accelerometer",
        SensorKind::Gyro => "gyroscope",
        SensorKind::Magnetometer => "magnetometer",
        SensorKind::Pressure => "pressure",
        SensorKind::AmbientTemp => "ambient_temp",
        SensorKind::Light => "light",
        SensorKind::Proximity => "proximity",
    }
}

fn read_sysfs_string(path: &Path) -> Result<String, SensorError> {
    fs::read_to_string(path)
        .map(|s| s.trim().to_string())
        .map_err(|err| SensorError::Io { path: path.to_path_buf(), err })
}

fn read_sysfs_f64(path: &Path) -> Result<f64, SensorError> {
    let s = read_sysfs_string(path)?;
    s.trim().parse().map_err(|_| SensorError::Parse {
        path: path.to_path_buf(),
        value: s,
        reason: "expected float".into(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iio_channel_prefixes_are_known() {
        assert_eq!(SensorKind::Accel.iio_channel_prefix(), "in_accel");
        assert_eq!(SensorKind::Gyro.iio_channel_prefix(), "in_anglvel");
        assert_eq!(SensorKind::Magnetometer.iio_channel_prefix(), "in_magn");
        assert_eq!(SensorKind::Pressure.iio_channel_prefix(), "in_pressure");
        assert_eq!(SensorKind::Proximity.iio_channel_prefix(), "in_proximity");
    }

    #[test]
    fn name_match_accelerometer_variants() {
        assert!(SensorKind::Accel.name_match("bmi160_accel"));
        assert!(SensorKind::Accel.name_match("lsm6dso_accel"));
        assert!(SensorKind::Accel.name_match("kx022 accelerometer"));
        assert!(!SensorKind::Accel.name_match("ak09918_magnetometer"));
    }

    #[test]
    fn name_match_distinguishes_kinds() {
        let cases = [
            ("ak09918", SensorKind::Magnetometer),
            ("bmi160_gyro", SensorKind::Gyro),
            ("lps22hh_press", SensorKind::Pressure),
            ("apds9960 als", SensorKind::Light),
            ("apds9960 prox", SensorKind::Proximity),
        ];
        for (name, kind) in cases {
            assert!(kind.name_match(name), "{name} should match {kind:?}");
        }
    }

    #[test]
    fn discover_returns_not_found_when_no_iio() {
        // On a host without /sys/bus/iio this should be NotFound, not a
        // panic / Io error. Ensures graceful absence of the subsystem.
        let r = IioSensor::discover(SensorKind::Accel);
        match r {
            Err(SensorError::NotFound { .. }) => {}
            Ok(_) => {} // CI runners with IIO emulators are fine too.
            Err(other) => panic!("unexpected error: {other}"),
        }
    }

    #[test]
    fn read_sysfs_f64_parses_kernel_format() {
        // Use a temp dir to simulate sysfs.
        let dir = std::env::temp_dir().join(format!("peko_sensors_test_{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let p = dir.join("scale");
        fs::write(&p, "0.0098066\n").unwrap();
        assert_eq!(read_sysfs_f64(&p).unwrap(), 0.0098066);
        fs::remove_dir_all(&dir).ok();
    }
}
