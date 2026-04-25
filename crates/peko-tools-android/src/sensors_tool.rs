use peko_core::tool::{Tool, ToolResult};
use peko_hal::{
    read_battery, read_light_prox, BatteryError, IioSensor, LightProxError, LightProxKind,
    SensorError, SensorKind,
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
    let sensor = match IioSensor::discover(kind) {
        Ok(s) => s,
        Err(SensorError::NotFound { kind: k }) => {
            return Ok(ToolResult::error(format!(
                "{k} not exposed via IIO on this kernel. Try `screenshot mode=info` for hardware diagnostics."
            )))
        }
        Err(e) => return Ok(ToolResult::error(format!("IIO discover failed: {e}"))),
    };
    match sensor.read_vec3() {
        Ok(v) => {
            let unit = match kind {
                SensorKind::Accel => "m/s²",
                SensorKind::Gyro => "rad/s",
                SensorKind::Magnetometer => "T",
                _ => "raw",
            };
            Ok(ToolResult::success(format!(
                "{} ({}): x={:.4} y={:.4} z={:.4} {}",
                kind_label(kind),
                sensor.name,
                v.x, v.y, v.z,
                unit
            )))
        }
        Err(e) => Ok(ToolResult::error(format!("read failed: {e}"))),
    }
}

fn read_iio_scalar(kind: SensorKind) -> anyhow::Result<ToolResult> {
    let sensor = match IioSensor::discover(kind) {
        Ok(s) => s,
        Err(SensorError::NotFound { kind: k }) => {
            return Ok(ToolResult::error(format!(
                "{k} not exposed via IIO on this kernel."
            )))
        }
        Err(e) => return Ok(ToolResult::error(format!("IIO discover failed: {e}"))),
    };
    match sensor.read_scalar() {
        Ok(s) => Ok(ToolResult::success(format!(
            "{} ({}): {:.3} {}",
            kind_label(kind),
            sensor.name,
            s.value,
            s.unit
        ))),
        Err(e) => Ok(ToolResult::error(format!("read failed: {e}"))),
    }
}

fn read_light_or_prox(kind: LightProxKind, timeout_ms: u64) -> anyhow::Result<ToolResult> {
    match read_light_prox(kind, Duration::from_millis(timeout_ms)) {
        Ok(r) => Ok(ToolResult::success(format!(
            "{}: {:.3} {} (source: {})",
            light_prox_label(kind),
            r.value,
            r.unit,
            r.source.display()
        ))),
        Err(LightProxError::NotFound { kind: k }) => Ok(ToolResult::error(format!(
            "{k} sensor not found via input or sysfs"
        ))),
        Err(LightProxError::Timeout) => Ok(ToolResult::error(format!(
            "{} read timed out — sensor may need an event before reporting",
            light_prox_label(kind)
        ))),
        Err(e) => Ok(ToolResult::error(format!("read failed: {e}"))),
    }
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
