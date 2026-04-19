use peko_core::tool::{Tool, ToolResult};
use serde_json::json;
use std::future::Future;
use std::pin::Pin;

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

            let msg = match (pin_available, pin_sent) {
                (true, true) => "Device unlocked — woke the display, dismissed keyguard, typed PIN + ENTER.",
                (false, _) => "No PIN configured; woke the display and dismissed the basic keyguard. \
                               If a credential prompt is still showing, set security.lock_pin in the Config tab.",
                (true, false) => "Tried to unlock but the PIN entry failed. Check that security.lock_pin \
                                  is a digits-only value in the config.",
            };
            Ok(ToolResult::success(msg.to_string()))
        })
    }
}
