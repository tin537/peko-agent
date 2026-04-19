//! Framework-path SMS tool.
//!
//! This replaces the AT-over-serial [`crate::SmsTool`] on devices where
//! RILD owns the modem channel (which is essentially every modern phone:
//! OnePlus, Pixel, Samsung, Xiaomi ≥ Android 10). Instead of talking to
//! `/dev/smd11` directly, we dispatch to a bundled priv-app
//! (`com.peko.shim.sms`) via `am broadcast`. The shim then calls the
//! real `SmsManager.sendTextMessage()` API, which goes through
//! `ISms → TelephonyManager → RILD → modem` — the same path the stock
//! Messages app uses. See `android/peko-sms-shim/` for the receiver.
//!
//! ## Wire protocol
//!
//! Send side (agent → shim):
//! ```text
//! am broadcast -n com.peko.shim.sms/.SmsCommandReceiver \
//!     -a com.peko.shim.sms.SEND \
//!     --es id <uuid> --es to <phone> --es body <text>
//! ```
//!
//! Receive side (shim → agent) — flat JSON at `/data/peko/sms_out/<id>.json`:
//! ```json
//! { "id": "...", "status": "queued|sent|delivered|error", "ts": ...,
//!   "to": "+...", "body_len": N, "error": "..." }
//! ```
//!
//! The shim writes "queued" as soon as `SmsManager` accepts the call,
//! then upgrades the same file to "sent" / "delivered" / "error" as the
//! radio callbacks fire. We poll the file and settle on whatever terminal
//! state arrives within the configured timeout (default 15s).
//!
//! ## Safety rails
//!
//! SMS costs real money. This tool is marked `is_dangerous=true` and
//! additionally enforces:
//!
//!   - A sliding-window rate limit (hour + day caps from [`SmsConfig`])
//!   - Basic E.164-ish phone number validation — digits and an optional
//!     leading `+`, nothing else, 6–16 characters total. Catches the
//!     agent fat-fingering `send to Alice` as a number.
//!   - Body length cap at 1000 chars — single SMS is 160 GSM-7 / 70 UCS-2,
//!     SmsManager splits above that but we don't want to silently send
//!     a 20-part message.
//!   - Audit log at `/data/peko/sms_sent.log` so you can reconcile with
//!     the carrier bill. Appended to on every attempt regardless of
//!     outcome.

use peko_config::SmsConfig;
use peko_core::tool::{Tool, ToolResult};
use serde_json::json;
use std::collections::VecDeque;
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;
use tokio::time::sleep;
use uuid::Uuid;

/// Where the shim writes result files. Mirrors the constant in
/// `SmsCommandReceiver.kt`; must be kept in sync if ever changed.
const RESULT_DIR: &str = "/data/peko/sms_out";

/// Flat-file audit trail. Append-only; peko-agent creates the file at
/// startup if missing, and we never rotate — operators are expected to
/// check it when reconciling carrier bills. Format: one JSON object per
/// line.
const AUDIT_LOG: &str = "/data/peko/sms_sent.log";

pub struct SmsFrameworkTool {
    cfg: SmsConfig,
    /// Sliding-window timestamps of attempted sends (successful or
    /// not — we want the limit to catch runaway retry loops too).
    history: Arc<Mutex<VecDeque<Instant>>>,
}

impl SmsFrameworkTool {
    pub fn new(cfg: SmsConfig) -> Self {
        // Make the result + audit paths exist so we don't race with the
        // shim on the first send. 0770 gives the shim's appuid (granted
        // system-like via priv-app) plus root write; peko-agent runs as
        // root under Magisk so it can create these regardless.
        let _ = std::fs::create_dir_all(RESULT_DIR);
        let _ = std::fs::File::options().create(true).append(true).open(AUDIT_LOG);
        Self {
            cfg,
            history: Arc::new(Mutex::new(VecDeque::with_capacity(64))),
        }
    }

    /// Prune timestamps older than 24h and return (per_hour, per_day).
    /// Caller holds the history lock for mutation.
    fn usage(history: &mut VecDeque<Instant>) -> (u32, u32) {
        let now = Instant::now();
        let hour_ago = now.checked_sub(Duration::from_secs(3600));
        let day_ago  = now.checked_sub(Duration::from_secs(86_400));

        // Drop anything older than a day; counts for the hour window are
        // derived from the tail slice since VecDeque is chronologically
        // ordered.
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

impl Tool for SmsFrameworkTool {
    // Expose as "sms" — same name as the AT-based tool so the agent's
    // prompt doesn't need to know which backend is active. main.rs
    // picks whichever can initialise.
    fn name(&self) -> &str { "sms" }

    fn description(&self) -> &str {
        "Send an SMS text message to a phone number. Uses the Android \
         SmsManager framework via a bundled priv-app — works on stock \
         phones where the carrier's AT channel is blocked. \
         Rate-limited; calls may be rejected if the hourly/daily quota \
         is exceeded (see config [tools.sms_config])."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "to": {
                    "type": "string",
                    "description": "Destination phone number in E.164 form (e.g. +66812345678). Digits + optional leading '+', 6–16 chars."
                },
                "message": {
                    "type": "string",
                    "description": "Message body. Keep under 160 chars for a single SMS; up to 1000 allowed (will be split into parts by the carrier)."
                }
            },
            "required": ["to", "message"]
        })
    }

    fn is_dangerous(&self) -> bool { true }

    fn execute(
        &self,
        args: serde_json::Value,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<ToolResult>> + Send + '_>> {
        let cfg = self.cfg.clone();
        let history = self.history.clone();

        Box::pin(async move {
            let to = args["to"].as_str()
                .ok_or_else(|| anyhow::anyhow!("missing 'to' parameter"))?
                .trim();
            let message = args["message"].as_str()
                .ok_or_else(|| anyhow::anyhow!("missing 'message' parameter"))?;

            // ── Validation ──────────────────────────────────────────
            if !looks_like_phone_number(to) {
                return Ok(ToolResult::error(format!(
                    "'{}' doesn't look like a phone number. Use E.164 form \
                     (digits and optional leading '+', 6–16 chars).", to
                )));
            }
            if message.is_empty() {
                return Ok(ToolResult::error("message body is empty"));
            }
            if message.len() > 1000 {
                return Ok(ToolResult::error(format!(
                    "message too long ({} chars); cap is 1000. Split it yourself.",
                    message.len()
                )));
            }

            // ── Rate limit ──────────────────────────────────────────
            {
                let mut h = history.lock().await;
                let (per_hour, per_day) = Self::usage(&mut h);
                if per_hour >= cfg.max_per_hour {
                    return Ok(ToolResult::error(format!(
                        "hourly SMS quota hit ({}/{}). Raise tools.sms_config.max_per_hour or wait.",
                        per_hour, cfg.max_per_hour
                    )));
                }
                if per_day >= cfg.max_per_day {
                    return Ok(ToolResult::error(format!(
                        "daily SMS quota hit ({}/{}). Raise tools.sms_config.max_per_day or wait.",
                        per_day, cfg.max_per_day
                    )));
                }
                // Record NOW — before we send — so a crash mid-send still
                // counts against the quota on next boot once history is
                // rebuilt from audit log (future work).
                h.push_back(Instant::now());
            }

            // ── Shim presence ───────────────────────────────────────
            if !shim_installed() {
                return Ok(ToolResult::error(
                    "com.peko.shim.sms not installed. Rebuild the Magisk module with \
                     `./magisk/build-module.sh --with-sms-shim`, reinstall, and reboot. \
                     Until then, SMS send is unavailable on this device."
                ));
            }

            // ── Send ────────────────────────────────────────────────
            let id = Uuid::new_v4().to_string();
            audit_attempt(&id, to, message.len());

            let cmd = format!(
                "am broadcast -n com.peko.shim.sms/.SmsCommandReceiver \
                    -a com.peko.shim.sms.SEND \
                    --es id {id} \
                    --es to {} \
                    --es body {} \
                    >/dev/null 2>&1",
                shell_quote(to),
                shell_quote(message),
            );
            let sent = Command::new("sh").arg("-c").arg(&cmd)
                .stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null())
                .status()
                .map(|s| s.success()).unwrap_or(false);
            if !sent {
                return Ok(ToolResult::error("am broadcast to SMS shim failed (shell spawn error)"));
            }

            // ── Poll ────────────────────────────────────────────────
            let result_path = PathBuf::from(RESULT_DIR).join(format!("{id}.json"));
            let deadline = Instant::now() + Duration::from_secs(cfg.send_timeout_secs);
            // Poll every 250ms so a fast send (<1s) returns quickly but
            // we don't spin the CPU for longer waits. The `queued` state
            // arrives within ~100ms; "sent" typically within 2–5s; some
            // "delivered" never arrive (carriers vary), so we accept
            // "sent" as a terminal success state.
            loop {
                if Instant::now() >= deadline {
                    audit_outcome(&id, "timeout", None);
                    return Ok(ToolResult::error(format!(
                        "no response from SMS shim within {}s — either the broadcast \
                         didn't reach com.peko.shim.sms, or the radio is taking a long \
                         time. Check the audit log at {AUDIT_LOG}.",
                        cfg.send_timeout_secs
                    )));
                }
                if let Ok(bytes) = tokio::fs::read(&result_path).await {
                    // Use try_parse so we don't crash on a torn write.
                    if let Ok(v) = serde_json::from_slice::<serde_json::Value>(&bytes) {
                        let status = v.get("status").and_then(|s| s.as_str()).unwrap_or("?");
                        if status == "sent" || status == "delivered" {
                            audit_outcome(&id, status, None);
                            return Ok(ToolResult::success(format!(
                                "SMS {status} to {to}: \"{}\"",
                                truncate(message, 60)
                            )));
                        }
                        if status == "error" {
                            let err = v.get("error").and_then(|s| s.as_str())
                                .unwrap_or("unknown error from SMS shim");
                            audit_outcome(&id, "error", Some(err));
                            return Ok(ToolResult::error(format!("SMS failed: {err}")));
                        }
                        // "queued" or anything else → keep waiting for the
                        // terminal state. SmsCommandReceiver will upgrade
                        // the same file as the radio callbacks fire.
                    }
                }
                sleep(Duration::from_millis(250)).await;
            }
        })
    }
}

// ─── helpers ──────────────────────────────────────────────────────────

/// Very cheap validation. We're not trying to be libphonenumber; we
/// just want to reject obvious agent mistakes like passing a contact
/// name or an English sentence.
fn looks_like_phone_number(s: &str) -> bool {
    let stripped = s.strip_prefix('+').unwrap_or(s);
    (6..=16).contains(&stripped.len()) && stripped.bytes().all(|b| b.is_ascii_digit())
}

/// Quote a string for `am broadcast --es`. The `am` parser handles
/// single-quoted args well; we just need to escape embedded single
/// quotes. Bash's standard trick: close → escaped-quote → reopen.
fn shell_quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('\'');
    for c in s.chars() {
        if c == '\'' { out.push_str("'\\''"); } else { out.push(c); }
    }
    out.push('\'');
    out
}

fn shim_installed() -> bool {
    // `pm list packages` is cheap (~50ms) and doesn't need binder
    // manoeuvres — it's answered by PackageManagerService via a simple
    // query.
    Command::new("sh").arg("-c")
        .arg("pm list packages | grep -q '^package:com.peko.shim.sms$'")
        .stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null())
        .status()
        .map(|s| s.success()).unwrap_or(false)
}

fn audit_attempt(id: &str, to: &str, body_len: usize) {
    let line = format!(
        r#"{{"id":"{id}","ts":{},"event":"attempt","to":"{to}","body_len":{body_len}}}"#,
        chrono::Utc::now().timestamp_millis(),
    );
    append_line(AUDIT_LOG, &line);
}

fn audit_outcome(id: &str, status: &str, error: Option<&str>) {
    let err = error.map(|e| format!(r#","error":"{}""#, escape_json_str(e))).unwrap_or_default();
    let line = format!(
        r#"{{"id":"{id}","ts":{},"event":"outcome","status":"{status}"{err}}}"#,
        chrono::Utc::now().timestamp_millis(),
    );
    append_line(AUDIT_LOG, &line);
}

fn append_line(path: &str, line: &str) {
    use std::io::Write;
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(f, "{line}");
    }
}

fn escape_json_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '"'  => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            _    => out.push(c),
        }
    }
    out
}

fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n { s.to_string() } else {
        let mut it = s.chars();
        let head: String = (&mut it).take(n).collect();
        format!("{head}…")
    }
}
