//! Wake + lockscreen-dismiss helper shared by every UI tool.
//!
//! Peko runs as root on the device, so `input keyevent` and the various
//! `dumpsys` queries are available. Without this helper, a sleeping phone
//! causes:
//!   - `screencap` to return a black framebuffer (user-visible)
//!   - touch/swipe events to hit the dimmed overlay instead of the app
//!   - text_input to flow into a keyguard that's not accepting input
//!
//! The helper is a cheap pre-flight call — a single dumpsys grep when the
//! screen is already on adds ~20ms. When the screen is off it adds about
//! 600ms (wake + swipe-to-dismiss + settle). Good tradeoff for "the task
//! actually works even if you asked peko from across the room".
//!
//! Limitation: if the user set a PIN / pattern / password, swipe-to-dismiss
//! reveals the passcode screen but can't unlock it. Downstream UI actions
//! will fail until the user (or a future `unlock_device` tool that inputs
//! a stored PIN) dismisses the keyguard manually.

use std::process::Command;
use std::time::Duration;

/// Hard cap on how long we'll wait for `mWakefulness=Awake` after sending
/// KEYCODE_WAKEUP. Empirically 600–900ms on sdm845 (fajita); we pad to
/// 1.5s so other older SoCs have headroom. Above this we give up and
/// return — the caller sees a black screenshot and can report the issue.
const WAKE_TIMEOUT_MS: u64 = 1500;
/// Single-iteration poll interval while waiting for wakefulness to flip.
const POLL_INTERVAL_MS: u64 = 100;
/// Extra settle time AFTER the display reports Awake, so the compositor
/// has a few frames to render into the framebuffer before we read it.
/// Without this, direct /dev/graphics/fb0 reads return all-black because
/// the hardware is on but SurfaceFlinger hasn't pushed a frame yet.
const POST_WAKE_SETTLE_MS: u64 = 400;

/// Run a shell command and return trimmed stdout. Errors become empty
/// strings — callers treat "empty" the same as "didn't match".
fn sh(cmd: &str) -> String {
    Command::new("sh").arg("-c").arg(cmd).output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default()
}

/// Wake the screen and attempt to dismiss a non-secure lockscreen.
///
/// Always returns `true`. The tool calling us will surface real failures
/// (black screenshot, dead tap, etc.) if a secure keyguard blocks the
/// operation — we can't reliably detect "locked with PIN" across ROMs
/// anyway: on OnePlus 6T / Android 13 `dumpsys window` reports
/// `mDreamingLockscreen=true` and `isKeyguardShowing=true` even when the
/// user is actively tapping the home screen, so we'd false-positive.
///
/// Behaviour:
///   - if `mWakefulness=Awake` already, do nothing (fast path, ~20ms)
///   - otherwise send `KEYCODE_WAKEUP`, wait for display, swipe up to
///     dismiss the basic no-PIN lockscreen
pub fn ensure_awake() -> bool {
    // Shell out via `sh -c` rather than Command::new("input") so we
    // always pick up /system/bin — Command's own PATH resolution can
    // miss it on Magisk-seeded environments.
    //
    // Three commands, in order:
    //   1. KEYCODE_WAKEUP       — flip mWakefulness to Awake
    //   2. wm dismiss-keyguard  — remove the keyguard overlay. On sdm845
    //                              this is load-bearing: KEYCODE_WAKEUP
    //                              alone reports Awake but the display
    //                              hardware doesn't actually power up,
    //                              so /dev/graphics/fb0 stays all-zero.
    //                              `wm dismiss-keyguard` triggers the
    //                              real display transition.
    //   3. svc power stayon true — prevent auto-sleep mid-task. Only
    //                              effective while on USB power (which
    //                              is typical when peko is in use), and
    //                              idempotent so calling every time is
    //                              fine. We never clear it — no point.
    let _ = Command::new("sh").args(["-c", "\
        input keyevent KEYCODE_WAKEUP; \
        wm dismiss-keyguard; \
        svc power stayon true \
    "]).status();

    // Poll until PowerManager reports Awake. Usually 300–500ms after
    // dismiss-keyguard on sdm845.
    let deadline = std::time::Instant::now() + Duration::from_millis(WAKE_TIMEOUT_MS);
    while !is_awake() {
        if std::time::Instant::now() >= deadline { break; }
        std::thread::sleep(Duration::from_millis(POLL_INTERVAL_MS));
    }
    // Even after Awake, SurfaceFlinger needs a few frames to paint into
    // the framebuffer before direct reads become meaningful.
    std::thread::sleep(Duration::from_millis(POST_WAKE_SETTLE_MS));

    true
}

fn is_awake() -> bool {
    sh("dumpsys power | grep -E 'mWakefulness='").contains("Awake")
}
