use peko_core::tool::{Tool, ToolResult};
use peko_hal::InputDevice;
use serde_json::json;
use std::future::Future;
use std::pin::Pin;
use std::process::{Command, Stdio};
use std::sync::Arc;
use tokio::sync::Mutex;

/// Preserved for frameworkless AOSP builds where no `input` binary exists.
/// In normal Android the raw path is rarely used — the shell path below
/// goes through InputManagerService and just works.
pub struct KeyEventTool {
    device: Arc<Mutex<InputDevice>>,
}

impl KeyEventTool {
    pub fn new(device: InputDevice) -> Self {
        Self { device: Arc::new(Mutex::new(device)) }
    }
}

/// Fire `input keyevent KEYCODE_<name>` and return whether the shell
/// reported success. We redirect all fds to /dev/null (see screen_state.rs)
/// so system_server's binder callbacks don't trip sepolicy on the inherited
/// peko.log fd.
fn shell_keyevent(key_name: &str) -> bool {
    let cmd = format!("input keyevent KEYCODE_{}", key_name.to_uppercase());
    Command::new("sh")
        .arg("-c").arg(&cmd)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

impl Tool for KeyEventTool {
    fn name(&self) -> &str { "key_event" }

    fn description(&self) -> &str {
        "Press a hardware key. Supported keys: HOME, BACK, POWER, VOLUME_UP, VOLUME_DOWN, ENTER"
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "key": {
                    "type": "string",
                    "enum": ["HOME", "BACK", "POWER", "VOLUME_UP", "VOLUME_DOWN", "ENTER"],
                    "description": "The key to press"
                }
            },
            "required": ["key"]
        })
    }

    fn execute(
        &self,
        args: serde_json::Value,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<ToolResult>> + Send + '_>> {
        let device = self.device.clone();
        Box::pin(async move {
            let key_name = args["key"].as_str()
                .ok_or_else(|| anyhow::anyhow!("missing 'key' parameter"))?
                .to_string();

            // Primary path: shell `input keyevent`. HOME/BACK/POWER are
            // routed through InputManagerService to the right dispatcher
            // (NavigationBar, PhoneWindowManager, etc) on every Android
            // ROM that has /system/bin/input. The raw evdev path can
            // inject into the wrong /dev/input node entirely — the
            // touchscreen doesn't advertise KEY_HOMEPAGE.
            if shell_keyevent(&key_name) {
                return Ok(ToolResult::success(format!(
                    "Pressed {} key (via input keyevent)", key_name
                )));
            }

            // Fallback path for frameworkless devices: find an input node
            // whose EV_KEY bitmap advertises the requested code, then
            // inject directly. Try each candidate code — HOME maps to
            // both KEY_HOMEPAGE (172, common on Android) and KEY_HOME
            // (102, legacy PC kbd layout).
            let codes = InputDevice::key_codes_for_name(&key_name);
            if codes.is_empty() {
                return Ok(ToolResult::error(format!("unknown key: {}", key_name)));
            }

            for &code in codes {
                if let Ok(mut dev) = InputDevice::find_device_with_key(code) {
                    if dev.inject_key(code).is_ok() {
                        return Ok(ToolResult::success(format!(
                            "Pressed {} key (raw /dev/input code {})", key_name, code
                        )));
                    }
                }
            }

            // Last resort: the touchscreen device we were given at boot.
            // Most touchscreens don't advertise HOME, but we try anyway —
            // better to attempt and surface failure than silently do
            // nothing.
            let raw = {
                let mut dev = device.lock().await;
                dev.inject_key(codes[0])
            };
            match raw {
                Ok(_) => Ok(ToolResult::success(format!(
                    "Pressed {} key (fallback to default input device)", key_name
                ))),
                Err(e) => Ok(ToolResult::error(format!(
                    "Failed to press {}: shell keyevent and all raw paths failed: {}",
                    key_name, e
                ))),
            }
        })
    }
}
