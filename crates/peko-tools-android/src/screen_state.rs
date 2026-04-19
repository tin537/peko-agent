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

use std::process::{Command, Stdio};
use std::sync::RwLock;
use std::time::Duration;

use peko_hal::InputDevice;

/// Process-global lockscreen PIN, seeded from config.toml by main.rs
/// at startup and updated by the `/api/config` POST handler when the
/// user changes it in the UI. `None` = auto-unlock disabled; `Some(s)`
/// = try to type `s` + ENTER after waking the display.
///
/// RwLock rather than OnceLock because the UI save path needs to
/// replace it at runtime, not just once at boot.
static LOCK_PIN: RwLock<Option<String>> = RwLock::new(None);

/// Kernel-level swipe that bypasses InputManagerService.
///
/// This is the key escape hatch that makes peko "agent as OS" actually
/// deliver on that promise. `input swipe` / `input touchscreen swipe`
/// route through InputManagerService.injectInputEvent(), which tags
/// every event with FLAG_INJECTED. Android's keyguard window sets
/// `INPUT_FEATURE_DISABLE_USER_ACTIVITY` and filters injected events —
/// that's why userspace apps (and even root apps going through the
/// `input` binary) can't dismiss the keyguard on a non-credential ROM.
///
/// Writing raw `input_event` structs to /dev/input/event2 is a
/// different path entirely. InputReader in system_server picks them
/// up from the kernel input device and delivers them to windows
/// without the injected flag — they're indistinguishable from a
/// finger on the glass. The keyguard accepts them.
///
/// Opens a fresh fd each call because we don't want to lock-contend
/// with TouchTool on the shared Arc<Mutex<InputDevice>>, and opening
/// /dev/input/eventN is a kernel-local operation (~100µs).
fn raw_unlock_swipe() -> bool {
    let Ok(mut dev) = InputDevice::find_touchscreen() else { return false; };
    // Swipe from lower-middle (inside the touch area, above nav gesture
    // bar) all the way up to mid-screen. Duration 150ms — matches a
    // typical human fling; below 80ms the gesture detector rejects it
    // as "accidental glitch", above 400ms it's treated as a drag start.
    dev.inject_swipe(540, 2100, 540, 600, 150).is_ok()
}

/// Called at startup and whenever the user saves a new PIN through the
/// Config UI. Empty / whitespace / non-digit values are treated as
/// "disabled" so a partially-typed PIN doesn't lock us out.
pub fn set_lock_pin(pin: Option<String>) {
    let normalised = pin
        .map(|p| p.trim().to_string())
        .filter(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()));
    if let Ok(mut guard) = LOCK_PIN.write() {
        *guard = normalised;
    }
}

fn lock_pin() -> Option<String> {
    LOCK_PIN.read().ok().and_then(|g| g.clone())
}

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
///
/// CRITICAL fd hygiene: every `input` / `wm` / `svc` / `cmd` invocation
/// dispatches to system_server over binder, and system_server inherits
/// our stderr fd through the chain. Our stderr points at
/// /data/peko/peko.log (service.sh's `2>&1` redirect), and sepolicy
/// denies system_server writing to that file. Without the fix below,
/// system_server aborts the binder transaction when it can't emit
/// diagnostics, so every tool call silently no-ops.
///
/// We spawn with `Stdio::null()` on stderr at the Rust level — that
/// replaces the inherited fd before sh even sees it, so anything
/// further down the process tree (including async subprocesses spawned
/// by `cmd`) writes to /dev/null. Shell-level `2>/dev/null` alone isn't
/// enough because it only applies to the direct child, not to whatever
/// `input`/`cmd` fan out to internally.
fn sh(cmd: &str) -> String {
    Command::new("sh")
        .arg("-c").arg(cmd)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default()
}

/// Fire-and-forget variant — same fd hygiene as `sh`, but we don't need
/// the output. Used for side-effect commands (wake, dismiss, input text)
/// where we only care that they don't bleed errors into our log or trip
/// the sepolicy fd-inheritance trap.
fn sh_silent(cmd: &str) {
    let _ = Command::new("sh")
        .arg("-c").arg(cmd)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
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
    // Fast-path: if the display is already on, we assume the user or a
    // previous call has already dealt with the keyguard. Don't re-type
    // the PIN here — if there's an editable field focused, we'd inject
    // the PIN into it, which is exactly the kind of surprise we want
    // to avoid.
    let was_already_awake = is_awake();
    if was_already_awake {
        return true;
    }

    // Shell out via `sh -c` rather than Command::new("input") so we
    // always pick up /system/bin — Command's own PATH resolution can
    // miss it on Magisk-seeded environments.
    //
    // CRITICAL: redirect stdout+stderr to /dev/null. `input`, `wm`, and
    // `svc` all dispatch to system_server over binder. system_server
    // runs in u:r:system_server:s0 and inherits our file descriptors.
    // Without redirects, system_server tries to write status/error to
    // our fd 1/2 (which point to /data/peko/peko.log). SELinux denies
    // system_server → system_data_root_file WRITE, which aborts the
    // entire binder transaction with "Failed transaction (2147483646)".
    // Net effect before this fix: every unlock_device call silently
    // no-op'd while the agent was told everything worked.
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
    sh_silent("input keyevent KEYCODE_WAKEUP; svc power stayon true");

    // Poll until PowerManager reports Awake. Usually 300–500ms after
    // the keyevent on sdm845.
    let deadline = std::time::Instant::now() + Duration::from_millis(WAKE_TIMEOUT_MS);
    while !is_awake() {
        if std::time::Instant::now() >= deadline { break; }
        std::thread::sleep(Duration::from_millis(POLL_INTERVAL_MS));
    }
    // Even after Awake, SurfaceFlinger needs a few frames to paint into
    // the framebuffer before direct reads become meaningful.
    std::thread::sleep(Duration::from_millis(POST_WAKE_SETTLE_MS));

    // Kernel-level swipe-up to dismiss the no-credential keyguard.
    // `wm dismiss-keyguard` is unreliable on LineageOS 20 (keyguard
    // treats its own "dismiss" request as a hint and leaves the swipe
    // overlay up), and shell `input swipe` is filtered by the keyguard
    // window. Raw /dev/input/event2 writes reach the keyguard as if
    // they were a finger — this is the whole point of running as root.
    raw_unlock_swipe();
    std::thread::sleep(Duration::from_millis(POST_WAKE_SETTLE_MS));

    // PIN auto-entry. Only fires when:
    //   - we were dozing at entry (so we're coming out of a locked state,
    //     not mid-interaction — avoids typing into a focused TextField)
    //   - a numeric PIN is configured in [security].lock_pin
    // On non-secure devices the keyguard is already gone by now and the
    // PIN digits land nowhere useful (launcher eats them); on PIN-locked
    // devices they unlock the phone.
    if let Some(pin) = lock_pin() {
        // pin is already validated digits-only via set_lock_pin, so no
        // shell-escaping gymnastics needed. Small sleep between text and
        // ENTER so the IME framework commits the digits before we submit.
        let cmd = format!("input text {}; sleep 0.2; input keyevent KEYCODE_ENTER", pin);
        let _ = Command::new("sh").arg("-c").arg(&cmd).status();
        // Unlock animation on Android 13 / sdm845 is ~400ms; add headroom.
        std::thread::sleep(Duration::from_millis(600));
    }

    true
}

fn is_awake() -> bool {
    sh("dumpsys power | grep -E 'mWakefulness='").contains("Awake")
}

/// Explicit unlock — wakes the screen if needed, then always types the
/// configured PIN (even if the screen was already awake). Use this when
/// the caller INTENDS to unlock, as opposed to ensure_awake which is a
/// cautious pre-flight for any UI tool.
///
/// Returns:
///   Ok(true)  — PIN was sent
///   Ok(false) — no PIN configured, nothing to type (caller should
///               check whether the screen was simply on a no-PIN
///               lockscreen, which wm dismiss-keyguard already cleared)
pub fn enter_pin_now() -> bool {
    ensure_awake();
    let Some(pin) = lock_pin() else { return false; };
    // Use sh_silent so the child inherits /dev/null for stderr rather
    // than our peko.log fd — otherwise system_server's binder response
    // trips the sepolicy denial and the input transaction aborts.
    sh_silent(&format!("input text {}", pin));
    std::thread::sleep(Duration::from_millis(200));
    sh_silent("input keyevent KEYCODE_ENTER");
    std::thread::sleep(Duration::from_millis(600));
    true
}

/// Whether a lock PIN is currently configured. Exposed so the
/// unlock_device tool can tell the agent "no PIN configured — I dismissed
/// the basic swipe lock, that's all I can do" rather than lying about
/// a successful unlock.
pub fn has_lock_pin() -> bool {
    lock_pin().is_some()
}
