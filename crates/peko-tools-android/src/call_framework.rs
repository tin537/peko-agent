//! Framework-path call tool.
//!
//! The legacy [`crate::CallTool`] uses AT+D / ATH / ATA over /dev/smd11.
//! That's a dead path on modern phones (RILD holds the AT channel
//! exclusively), same story as the old SmsTool.
//!
//! The framework path is much simpler than the SMS one: calls don't
//! require a default-app role, CALL_PHONE is a dangerous perm but NOT
//! hard-restricted on Android 13, and there's a shell command
//! (`am start -a android.intent.action.CALL -d tel:...`) that actually
//! works when invoked from root. No priv-app APK needed — peko-agent
//! already runs as root via Magisk, so it can fire the intent itself.
//!
//! Trade-offs vs. the SMS shim design:
//!   - A successful `dial` puts the device into the InCallUI. The user
//!     (or whoever's in earshot of the phone) sees the call screen
//!     immediately. That's fine for the "call mom" use case, less fine
//!     for "silent autonomous dialing" — which is why this tool is
//!     marked dangerous and rate-limited.
//!   - hangup / answer via `am broadcast` don't reliably work across
//!     OEMs (the stock dialer owns the call state). Those actions
//!     fall back to `input keyevent KEYCODE_ENDCALL` / KEYCODE_CALL,
//!     which talk to the keypad-level handler and usually do the right
//!     thing even on locked stock dialers.
//!
//! See the `test on-device` notes at the bottom for verification.

use peko_core::tool::{Tool, ToolResult};
use serde_json::json;
use std::collections::VecDeque;
use std::future::Future;
use std::pin::Pin;
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;

/// Separate rate pool from the SMS tool. Calls are cheaper per-attempt
/// but the blast-radius of a rogue loop is arguably worse (an accidental
/// call placed autonomously is a social problem, not just a billing one).
/// Defaults are deliberately tight; raise via a future [tools.call_config]
/// if you actually need autonomous dialing at scale.
const MAX_DIALS_PER_HOUR: u32 = 3;
const MAX_DIALS_PER_DAY:  u32 = 10;
const AUDIT_LOG: &str = "/data/peko/calls.log";

pub struct CallFrameworkTool {
    history: Arc<Mutex<VecDeque<Instant>>>,
}

impl CallFrameworkTool {
    pub fn new() -> Self {
        let _ = std::fs::File::options().create(true).append(true).open(AUDIT_LOG);
        Self { history: Arc::new(Mutex::new(VecDeque::with_capacity(16))) }
    }

    fn count(history: &mut VecDeque<Instant>) -> (u32, u32) {
        let now = Instant::now();
        let day_ago  = now.checked_sub(Duration::from_secs(86_400));
        let hour_ago = now.checked_sub(Duration::from_secs(3_600));
        if let Some(cutoff) = day_ago {
            while history.front().map_or(false, |t| *t < cutoff) {
                history.pop_front();
            }
        }
        let per_hour = hour_ago
            .map(|h| history.iter().filter(|t| **t >= h).count() as u32)
            .unwrap_or(history.len() as u32);
        let per_day = history.len() as u32;
        (per_hour, per_day)
    }
}

impl Default for CallFrameworkTool {
    fn default() -> Self { Self::new() }
}

impl Tool for CallFrameworkTool {
    fn name(&self) -> &str { "call" }

    fn description(&self) -> &str {
        "Make or end phone calls via Android's Telecom framework. \
         Actions: dial (place a call to a phone number), hangup (end \
         any active call), answer (pick up an incoming call). The phone \
         screen lights up during dialing — peko cannot silently call \
         someone. Rate-limited: small default caps to keep runaway \
         loops from burning real credits."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["dial", "hangup", "answer"],
                    "description": "What to do: dial a new call, end an active call, or pick up an incoming ring."
                },
                "number": {
                    "type": "string",
                    "description": "Phone number in E.164 form (e.g. +66812345678). Required for 'dial'; ignored for hangup/answer."
                }
            },
            "required": ["action"]
        })
    }

    fn is_dangerous(&self) -> bool { true }

    fn execute(
        &self,
        args: serde_json::Value,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<ToolResult>> + Send + '_>> {
        let history = self.history.clone();
        Box::pin(async move {
            let action = args["action"].as_str()
                .ok_or_else(|| anyhow::anyhow!("missing 'action'"))?;

            match action {
                "dial" => {
                    let number = args["number"].as_str()
                        .ok_or_else(|| anyhow::anyhow!("missing 'number' for dial"))?
                        .trim();
                    if !looks_like_phone_number(number) {
                        return Ok(ToolResult::error(format!(
                            "'{}' doesn't look like a phone number. Use E.164 form \
                             (digits and optional leading '+', 6–16 chars).", number
                        )));
                    }
                    {
                        let mut h = history.lock().await;
                        let (per_hour, per_day) = Self::count(&mut h);
                        if per_hour >= MAX_DIALS_PER_HOUR {
                            return Ok(ToolResult::error(format!(
                                "hourly dial quota hit ({}/{}) — wait before calling again",
                                per_hour, MAX_DIALS_PER_HOUR
                            )));
                        }
                        if per_day >= MAX_DIALS_PER_DAY {
                            return Ok(ToolResult::error(format!(
                                "daily dial quota hit ({}/{}) — try again tomorrow",
                                per_day, MAX_DIALS_PER_DAY
                            )));
                        }
                        h.push_back(Instant::now());
                    }
                    audit("dial", number);

                    // tel: URIs can contain a leading '+' which is fine
                    // for am; we escape nothing else because E.164 has
                    // no shell-special chars after validation.
                    let cmd = format!(
                        "am start -a android.intent.action.CALL -d tel:{}",
                        shell_quote_for_tel(number)
                    );
                    let ok = Command::new("sh").arg("-c").arg(&cmd)
                        .stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null())
                        .status().map(|s| s.success()).unwrap_or(false);
                    if !ok {
                        return Ok(ToolResult::error(
                            "failed to dispatch CALL intent — is the Dialer app present?"
                        ));
                    }
                    Ok(ToolResult::success(format!(
                        "Dialing {}. The call screen should be visible on the phone; \
                         the radio takes a few seconds to connect.", number
                    )))
                }
                "hangup" => {
                    audit("hangup", "");
                    // KEYCODE_ENDCALL (6) is honoured by the stock
                    // dialer, Google Phone, and OEM dialers. It's the
                    // most portable "hang up" primitive that doesn't
                    // require holding the Telecom role.
                    let ok = Command::new("sh").arg("-c").arg("input keyevent KEYCODE_ENDCALL")
                        .stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null())
                        .status().map(|s| s.success()).unwrap_or(false);
                    if ok {
                        Ok(ToolResult::success("Sent end-call keystroke. If a call was active, it's ending now."))
                    } else {
                        Ok(ToolResult::error("failed to send KEYCODE_ENDCALL"))
                    }
                }
                "answer" => {
                    audit("answer", "");
                    // KEYCODE_CALL (5) answers an incoming ring on all
                    // stock dialers. Has no effect if nothing is ringing.
                    let ok = Command::new("sh").arg("-c").arg("input keyevent KEYCODE_CALL")
                        .stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null())
                        .status().map(|s| s.success()).unwrap_or(false);
                    if ok {
                        Ok(ToolResult::success("Sent answer keystroke. If a call was ringing, it should be connected now."))
                    } else {
                        Ok(ToolResult::error("failed to send KEYCODE_CALL"))
                    }
                }
                _ => Ok(ToolResult::error(format!("unknown call action: {}", action))),
            }
        })
    }
}

fn looks_like_phone_number(s: &str) -> bool {
    let stripped = s.strip_prefix('+').unwrap_or(s);
    (6..=16).contains(&stripped.len()) && stripped.bytes().all(|b| b.is_ascii_digit())
}

/// E.164 numbers are safe for shell because they're only '+' and
/// digits, but we wrap defensively in single quotes anyway so any
/// future relaxation of the validator can't introduce an injection.
fn shell_quote_for_tel(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('\'');
    for c in s.chars() {
        if c == '\'' { out.push_str("'\\''"); } else { out.push(c); }
    }
    out.push('\'');
    out
}

fn audit(event: &str, target: &str) {
    use std::io::Write;
    let now = chrono::Utc::now().timestamp_millis();
    let safe_target = target.replace('"', "\\\"");
    let line = format!(r#"{{"ts":{},"event":"{}","target":"{}"}}"#, now, event, safe_target);
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(AUDIT_LOG) {
        let _ = writeln!(f, "{line}");
    }
}
