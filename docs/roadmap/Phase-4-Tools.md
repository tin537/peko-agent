# Phase 4: Tools

> Android tool implementations — giving the agent hands.

---

## Goal

Implement all [[../implementation/Tool-System|Tool trait]] implementations for Android, backed by [[../implementation/peko-hal|peko-hal]]. Each tool is tested individually, then integrated with the agent loop.

## Prerequisites

- [[Phase-1-Foundation]] completed (Tool trait, registry)
- [[Phase-3-Hardware]] completed (HAL wrappers)

## Tasks

### 4.1 ScreenshotTool

- [ ] Implement `Tool` trait with name "screenshot", empty input schema
- [ ] Capture via Framebuffer (primary) or DRM (fallback)
- [ ] Encode to PNG via `image` crate
- [ ] Base64-encode for LLM inclusion
- [ ] Optional: downscale to 720p for smaller payloads
- [ ] Test: take screenshot, decode base64, verify image

### 4.2 TouchTool

- [ ] Implement `Tool` trait with name "touch"
- [ ] Input schema: `action` (tap/long_press/swipe), `x`, `y`, optional `x2`, `y2`, `duration`
- [ ] Delegate to `InputDevice::inject_tap()` / `inject_swipe()`
- [ ] Test: agent-initiated tap on specific coordinates

### 4.3 KeyEventTool

- [ ] Input schema: `key` (HOME, BACK, POWER, VOLUME_UP, VOLUME_DOWN, ENTER)
- [ ] Map key names to Linux keycodes
- [ ] Inject via InputDevice
- [ ] Test: press HOME key → verify screen changes

### 4.4 TextInputTool

- [ ] Input schema: `text` (string to type)
- [ ] Inject individual key events for ASCII characters
- [ ] Handle special characters (shift, symbols)
- [ ] Test: type "hello world" into a text field

### 4.5 SmsTool

- [ ] Input schema: `to` (phone number), `message` (text)
- [ ] `is_dangerous() → true`
- [ ] Send via SerialModem AT commands
- [ ] Handle error cases (no signal, invalid number)
- [ ] Test: send real SMS (use test SIM or verify via modem response only)

### 4.6 CallTool

- [ ] Input schema: `action` (dial/hangup/answer), optional `number`
- [ ] `is_dangerous() → true`
- [ ] Implement via AT commands (ATD, ATH, ATA)
- [ ] Monitor call state via URCs
- [ ] Test: dial and immediately hang up

### 4.7 UiDumpTool

- [ ] Detect if framework is available (`getprop sys.boot_completed`)
- [ ] If yes: execute `uiautomator dump`
- [ ] If no: return error (rely on ScreenshotTool + vision)
- [ ] `is_available()` returns false in frameworkless mode
- [ ] Test: dump UI XML in hybrid mode

### 4.8 FileSystemTool

- [ ] Input schema: `action` (read/write/list/search/delete), `path`, optional `content`
- [ ] Implement path sandboxing (configurable allowed paths)
- [ ] Use `std::fs` operations
- [ ] `is_dangerous()` for write/delete actions
- [ ] Test: read, write, list files in `/data/peko/`

### 4.9 ShellTool

- [ ] Input schema: `command`, optional `timeout`
- [ ] `is_dangerous() → true`
- [ ] Execute via `tokio::process::Command`
- [ ] Capture stdout + stderr, enforce timeout
- [ ] Test: run `ls -la`, verify output

### 4.10 NotificationTool

- [ ] Check LED state via `/sys/class/leds/`
- [ ] If framework available: `dumpsys notification`
- [ ] Return structured notification info
- [ ] Test: read current notification state

### 4.11 Integration: Agent + Real Tools

- [ ] Register all tools in a test binary on device
- [ ] Connect to real Anthropic API (via WiFi)
- [ ] Run a complete task: "Take a screenshot and tell me what you see"
- [ ] Run a multi-step task: "Open Settings and find WiFi"
- [ ] Verify full ReAct loop with real vision + real touch

## Tool Priority

For MVP, implement in this order:

1. **ScreenshotTool** — the agent's eyes (essential)
2. **TouchTool** — the agent's primary input (essential)
3. **KeyEventTool** — HOME/BACK navigation (essential)
4. **ShellTool** — fallback for anything (useful)
5. **FileSystemTool** — reading/writing data (useful)
6. **TextInputTool** — typing text (important)
7. **SmsTool** — telephony (differentiating)
8. **CallTool** — telephony (differentiating)
9. **UiDumpTool** — enhanced perception (nice to have)
10. **NotificationTool** — awareness (nice to have)

## Definition of Done

An integration test on a real device where the agent:
1. Takes a screenshot of the home screen
2. Identifies an app icon using vision
3. Taps the icon to open the app
4. Takes another screenshot to verify
5. Reports what it sees

## Related

- [[Phase-3-Hardware]] — Previous phase (HAL)
- [[Phase-5-Integration]] — Next phase (binary)
- [[../implementation/peko-tools-android]] — Tool designs
- [[../implementation/Tool-System]] — Trait architecture

---

#roadmap #phase-4 #tools
