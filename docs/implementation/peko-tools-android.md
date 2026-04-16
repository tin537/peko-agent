# peko-tools-android

> Android-specific tool implementations for device control.

---

## Purpose

This crate provides the agent's **hands** — concrete implementations of the [[Tool-System|`Tool` trait]] that interact with Android hardware through [[peko-hal]]. Each tool is a self-contained struct that the agent can invoke by name.

## Tool Inventory

| Tool | What it does | Kernel interface | Dangerous? |
|---|---|---|---|
| `ScreenshotTool` | Capture current screen | [[../knowledge/Screen-Capture\|framebuffer/DRM]] | No |
| `TouchTool` | Tap, long-press, swipe | [[../knowledge/Touch-Input-System\|evdev]] | No |
| `KeyEventTool` | Press hardware keys | evdev | No |
| `TextInputTool` | Type text into fields | evdev / clipboard | No |
| `SmsTool` | Send SMS messages | [[../knowledge/Telephony-AT-Commands\|AT commands]] | **Yes** |
| `CallTool` | Make/answer/end calls | AT commands | **Yes** |
| `UiDumpTool` | Dump UI hierarchy XML | uiautomator (if available) | No |
| `NotificationTool` | Read notification state | /sys/class/leds or dumpsys | No |
| `FileSystemTool` | Read/write/list files | std::fs | Partially |
| `ShellTool` | Execute shell commands | tokio::process | **Yes** |

Tools marked "dangerous" require explicit confirmation through the control socket before execution. See [[Tool-System]] for the `is_dangerous()` mechanism.

## Tool Details

### ScreenshotTool

Captures the current display as a base64-encoded PNG:

```
1. Read raw pixel data from /dev/graphics/fb0 (mmap)
   or /dev/dri/card0 (DRM ioctl)
2. Convert RGBA buffer to PNG via `image` crate
3. Base64-encode the PNG
4. Return as tool result for inclusion in multimodal LLM message
```

Falls back to `screencap` binary if SurfaceFlinger is running (hybrid mode).

Used by the LLM to "see" the screen — this is how the agent understands what's on the display. Related: [[../research/Computer-Use-Agents]].

### TouchTool

Injects touch events directly into the kernel input subsystem:

```rust
// Tap at (540, 1200)
touch_tool.execute(json!({
    "action": "tap",
    "x": 540,
    "y": 1200
}))
```

Supported actions:
- **tap** — `EV_ABS` down + `SYN_REPORT` + `EV_ABS` up + `SYN_REPORT`
- **long_press** — down + delay (500ms) + up
- **swipe** — down + series of move events + up

Each event writes an `input_event` struct to the touchscreen's `/dev/input/eventN`. See [[../knowledge/Touch-Input-System]] for the evdev protocol details.

### KeyEventTool

Sends hardware key presses:

```rust
key_event_tool.execute(json!({
    "key": "HOME"  // HOME, BACK, POWER, VOLUME_UP, VOLUME_DOWN, ENTER
}))
```

Uses `EV_KEY` event type with Android keycodes.

### TextInputTool

Types text character by character:

- **Frameworkless mode**: Injects individual key events for each ASCII character
- **Hybrid mode**: Writes to clipboard + injects paste key combo
- Handles special characters via keycode mapping

### SmsTool

Sends SMS via direct modem AT commands:

```rust
sms_tool.execute(json!({
    "to": "+1234567890",
    "message": "Hello from Peko"
}))
```

Sequence:
1. Open serial device via [[peko-hal]]'s `SerialModem`
2. `AT+CMGF=1` — set text mode
3. `AT+CMGS="+1234567890"` — start message
4. Send message body + Ctrl-Z (0x1A) to send

See [[../knowledge/Telephony-AT-Commands]] for the full AT command reference.

### CallTool

Manages phone calls:

```rust
call_tool.execute(json!({
    "action": "dial",      // dial, hangup, answer
    "number": "+1234567890"
}))
```

- `ATD+1234567890;` — dial
- `ATH` — hang up
- `ATA` — answer incoming
- Monitors `RING`, `NO CARRIER`, `BUSY` for call state

### UiDumpTool

Captures UI hierarchy as XML:

- If Android framework is running: `uiautomator dump /dev/stdout`
- If frameworkless: returns error, rely on ScreenshotTool + vision LLM
- XML contains element bounds, text, content descriptions — useful for precise coordinate targeting

### FileSystemTool

Standard file operations with **path sandboxing**:

```rust
filesystem_tool.execute(json!({
    "action": "read",  // read, write, list, search, delete
    "path": "/data/peko/notes/todo.txt"
}))
```

Path sandboxing prevents the agent from accidentally modifying system files. Configurable allowed paths in [[../implementation/peko-config|config.toml]].

### ShellTool

Executes arbitrary shell commands:

```rust
shell_tool.execute(json!({
    "command": "ls -la /data/peko/",
    "timeout": 10
}))
```

- Uses `tokio::process::Command`
- Captures stdout + stderr
- Enforces configurable timeout (default 30s)
- Marked as `is_dangerous() = true`

## Registration

All tools are registered at startup in [[peko-agent-binary|main.rs]]:

```rust
let mut registry = ToolRegistry::new();

let input_device = InputDevice::find_touchscreen()?;
let display = Framebuffer::open()?;

registry.register(ScreenshotTool::new(display));
registry.register(TouchTool::new(input_device.clone()));
registry.register(KeyEventTool::new(input_device));
registry.register(TextInputTool::new());
registry.register(SmsTool::new(SerialModem::open()?));
registry.register(CallTool::new(SerialModem::open()?));
// ... etc
```

## Related

- [[Tool-System]] — How the trait and registry work
- [[peko-hal]] — Hardware abstraction used by tools
- [[../knowledge/Touch-Input-System]] — evdev deep dive
- [[../knowledge/Screen-Capture]] — Framebuffer/DRM deep dive
- [[../knowledge/Telephony-AT-Commands]] — AT command reference

---

#implementation #tools #android
