//! Parse `dumpsys sensorservice` output to read sensor values without
//! talking to the binder Sensor HAL directly.
//!
//! On Qualcomm devices (sdm845 + similar) all motion / light / prox /
//! pressure sensors live behind SLPI and are NOT exposed via /sys/bus/iio
//! or /sys/class/sensors. The only userspace path is the Sensor HAL
//! binder service. Calling that from Rust would mean rolling our own
//! libbinder client — months of work.
//!
//! `dumpsys sensorservice` already includes a "Recent Sensor events"
//! section with the last N samples for every active sensor on the
//! device. Reading it is a stateless shell call returning text. We
//! parse three sections:
//!
//!   1. `Sensor List:`  — handle, name, type_id, vendor.
//!   2. `Recent Sensor events:` — per-sensor recent samples.
//!   3. We pick the latest sample for the requested kind by mapping
//!      kind → Android sensor type id → matching sensor by name.
//!
//! Caveat: a sensor that has no active subscriber on the device shows
//! no recent events. The agent gets a clear "no recent events"
//! error rather than fake data. In practice the framework keeps accel
//! + prox + mag busy almost always, so this only bites uncommon
//! sensors.

use std::collections::HashMap;
use std::process::{Command, Stdio};

#[derive(Debug, thiserror::Error)]
pub enum DumpsysError {
    #[error("dumpsys not available — is the framework running?")]
    NotAvailable,
    #[error("dumpsys sensorservice exited non-zero: {0}")]
    Failed(String),
    #[error("no sensor of type {type_id} ({label}) in dumpsys output")]
    SensorNotListed { type_id: u32, label: &'static str },
    #[error("sensor '{name}' has no recent events (likely no active subscriber)")]
    NoRecentEvents { name: String },
    #[error("could not parse event line: {0}")]
    Parse(String),
}

#[derive(Debug, Clone)]
pub struct DumpsysSensor {
    pub handle: u32,
    pub name: String,
    pub vendor: String,
    pub type_id: u32,
}

#[derive(Debug, Clone)]
pub struct DumpsysReading {
    pub sensor_name: String,
    pub values: Vec<f64>,
    pub wall_clock: Option<String>,
}

/// Run `dumpsys sensorservice` and return the captured stdout.
/// Bounded by std `Command` semantics (no built-in timeout); call sites
/// run this inside `spawn_blocking` so a stuck dumpsys doesn't park
/// the tokio reactor.
pub fn capture() -> Result<String, DumpsysError> {
    let out = Command::new("dumpsys")
        .arg("sensorservice")
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .map_err(|_| DumpsysError::NotAvailable)?;
    if !out.status.success() {
        return Err(DumpsysError::Failed(
            String::from_utf8_lossy(&out.stderr).to_string(),
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).to_string())
}

/// Parse the `Sensor List:` block. Each entry looks like
///   `0x0000000b) lsm6ds3c Accelerometer Non-wakeup | STMicro         | ver: 352 | type: android.sensor.accelerometer(1) | ...`
pub fn parse_sensor_list(text: &str) -> Vec<DumpsysSensor> {
    let mut out = Vec::new();
    let mut in_list = false;
    for line in text.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("Sensor List:") {
            in_list = true;
            continue;
        }
        if !in_list {
            continue;
        }
        // Section ends at the next non-indented header (Fusion States,
        // Recent Sensor events, etc.) — those start at column 0 and
        // don't begin with "0x".
        if !trimmed.starts_with("0x") {
            // Continuation lines (capability info) start with a tab; skip them.
            if line.starts_with('\t') {
                continue;
            }
            // Real header — done.
            break;
        }

        // Expected: "0xHANDLE) NAME | VENDOR | ver: N | type: android.sensor.X(ID) | ..."
        let Some((handle_str, rest)) = trimmed.split_once(')') else { continue };
        let Ok(handle) = u32::from_str_radix(handle_str.trim_start_matches("0x"), 16) else { continue };

        let parts: Vec<&str> = rest.split('|').map(|s| s.trim()).collect();
        if parts.len() < 4 {
            continue;
        }
        let name = parts[0].to_string();
        let vendor = parts[1].to_string();

        // Find the "type:" segment and extract the parenthesised type id.
        let mut type_id: u32 = 0;
        for p in &parts {
            if let Some(ts) = p.strip_prefix("type:") {
                if let Some(start) = ts.rfind('(') {
                    if let Some(end) = ts[start + 1..].find(')') {
                        let id_str = &ts[start + 1..start + 1 + end];
                        if let Ok(n) = id_str.trim().parse::<u32>() {
                            type_id = n;
                            break;
                        }
                    }
                }
            }
        }

        out.push(DumpsysSensor {
            handle,
            name,
            vendor,
            type_id,
        });
    }
    out
}

/// Parse the `Recent Sensor events:` block into per-sensor latest reading.
/// Returns a map keyed by sensor name (verbatim, including "Non-wakeup"
/// suffix). Each value is the latest numbered event seen.
pub fn parse_recent_events(text: &str) -> HashMap<String, DumpsysReading> {
    let mut out = HashMap::new();
    let mut current_name: Option<String> = None;

    let mut in_section = false;
    for line in text.lines() {
        if line.starts_with("Recent Sensor events:") {
            in_section = true;
            continue;
        }
        if !in_section {
            continue;
        }

        // Section header per sensor: "<name>: last N events"
        if !line.starts_with('\t') && !line.starts_with(' ') {
            if let Some(name) = parse_sensor_event_header(line) {
                current_name = Some(name);
            } else if line.trim().is_empty() {
                continue;
            } else {
                // Another top-level header we don't handle.
                in_section = !line.contains("Pre-fusion") && !line.contains("Mode :");
            }
            continue;
        }

        // Numbered sample row: "<N> (ts=<...>, wall=<...>) v1, v2, v3, ..."
        let Some(name) = current_name.clone() else { continue };
        if let Some((wall, values)) = parse_event_row(line) {
            out.insert(
                name.clone(),
                DumpsysReading {
                    sensor_name: name,
                    values,
                    wall_clock: wall,
                },
            );
        }
    }
    out
}

fn parse_sensor_event_header(line: &str) -> Option<String> {
    // Format: "<sensor name>: last <N> events"
    let trimmed = line.trim();
    let (name, tail) = trimmed.rsplit_once(':')?;
    if !tail.trim().starts_with("last") {
        return None;
    }
    Some(name.trim().to_string())
}

fn parse_event_row(line: &str) -> Option<(Option<String>, Vec<f64>)> {
    // Find ", wall=...)" boundary to skip the timestamp prefix.
    let after_paren_idx = line.find(')')?;
    let header = &line[..after_paren_idx];
    let rest = line[after_paren_idx + 1..].trim();

    let wall = header
        .find("wall=")
        .map(|i| header[i + 5..].trim_end_matches(',').trim().to_string());

    let values: Vec<f64> = rest
        .split(',')
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .filter_map(|s| s.parse::<f64>().ok())
        .collect();
    if values.is_empty() {
        return None;
    }
    Some((wall, values))
}

/// Read the latest sample for the given Android sensor type. Picks the
/// first matching sensor by listing order, preferring non-wakeup names
/// because those are typically what the framework keeps subscribed.
pub fn latest_for_type(
    text: &str,
    type_id: u32,
    label: &'static str,
) -> Result<DumpsysReading, DumpsysError> {
    let sensors = parse_sensor_list(text);
    let recent = parse_recent_events(text);

    let mut candidates: Vec<&DumpsysSensor> =
        sensors.iter().filter(|s| s.type_id == type_id).collect();
    if candidates.is_empty() {
        return Err(DumpsysError::SensorNotListed { type_id, label });
    }
    // Prefer non-wakeup variant when both are listed.
    candidates.sort_by_key(|s| if s.name.to_lowercase().contains("non-wakeup") { 0 } else { 1 });

    for s in &candidates {
        if let Some(r) = recent.get(&s.name) {
            return Ok(r.clone());
        }
    }
    Err(DumpsysError::NoRecentEvents {
        name: candidates[0].name.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = include_str!("../../../tests/device-test/dumpsys_sensorservice_OnePlus6T.txt");

    #[test]
    fn sample_file_present_and_nonempty() {
        assert!(SAMPLE.contains("Sensor List:"), "sample fixture is missing");
        assert!(SAMPLE.contains("Recent Sensor events:"));
    }

    #[test]
    fn parses_sensor_list_handles_and_types() {
        let s = parse_sensor_list(SAMPLE);
        assert!(s.len() >= 30, "expected at least 30 sensors, got {}", s.len());
        let accel = s.iter().find(|x| x.handle == 0x0b).expect("0x0b accel");
        assert_eq!(accel.type_id, 1);
        assert!(accel.name.contains("Accelerometer"));
        assert!(accel.vendor.contains("STMicro"));
        let gyro = s.iter().find(|x| x.handle == 0x29).expect("0x29 gyro");
        assert_eq!(gyro.type_id, 4);
        let mag = s.iter().find(|x| x.handle == 0x15).expect("0x15 mag");
        assert_eq!(mag.type_id, 2);
        let prox = s.iter().find(|x| x.handle == 0x53).expect("0x53 prox");
        assert_eq!(prox.type_id, 8);
        let light = s.iter().find(|x| x.handle == 0x35).expect("0x35 light");
        assert_eq!(light.type_id, 5);
    }

    #[test]
    fn parses_recent_events_for_accelerometer() {
        let r = parse_recent_events(SAMPLE);
        let accel = r
            .get("lsm6ds3c Accelerometer Non-wakeup")
            .expect("recent accel events");
        assert_eq!(accel.values.len(), 3);
        // From the captured fixture, the latest event has values
        // (0.99, -0.55, 9.99) ± normal jitter — assert the magnitudes
        // are plausible (gravity-dominated, ~9.8 m/s² on z).
        assert!(accel.values[2].abs() > 8.0 && accel.values[2].abs() < 11.0);
    }

    #[test]
    fn parses_recent_events_for_magnetometer() {
        let r = parse_recent_events(SAMPLE);
        let mag = r
            .get("ak0991x Magnetometer Non-wakeup")
            .expect("recent mag events");
        assert_eq!(mag.values.len(), 3);
    }

    #[test]
    fn latest_for_type_picks_non_wakeup() {
        // Accelerometer type=1 has both wakeup (0x0c) and non-wakeup (0x0b)
        // entries; we prefer non-wakeup and it has recent events.
        let r = latest_for_type(SAMPLE, 1, "accelerometer").unwrap();
        assert!(r.sensor_name.contains("Non-wakeup"));
        assert_eq!(r.values.len(), 3);
    }

    #[test]
    fn latest_for_type_returns_error_for_missing_type() {
        // Type 6 = pressure barometer, not present in this device.
        let r = latest_for_type(SAMPLE, 6, "pressure");
        match r {
            Err(DumpsysError::SensorNotListed { type_id: 6, .. }) => {}
            other => panic!("expected SensorNotListed for type 6, got {other:?}"),
        }
    }

    #[test]
    fn parse_event_row_extracts_three_floats() {
        let line = "\t35 (ts=420098.977563937, wall=12:24:47.047) 1.01, -0.54, 10.01, ";
        let (wall, vals) = parse_event_row(line).unwrap();
        assert_eq!(wall.as_deref(), Some("12:24:47.047"));
        assert_eq!(vals, vec![1.01, -0.54, 10.01]);
    }

    #[test]
    fn parse_sensor_event_header_handles_colon_in_name() {
        // No real-world example with a colon in the name, but the
        // splitter uses rsplit_once so a colon in the prefix wouldn't
        // confuse it.
        let h = parse_sensor_event_header("Foo: Bar Sensor: last 5 events");
        assert_eq!(h.as_deref(), Some("Foo: Bar Sensor"));
    }
}
