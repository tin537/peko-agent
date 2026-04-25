use peko_core::tool::{Tool, ToolResult};
use peko_hal::{
    alsa_topology, media_volume_get, mixer_get, mixer_list, mixer_set, tinymix_available,
    AudioError,
};
use serde_json::json;
use std::future::Future;
use std::pin::Pin;

pub struct AudioTool;

impl AudioTool {
    pub fn new() -> Self { Self }
}

impl Default for AudioTool {
    fn default() -> Self { Self::new() }
}

impl Tool for AudioTool {
    fn name(&self) -> &str { "audio" }

    fn description(&self) -> &str {
        "Inspect ALSA topology and control kernel mixer / media stream volumes. \
         Actions: info (cards + PCM devices), list_mixer, get_mixer, set_mixer, \
         get_volume (stream={music,ring,notification,alarm,...}). \
         PCM record/playback are deferred to Phase 5 (overlay APK shim)."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["info", "list_mixer", "get_mixer", "set_mixer", "get_volume"],
                    "description": "Operation to perform."
                },
                "control": {
                    "type": "string",
                    "description": "Mixer control name (for get_mixer/set_mixer)."
                },
                "value": {
                    "type": "string",
                    "description": "New mixer value (for set_mixer)."
                },
                "stream": {
                    "type": "string",
                    "description": "Audio stream name (for get_volume): music, ring, notification, alarm, voice."
                }
            },
            "required": ["action"]
        })
    }

    fn execute(
        &self,
        args: serde_json::Value,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<ToolResult>> + Send + '_>> {
        let action = args["action"].as_str().unwrap_or("").to_string();
        let control = args["control"].as_str().map(String::from);
        let value = args["value"].as_str().map(String::from);
        let stream = args["stream"].as_str().map(String::from);
        Box::pin(async move {
            tokio::task::spawn_blocking(move || dispatch(&action, control, value, stream))
                .await
                .map_err(|e| anyhow::anyhow!("audio task panicked: {e}"))?
        })
    }
}

fn dispatch(
    action: &str,
    control: Option<String>,
    value: Option<String>,
    stream: Option<String>,
) -> anyhow::Result<ToolResult> {
    match action {
        "info" => match alsa_topology() {
            Ok(t) => {
                let mut out = format!(
                    "ALSA: {} card(s), {} playback PCM(s), {} capture PCM(s).\n",
                    t.cards.len(),
                    t.pcm_playback.len(),
                    t.pcm_capture.len()
                );
                for c in &t.cards {
                    out.push_str(&format!("  card #{}: id={} ({})\n", c.index, c.id, c.name));
                }
                out.push_str(&format!(
                    "  tinymix: {}\n",
                    if tinymix_available() { "available" } else { "not on PATH" }
                ));
                Ok(ToolResult::success(out))
            }
            Err(AudioError::NoAlsa) => Ok(ToolResult::error(
                "ALSA not present (kernel without /proc/asound). \
                 No audio HAL on this device.".to_string(),
            )),
            Err(e) => Ok(ToolResult::error(format!("audio info failed: {e}"))),
        },
        "list_mixer" => match mixer_list() {
            Ok(controls) => {
                let mut out = format!("{} mixer control(s):\n", controls.len());
                for c in controls.iter().take(50) {
                    out.push_str(&format!(
                        "  [{}] {:<10} {:<48} = {}\n",
                        c.id, c.kind, c.name, c.value
                    ));
                }
                if controls.len() > 50 {
                    out.push_str(&format!("  … {} more (truncated)\n", controls.len() - 50));
                }
                Ok(ToolResult::success(out))
            }
            Err(AudioError::NoTinymix) => Ok(ToolResult::error(
                "tinymix not available — install or use info action".to_string(),
            )),
            Err(e) => Ok(ToolResult::error(format!("list_mixer failed: {e}"))),
        },
        "get_mixer" => {
            let Some(name) = control else {
                return Ok(ToolResult::error("missing 'control' parameter".to_string()));
            };
            match mixer_get(&name) {
                Ok(v) => Ok(ToolResult::success(format!("{name}: {v}"))),
                Err(AudioError::ControlNotFound { name }) => {
                    Ok(ToolResult::error(format!("control '{name}' not found")))
                }
                Err(e) => Ok(ToolResult::error(format!("get_mixer failed: {e}"))),
            }
        }
        "set_mixer" => {
            let Some(name) = control else {
                return Ok(ToolResult::error("missing 'control' parameter".to_string()));
            };
            let Some(v) = value else {
                return Ok(ToolResult::error("missing 'value' parameter".to_string()));
            };
            match mixer_set(&name, &v) {
                Ok(_) => Ok(ToolResult::success(format!("set {name} = {v}"))),
                Err(e) => Ok(ToolResult::error(format!("set_mixer failed: {e}"))),
            }
        }
        "get_volume" => {
            let stream = stream.unwrap_or_else(|| "music".to_string());
            match media_volume_get(&stream) {
                Ok(n) => Ok(ToolResult::success(format!("{stream} volume: {n}"))),
                Err(e) => Ok(ToolResult::error(format!("get_volume failed: {e}"))),
            }
        }
        "" => Ok(ToolResult::error("missing 'action' parameter".to_string())),
        other => Ok(ToolResult::error(format!(
            "unknown action '{other}'. valid: info, list_mixer, get_mixer, set_mixer, get_volume"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_lists_all_actions() {
        let t = AudioTool::new();
        let s = t.parameters_schema();
        let actions: Vec<&str> = s["properties"]["action"]["enum"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|v| v.as_str())
            .collect();
        for a in ["info", "list_mixer", "get_mixer", "set_mixer", "get_volume"] {
            assert!(actions.contains(&a), "schema missing {a}");
        }
    }
}
