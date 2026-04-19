use peko_core::tool::{Tool, ToolResult};
use serde_json::json;
use std::future::Future;
use std::pin::Pin;
use std::process::{Command, Stdio};

use crate::screen_state::{enter_pin_now, has_lock_pin};

/// One-shot "get past the lockscreen" tool.
///
/// Without this, the agent typically fumbles through:
///   press POWER → screenshot → say "I see lockscreen" → swipe → ???
/// which wastes iterations AND trips the safety rail in `ensure_awake`
/// (screen awake → don't type PIN → agent is stuck on credential prompt).
///
/// With this, the agent reads "unlock_device" from the tool list and
/// calls it as a single iteration. The runtime side handles wake,
/// keyguard-dismiss, and PIN entry atomically.
pub struct UnlockDeviceTool;

impl UnlockDeviceTool {
    pub fn new() -> Self { Self }
}

impl Default for UnlockDeviceTool {
    fn default() -> Self { Self::new() }
}

impl Tool for UnlockDeviceTool {
    fn name(&self) -> &str { "unlock_device" }

    fn description(&self) -> &str {
        "Wake the phone and unlock the lockscreen. Wakes the display, \
         dismisses any keyguard overlay, and types the saved PIN + ENTER \
         if one is configured under [security].lock_pin. Call this FIRST \
         when the user asks to wake, log in, unlock, or open the device — \
         one call replaces the key_event + screenshot + swipe + type_pin \
         sequence the agent would otherwise improvise."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({ "type": "object", "properties": {}, "required": [] })
    }

    fn execute(
        &self,
        _args: serde_json::Value,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<ToolResult>> + Send + '_>> {
        Box::pin(async move {
            let pin_available = has_lock_pin();
            let pin_sent = enter_pin_now();

            // Best-effort post-check: did we actually land on the home screen?
            // On LineageOS 20 most dumpsys fields lie about keyguard state
            // (isKeyguardShowing stays true even on home), but the focused
            // WINDOW/APP reliably flips from NotificationShade+keyguard to the
            // launcher once the keyguard is actually dismissed. If the focus
            // is still NotificationShade the keyguard didn't come down, no
            // matter what our wake sequence reported.
            let focus = Command::new("sh")
                .arg("-c")
                .arg("dumpsys window | grep -E '^  mCurrentFocus' | head -1")
                .stdin(Stdio::null()).stderr(Stdio::null())
                .output()
                .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
                .unwrap_or_default();
            let still_on_keyguard = focus.contains("NotificationShade")
                || focus.contains("Keyguard");

            let (content, is_error) = match (pin_sent, still_on_keyguard) {
                (_, false) =>
                    ("Device unlocked — woke the display, dismissed keyguard, reached the home screen.".to_string(), false),
                (true, true) =>
                    ("Typed the configured PIN but the keyguard is still up. Either the PIN is wrong, \
                      or the ROM is blocking injected input on the credential screen. Check \
                      security.lock_pin and try again, or unlock the phone manually.".to_string(), true),
                (false, true) if pin_available =>
                    ("Woke the screen but couldn't enter the PIN — the `input` binary may be blocked \
                      by sepolicy. Check dmesg for avc denials against comm=cmd.".to_string(), true),
                (false, true) =>
                    ("Woke the screen but the keyguard is still showing. No PIN is configured — on \
                      LineageOS and many other Android 13 ROMs the swipe-to-unlock keyguard filters \
                      programmatic input for security. Fix by one of: \
                      (a) Settings > Security > Screen lock = None, \
                      (b) set a PIN and add it under Config > Security > lock_pin, \
                      (c) swipe the phone manually once then leave it on.".to_string(), true),
            };
            Ok(if is_error { ToolResult::error(content) } else { ToolResult::success(content) })
        })
    }
}
