# The Agent-as-OS Vision

> What if the LLM agent wasn't an app *inside* an OS, but *was* the OS-level orchestrator itself?

---

## The Problem

Every existing mobile LLM agent — [[research/Mobile-Agents|AppAgent]], [[research/Mobile-Agents|Mobile-Agent]], [[research/Mobile-Agents|AutoDroid]] — runs as an **application within the Android framework**. This means:

1. **~800 MB memory overhead** from Zygote, ART, SystemServer, SurfaceFlinger before your agent even loads
2. **Permission sandbox** — you must ask the OS for permission to touch its own screen
3. **Accessibility service dependency** — agents hack through accessibility APIs meant for screen readers
4. **Multi-millisecond latency** for every hardware interaction, routed through Binder IPC
5. **Competing for resources** with an OS designed for multi-app GUI paradigms the agent doesn't need

## The Insight

Android's Linux kernel already provides **every hardware interface an agent needs**:

| Need | Kernel Interface | Framework Path (eliminated) |
|---|---|---|
| Touch injection | `/dev/input/event*` via [[knowledge/Touch-Input-System\|evdev]] | InputManagerService → Binder → App |
| Screen capture | `/dev/graphics/fb0` or [[knowledge/Screen-Capture\|DRM]] | SurfaceFlinger → Binder → screencap |
| SMS | [[knowledge/Telephony-AT-Commands\|AT commands]] via serial | RIL → TelephonyManager → Intent |
| Phone calls | AT commands via serial | Same RIL stack |
| Networking | TCP/IP sockets | Same (but no permission check) |

The Android framework exists to support **multi-app GUI paradigms**. An autonomous agent doesn't need them.

## The Architecture

Strip away everything between the agent and the kernel:

```
Traditional Android Agent:
  LLM Agent App → Android Framework → Linux Kernel → Hardware

Peko Agent:
  LLM Agent (Rust binary) → Linux Kernel → Hardware
```

The agent boots as a [[architecture/Boot-Sequence|PID-1 child]] via `init.rc`, before Zygote ever starts. It gets:

- **Root-level access** to all kernel interfaces
- **Sub-millisecond hardware interaction** (no Binder IPC)
- **< 50 MB memory footprint** (vs ~800 MB)
- **No permission sandbox** — it IS the system
- **Custom [[knowledge/SELinux-Policy|SELinux domain]]** for security boundaries

## What This Enables

1. **Fully autonomous device control** — the agent can do anything a human can do, faster
2. **Minimal hardware requirements** — runs on cheap Android devices without needing RAM for framework
3. **Real-time responsiveness** — direct hardware access means the agent reacts instantly
4. **Dedicated agent appliances** — phones/tablets that exist solely as agent endpoints
5. **Edge AI with cloud reasoning** — local hardware control + cloud LLM intelligence

## Why Now

Three converging trends make this feasible:

1. **Rust in AOSP** — Google officially supports [[knowledge/Rust-On-Android|Rust for Android system code]] since 2021
2. **Vision-capable LLMs** — models can understand screenshots, eliminating need for UI hierarchy XML
3. **Streaming tool use** — [[implementation/SSE-Streaming|SSE-based tool calling]] enables real-time agent loops

## Related Concepts

- [[research/Computer-Use-Agents]] — Anthropic's Claude Computer Use took a similar direction for desktop
- [[research/Related-Work-Overview]] — OS-Copilot explored OS-level agents but still within the OS
- [[knowledge/Android-Internals]] — Understanding what we're bypassing

---

#vision #architecture #core-concept
