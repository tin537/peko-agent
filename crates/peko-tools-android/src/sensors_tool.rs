use peko_core::tool::{Tool, ToolResult};
use peko_hal::{
    capture_dumpsys_sensorservice, dumpsys_latest_for_type, read_battery, read_light_prox,
    BatteryError, DumpsysError, IioSensor, LightProxError, LightProxKind, SensorError, SensorKind,
};
use serde_json::json;
use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

/// `sensors` tool: exposes battery, IIO motion sensors, and the
/// input-subsystem light/proximity readers to the LLM. All work runs
/// in a `spawn_blocking` pool because sysfs reads on a slow eMMC can
/// take 10–30ms and we don't want to park tokio's main worker.
pub struct SensorsTool;

impl SensorsTool {
    pub fn new() -> Self { Self }
}

impl Default for SensorsTool {
    fn default() -> Self { Self::new() }
}

impl Tool for SensorsTool {
    fn name(&self) -> &str { "sensors" }

    fn description(&self) -> &str {
        "Read on-device sensors directly from kernel sysfs / input. \
         Supported `sensor` values: battery, accel, gyro, magnetometer, \
         pressure, ambient_temp, light, proximity. Returns SI / human \
         units. Each call is a fresh sample — no subscriptions."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "sensor": {
                    "type": "string",
                    "enum": [
                        "battery", "accel", "gyro", "magnetometer",
                        "pressure", "ambient_temp", "light", "proximity"
                    ],
                    "description": "Which sensor to read."
                },
                "timeout_ms": {
                    "type": "integer",
                    "description": "For light/proximity: how long to wait for an event (default 800)."
                }
            },
            "required": ["sensor"]
        })
    }

    fn execute(
        &self,
        args: serde_json::Value,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<ToolResult>> + Send + '_>> {
        Box::pin(async move {
            let sensor = args["sensor"].as_str().unwrap_or("").to_string();
            let timeout_ms = args["timeout_ms"].as_u64().unwrap_or(800);
            tokio::task::spawn_blocking(move || dispatch(&sensor, timeout_ms))
                .await
                .map_err(|e| anyhow::anyhow!("sensors task panicked: {e}"))?
        })
    }
}

fn dispatch(sensor: &str, timeout_ms: u64) -> anyhow::Result<ToolResult> {
    match sensor {
        "battery" => read_battery_tool(),
        "accel" => read_iio_vec3(SensorKind::Accel),
        "gyro" => read_iio_vec3(SensorKind::Gyro),
        "magnetometer" => read_iio_vec3(SensorKind::Magnetometer),
        "pressure" => read_iio_scalar(SensorKind::Pressure),
        "ambient_temp" => read_iio_scalar(SensorKind::AmbientTemp),
        "light" => read_light_or_prox(LightProxKind::Light, timeout_ms),
        "proximity" => read_light_or_prox(LightProxKind::Proximity, timeout_ms),
        "" => Ok(ToolResult::error("missing 'sensor' parameter".to_string())),
        other => Ok(ToolResult::error(format!(
            "unknown sensor '{other}'. valid: battery, accel, gyro, magnetometer, \
             pressure, ambient_temp, light, proximity"
        ))),
    }
}

fn read_battery_tool() -> anyhow::Result<ToolResult> {
    match read_battery() {
        Ok(s) => {
            let mut out = String::from("Battery state:");
            if let Some(c) = s.capacity_pct {
                out.push_str(&format!("\n  capacity: {c}%"));
            }
            out.push_str(&format!("\n  status: {}", s.status.as_str()));
            out.push_str(&format!("\n  health: {}", s.health.as_str()));
            if let Some(v) = s.voltage_v {
                out.push_str(&format!("\n  voltage: {v:.3} V"));
            }
            if let Some(c) = s.current_ma {
                out.push_str(&format!("\n  current: {c:.1} mA"));
            }
            if let Some(t) = s.temperature_c {
                out.push_str(&format!("\n  temperature: {t:.1} °C"));
            }
            if let Some(tech) = s.technology {
                out.push_str(&format!("\n  technology: {tech}"));
            }
            if let Some(p) = s.present {
                out.push_str(&format!("\n  present: {p}"));
            }
            Ok(ToolResult::success(out))
        }
        Err(BatteryError::NotFound { path }) => Ok(ToolResult::error(format!(
            "no battery node at {} — likely a non-mobile / emulator target",
            path.display()
        ))),
        Err(e) => Ok(ToolResult::error(format!("battery read failed: {e}"))),
    }
}

fn read_iio_vec3(kind: SensorKind) -> anyhow::Result<ToolResult> {
    // Path 1: IIO sysfs. Cheap, fully kernel-direct, works in Lane A.
    match IioSensor::discover(kind) {
        Ok(sensor) => match sensor.read_vec3() {
            Ok(v) => return Ok(ToolResult::success(format_vec3_iio(kind, &sensor.name, v))),
            Err(e) => {
                tracing::debug!(error = %e, "IIO read failed, falling back to dumpsys");
            }
        },
        Err(SensorError::NotFound { .. }) => {
            // Expected on Qualcomm devices — fall through to dumpsys.
        }
        Err(e) => return Ok(ToolResult::error(format!("IIO discover failed: {e}"))),
    }

    // Path 2: dumpsys sensorservice. Lane B only — needs framework. The
    // Sensor HAL keeps recent samples cached for any sensor with an
    // active subscriber.
    read_via_dumpsys(kind, /* expected_axes */ Some(3))
}

fn read_iio_scalar(kind: SensorKind) -> anyhow::Result<ToolResult> {
    match IioSensor::discover(kind) {
        Ok(sensor) => match sensor.read_scalar() {
            Ok(s) => {
                return Ok(ToolResult::success(format!(
                    "{} ({}): {:.3} {} (source: IIO {})",
                    kind_label(kind),
                    sensor.name,
                    s.value,
                    s.unit,
                    s.source.display()
                )))
            }
            Err(e) => {
                tracing::debug!(error = %e, "IIO scalar read failed, falling back to dumpsys");
            }
        },
        Err(SensorError::NotFound { .. }) => {}
        Err(e) => return Ok(ToolResult::error(format!("IIO discover failed: {e}"))),
    }

    read_via_dumpsys(kind, /* expected_axes */ Some(1))
}

fn read_via_dumpsys(kind: SensorKind, expected_axes: Option<usize>) -> anyhow::Result<ToolResult> {
    let Some(type_id) = kind.android_type_id() else {
        return Ok(ToolResult::error(format!(
            "{} has no Android sensor type id; not readable via dumpsys",
            kind_label(kind)
        )));
    };
    let label = kind_label(kind);

    let text = match capture_dumpsys_sensorservice() {
        Ok(t) => t,
        Err(DumpsysError::NotAvailable) => {
            return Ok(ToolResult::error(format!(
                "{label} not exposed via IIO and dumpsys is unavailable \
                 (frameworkless build?). Lane A access requires a binder \
                 client to the Sensor HAL — not yet implemented."
            )))
        }
        Err(e) => return Ok(ToolResult::error(format!("dumpsys failed: {e}"))),
    };

    match dumpsys_latest_for_type(&text, type_id, label) {
        Ok(reading) => Ok(ToolResult::success(format_dumpsys_reading(
            kind,
            &reading,
            expected_axes,
        ))),
        Err(DumpsysError::SensorNotListed { type_id, label }) => Ok(ToolResult::error(format!(
            "{label} (type {type_id}) is not present in dumpsys sensorservice — \
             this device doesn't expose it"
        ))),
        Err(DumpsysError::NoRecentEvents { name }) => Ok(ToolResult::error(format!(
            "sensor '{name}' has no recent events. Open the Camera or another app \
             that uses {label} for a few seconds, then retry — dumpsys only sees \
             cached samples from active subscribers."
        ))),
        Err(e) => Ok(ToolResult::error(format!("dumpsys parse failed: {e}"))),
    }
}

fn format_vec3_iio(kind: SensorKind, name: &str, v: peko_hal::Vec3) -> String {
    let unit = match kind {
        SensorKind::Accel => "m/s²",
        SensorKind::Gyro => "rad/s",
        SensorKind::Magnetometer => "T",
        _ => "raw",
    };
    format!(
        "{} ({}): x={:.4} y={:.4} z={:.4} {} (source: IIO)",
        kind_label(kind),
        name,
        v.x, v.y, v.z,
        unit
    )
}

fn format_dumpsys_reading(
    kind: SensorKind,
    reading: &peko_hal::DumpsysReading,
    expected_axes: Option<usize>,
) -> String {
    let unit = match kind {
        SensorKind::Accel | SensorKind::Gyro => match kind {
            SensorKind::Accel => "m/s²",
            _ => "rad/s",
        },
        SensorKind::Magnetometer => "µT",
        SensorKind::Light => "lux",
        SensorKind::Proximity => "cm",
        SensorKind::Pressure => "hPa",
        SensorKind::AmbientTemp => "°C",
    };
    let wall = reading.wall_clock.as_deref().unwrap_or("?");
    let vals = &reading.values;
    let formatted = match (expected_axes.unwrap_or(0), vals.len()) {
        (3, _) if vals.len() >= 3 => {
            format!("x={:.4} y={:.4} z={:.4}", vals[0], vals[1], vals[2])
        }
        (1, _) if !vals.is_empty() => format!("{:.4}", vals[0]),
        _ => vals
            .iter()
            .map(|v| format!("{v:.4}"))
            .collect::<Vec<_>>()
            .join(", "),
    };
    format!(
        "{} ({}): {} {} (source: dumpsys, wall={})",
        kind_label(kind),
        reading.sensor_name,
        formatted,
        unit,
        wall
    )
}

fn read_light_or_prox(kind: LightProxKind, timeout_ms: u64) -> anyhow::Result<ToolResult> {
    // Path 1: input subsystem / /sys/class/sensors. Lane A friendly.
    match read_light_prox(kind, Duration::from_millis(timeout_ms)) {
        Ok(r) => {
            return Ok(ToolResult::success(format!(
                "{}: {:.3} {} (source: {})",
                light_prox_label(kind),
                r.value,
                r.unit,
                r.source.display()
            )))
        }
        Err(LightProxError::NotFound { .. }) | Err(LightProxError::Timeout) => {
            // Fall through to dumpsys. Don't surface the input-path
            // error if the dumpsys path succeeds — the agent only cares
            // about getting the value.
        }
        Err(e) => return Ok(ToolResult::error(format!("read failed: {e}"))),
    }

    // Path 2: dumpsys sensorservice (Lane B). On Qualcomm devices the
    // optical sensors are SLPI-bound; this is the only userspace path.
    let mapped = match kind {
        LightProxKind::Light => SensorKind::Light,
        LightProxKind::Proximity => SensorKind::Proximity,
    };
    read_via_dumpsys(mapped, /* expected_axes */ Some(1))
}

fn kind_label(kind: SensorKind) -> &'static str {
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

fn light_prox_label(k: LightProxKind) -> &'static str {
    match k {
        LightProxKind::Light => "light",
        LightProxKind::Proximity => "proximity",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_matches_supported_sensors() {
        let t = SensorsTool::new();
        let s = t.parameters_schema();
        let arr = s["properties"]["sensor"]["enum"].as_array().unwrap();
        let names: Vec<&str> = arr.iter().filter_map(|v| v.as_str()).collect();
        for must in [
            "battery",
            "accel",
            "gyro",
            "magnetometer",
            "pressure",
            "ambient_temp",
            "light",
            "proximity",
        ] {
            assert!(names.contains(&must), "schema missing {must}");
        }
    }
}
