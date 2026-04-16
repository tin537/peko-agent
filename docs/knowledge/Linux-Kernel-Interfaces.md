# Linux Kernel Interfaces

> The kernel subsystems Peko Agent talks to directly.

---

## Overview

Peko Agent communicates with hardware through five kernel subsystems:

```
Peko Agent
  ├── Input subsystem (/dev/input/*) ──► Touch, keys
  ├── Framebuffer (/dev/graphics/fb0) ─► Screen capture (legacy)
  ├── DRM/KMS (/dev/dri/*) ────────────► Screen capture (modern)
  ├── Serial/USB (/dev/ttyACM*) ───────► Modem (SMS, calls)
  └── Network stack (sockets) ─────────► LLM API calls
```

Each interface is wrapped by [[../implementation/peko-hal|peko-hal]] for safe Rust access.

## Input Subsystem (evdev)

See [[Touch-Input-System]] for the full deep dive.

- Device nodes: `/dev/input/event0`, `/dev/input/event1`, ...
- Each device has a name (query via `ioctl(EVIOCGNAME)`)
- Events are `input_event` structs: `{timestamp, type, code, value}`
- Types: `EV_ABS` (absolute positioning), `EV_KEY` (key presses), `EV_SYN` (sync)

**Used by**: `TouchTool`, `KeyEventTool`, `TextInputTool`

## Framebuffer

See [[Screen-Capture]] for the full deep dive.

- Device: `/dev/graphics/fb0` (Android) or `/dev/fb0` (standard Linux)
- Query screen info via `ioctl(FBIOGET_VSCREENINFO)` and `ioctl(FBIOGET_FSCREENINFO)`
- Map to memory via `mmap()` for zero-copy pixel reads
- Pixel format varies by device (RGBA, BGRA, RGB565)

**Used by**: `ScreenshotTool`

## DRM/KMS

See [[Screen-Capture]] for the full deep dive.

- Device: `/dev/dri/card0`
- Modern replacement for framebuffer
- Query connectors, CRTCs, planes via ioctls
- Create dumb buffers for screen capture
- More complex but more capable (multi-display, etc.)

**Used by**: `ScreenshotTool` (when framebuffer not available)

## Serial / TTY

See [[Telephony-AT-Commands]] for the full deep dive.

- Device: `/dev/ttyACM0` (USB modem) or device-specific paths
- Configured via `termios` (baud rate, character size, parity)
- Read/write AT command strings
- Handles unsolicited result codes (incoming calls, SMS notifications)

**Used by**: `SmsTool`, `CallTool`

## Network Stack

Standard POSIX sockets, accessed through Rust's `std::net` and `tokio::net`:

- TCP sockets for HTTPS connections to LLM APIs
- TLS via `rustls` (no OpenSSL dependency)
- Unix domain sockets for the control interface

**Used by**: [[../implementation/peko-transport|peko-transport]], [[../implementation/peko-agent-binary|control socket]]

## Key ioctl Operations

| ioctl | Subsystem | Purpose |
|---|---|---|
| `EVIOCGNAME` | Input | Get device name string |
| `EVIOCGABS` | Input | Get absolute axis info (min/max/resolution) |
| `EVIOCGBIT` | Input | Get supported event types |
| `FBIOGET_VSCREENINFO` | Framebuffer | Get resolution, bpp, color format |
| `FBIOGET_FSCREENINFO` | Framebuffer | Get line length, memory size |
| `DRM_IOCTL_MODE_GETRESOURCES` | DRM | List CRTCs, connectors, encoders |
| `DRM_IOCTL_MODE_CREATE_DUMB` | DRM | Create buffer for screen capture |
| `UI_SET_EVBIT` | uinput | Configure virtual device capabilities |
| `UI_DEV_CREATE` | uinput | Create virtual input device |

All wrapped by the `nix` crate in [[../implementation/peko-hal|peko-hal]].

## Permission Requirements

| Interface | Needed permission | How Peko Agent gets it |
|---|---|---|
| `/dev/input/*` | `input` group or `CAP_SYS_ADMIN` | `group input` in init.rc |
| `/dev/graphics/fb0` | `graphics` group | `group graphics` in init.rc |
| `/dev/dri/*` | `graphics` group | `group graphics` in init.rc |
| `/dev/ttyACM*` | `radio` group or root | `group radio` + `user root` |
| Raw sockets | `CAP_NET_RAW` | `capabilities NET_RAW` in init.rc |

Plus [[SELinux-Policy|SELinux policy]] must explicitly allow each access.

## Related

- [[Touch-Input-System]] — evdev deep dive
- [[Screen-Capture]] — Framebuffer + DRM deep dive
- [[Telephony-AT-Commands]] — Modem communication deep dive
- [[../implementation/peko-hal]] — Rust wrappers for all of these
- [[SELinux-Policy]] — Security policy for device access

---

#knowledge #kernel #linux #devices
