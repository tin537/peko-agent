# Architecture Overview

> Peko Agent is a single Rust binary that replaces the Android application stack with a direct kernel-to-LLM pipeline.

---

## System Layers

```
┌─────────────────────────────────────────────┐
│              Cloud LLM APIs                 │
│    (Anthropic / OpenRouter / Local)          │
└──────────────────┬──────────────────────────┘
                   │ HTTPS + SSE
┌──────────────────▼──────────────────────────┐
│         peko-agent (single binary)       │
│  ┌─────────────────────────────────────┐    │
│  │  peko-core (Agent Runtime)        │    │
│  │  ┌──────────┐ ┌──────────────────┐  │    │
│  │  │ ReAct    │ │ Tool Registry    │  │    │
│  │  │ Loop     │ │ (trait dispatch) │  │    │
│  │  └──────────┘ └──────────────────┘  │    │
│  │  ┌──────────┐ ┌──────────────────┐  │    │
│  │  │ Context  │ │ Session Store    │  │    │
│  │  │ Compress │ │ (SQLite+FTS5)   │  │    │
│  │  └──────────┘ └──────────────────┘  │    │
│  └─────────────────────────────────────┘    │
│  ┌──────────────┐  ┌───────────────────┐    │
│  │ peko-      │  │ peko-tools-     │    │
│  │ transport    │  │ android           │    │
│  │ (HTTP+SSE)   │  │ (Screenshot,      │    │
│  │              │  │  Touch, SMS, ...)  │    │
│  └──────────────┘  └───────┬───────────┘    │
│                            │                │
│  ┌─────────────────────────▼───────────┐    │
│  │  peko-hal (Hardware Abstraction)  │    │
│  │  InputDevice│Framebuffer│Modem│UInput│    │
│  └─────────────────────────────────────┘    │
└──────────────────┬──────────────────────────┘
                   │ ioctl / read / write
┌──────────────────▼──────────────────────────┐
│            Linux Kernel                      │
│  evdev │ framebuffer │ DRM │ serial │ net   │
└──────────────────┬──────────────────────────┘
                   │
┌──────────────────▼──────────────────────────┐
│          Hardware                             │
│  Touchscreen │ Display │ Modem │ WiFi/LTE   │
└─────────────────────────────────────────────┘
```

## What's Eliminated

The entire middle layer of traditional Android is gone:

| Component | Memory | Purpose | Peko Agent replacement |
|---|---|---|---|
| Zygote | ~50 MB | Fork app processes | Not needed — single process |
| ART/Dalvik | ~100 MB | Execute Java/Kotlin | Rust binary, no VM |
| SystemServer | ~200 MB | System services | Direct kernel access |
| SurfaceFlinger | ~80 MB | Display compositor | Direct framebuffer/DRM read |
| Binder IPC | overhead | Inter-process routing | In-process function calls |
| PackageManager | ~50 MB | App management | No apps to manage |
| ActivityManager | ~80 MB | UI lifecycle | No activities |

**Total saved: ~800 MB of RAM**

## Data Flow: A Single Agent Action

When the agent decides to tap a button on screen:

```
1. LLM returns tool_call: {"name": "touch", "args": {"x": 540, "y": 1200}}
   ↓ (SSE stream parsed by peko-transport)
2. peko-core dispatches to ToolRegistry
   ↓ (trait object dispatch)
3. peko-tools-android::TouchTool::execute()
   ↓ (calls into HAL)
4. peko-hal::InputDevice::inject_tap(540, 1200)
   ↓ (writes input_event structs)
5. write() to /dev/input/event2
   ↓ (kernel processes event)
6. Touchscreen controller receives coordinates
```

Total latency: **< 1ms** from tool dispatch to hardware event.

Compare with traditional Android path:
```
App → InputManager.injectInputEvent() → Binder → SystemServer
→ InputDispatcher → WindowManager → SurfaceFlinger → Hardware
```
That path: **5-15ms** with multiple context switches.

## Crate Responsibilities

See [[Crate-Map]] for the full dependency graph. Summary:

| Crate | Role | Platform-specific? |
|---|---|---|
| [[../implementation/peko-core\|peko-core]] | Agent brain — loop, tools, context, sessions | No |
| [[../implementation/peko-transport\|peko-transport]] | HTTP client, SSE, LLM providers | No |
| [[../implementation/peko-tools-android\|peko-tools-android]] | Android tool implementations | Yes |
| [[../implementation/peko-hal\|peko-hal]] | Kernel device wrappers | Yes |
| [[../implementation/peko-config\|peko-config]] | Configuration parsing | No |
| [[../implementation/peko-agent-binary\|peko-agent]] | Binary entry point | Yes |

The platform-agnostic crates (core, transport, config) can be tested on desktop Linux/macOS. Only the HAL and tools require Android hardware or emulation.

## Key Design Principles

1. **Trait-based abstraction** — All major boundaries use traits (`Tool`, `LlmProvider`, `TokenCounter`), enabling mock implementations for testing
2. **Async-first** — Everything runs on [[../knowledge/Why-Rust|tokio]], from HTTP streaming to tool execution to socket handling
3. **Zero framework dependency** — The binary links only against `libc` and kernel interfaces
4. **Fail-safe iteration** — [[../implementation/ReAct-Loop|Budget system]] prevents runaway agent loops
5. **Provider agnostic** — Hot-swap between cloud and local LLMs without code changes

## Related

- [[Boot-Sequence]] — How the binary actually starts
- [[Crate-Map]] — Dependency graph visualization
- [[../01-Vision]] — Why this architecture exists

---

#architecture #overview
