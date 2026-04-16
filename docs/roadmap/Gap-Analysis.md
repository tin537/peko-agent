# Gap Analysis: Peko Agent vs Original Peko Agent

> Comparing our Android Agent-as-OS implementation against the reference
> [Peko Agent by ftmstars](https://peko-agent.ftmstars.com/docs/)

---

## Current State Summary

Peko Agent is a **complete on-device agent runtime** — 10 tools, hardware HAL,
LLM streaming, web UI, ROM integration, 4MB binary at 5MB RSS. It works.

But the original Peko Agent's core innovation isn't the tools — it's the
**closed learning loop**. The agent that gets smarter the longer it runs.
We don't have that yet.

---

## Gap Matrix

### Critical (defines the learning loop)

| Feature | Original | Peko Agent | Impact |
|---|---|---|---|
| **Memory System** | FTS5 recall, LLM summarization, periodic nudges, cross-session | SQLite sessions only, no recall | Agent forgets everything between tasks |
| **Skills System** | Auto-creates reusable procedures from experience, self-improves | None | Agent resolves the same task from scratch every time |
| **User Modeling** | Honcho dialectic model, deepens over sessions | None | Agent treats every user the same, never adapts |

### Important (multiplies usefulness)

| Feature | Original | Peko Agent | Impact |
|---|---|---|---|
| **Messaging Gateway** | 15+ platforms (Telegram, Discord, Slack, WhatsApp, Signal, Matrix, SMS, Email...) | Web UI only | Can only interact when browser is open |
| **Scheduled Tasks** | Built-in cron + delivery to any platform | None | No autonomous recurring work |
| **Subagent Delegation** | Spawn isolated parallel agents | Single agent | Can't parallelize complex tasks |
| **MCP Integration** | Connect to any MCP server | None | Can't extend tools dynamically |
| **Voice Mode** | Real-time voice in CLI + messaging | None | No hands-free interaction |

### Nice-to-Have (polish)

| Feature | Original | Peko Agent | Impact |
|---|---|---|---|
| **SOUL.md** | Customizable personality file | Hardcoded prompt | Users can't personalize easily |
| **Context Files** | Per-conversation context injection | None | No project-specific context |
| **Programmatic Tool Calling** | `execute_code` for multi-step | None | Extra LLM round-trips |
| **Skill Hub** | Community sharing via agentskills.io | None | Can't share/import skills |
| **Batch Processing** | Multi-task queue, trajectory export | None | No bulk operations |
| **Container Isolation** | Docker/sandbox for dangerous tools | `is_dangerous()` flag only | Less safe execution |

---

## What We Have That Original Doesn't

Peko Agent has unique capabilities the Python-based Peko Agent lacks:

| Feature | Peko Agent | Original |
|---|---|---|
| **Agent-as-OS** | Boots from init.rc, IS the system | App inside an OS |
| **Direct kernel access** | evdev, framebuffer, AT commands | Relies on framework APIs |
| **< 5MB RSS** | Minimal memory footprint | Python + framework overhead |
| **Single binary** | 4MB stripped ELF, no dependencies | Python environment + pip packages |
| **ROM integration** | SELinux policy, init.rc, frameworkless boot | N/A |
| **Hardware HAL** | Direct ioctl wrappers for all devices | N/A |
| **Android device control** | Touch injection, screen capture, telephony | Desktop-focused |
| **Package management** | Install/uninstall/launch APKs natively | N/A |

---

## Architecture Comparison

```
Original Peko Agent (Python):
┌─────────────────────────────────────┐
│ Peko Agent                        │
│ ├── Agent Loop (ReAct)              │
│ ├── Memory System ◄── MISSING       │
│ │   ├── FTS5 cross-session search   │
│ │   ├── LLM summarization           │
│ │   └── Periodic nudges             │
│ ├── Skills System ◄── MISSING       │
│ │   ├── Skill creation from exp     │
│ │   ├── Skill self-improvement      │
│ │   └── Skills Hub integration      │
│ ├── User Model ◄── MISSING          │
│ │   └── Honcho dialectic modeling   │
│ ├── Messaging Gateway ◄── MISSING   │
│ │   └── 15+ platform adapters       │
│ ├── Cron Scheduler ◄── MISSING      │
│ ├── Subagent System ◄── MISSING     │
│ ├── MCP Bridge ◄── MISSING          │
│ ├── Voice System ◄── MISSING        │
│ ├── 47 Tools                        │
│ ├── Context Files                   │
│ └── SOUL.md Personality             │
│                                     │
│ Runs on: Linux/macOS/WSL2           │
│ Interface: CLI + messaging          │
└─────────────────────────────────────┘

Peko Agent (Rust):
┌─────────────────────────────────────┐
│ peko-agent binary (4MB)          │
│ ├── Agent Loop (ReAct) ✓            │
│ ├── peko-core ✓                   │
│ │   ├── Session persistence ✓       │
│ │   ├── Context compression ✓       │
│ │   ├── Iteration budget ✓          │
│ │   └── Memory monitor ✓            │
│ ├── peko-transport ✓              │
│ │   ├── Anthropic provider ✓        │
│ │   ├── OpenAI-compat provider ✓    │
│ │   └── Provider failover chain ✓   │
│ ├── peko-hal ✓ (UNIQUE)           │
│ │   ├── evdev input injection ✓     │
│ │   ├── Framebuffer capture ✓       │
│ │   ├── UInput virtual device ✓     │
│ │   ├── Serial modem AT cmds ✓      │
│ │   ├── Accessibility/UI dump ✓     │
│ │   └── Package manager ✓           │
│ ├── 10 Android Tools ✓ (UNIQUE)     │
│ ├── Web UI ✓                        │
│ │   ├── Chat + streaming ✓          │
│ │   ├── Device profile ✓            │
│ │   ├── Apps manager ✓              │
│ │   ├── SMS/notification stream ✓   │
│ │   └── Config persistence ✓        │
│ └── ROM integration ✓ (UNIQUE)      │
│     ├── init.rc service ✓           │
│     ├── SELinux policy ✓            │
│     └── Frameworkless boot ✓        │
│                                     │
│ Runs on: Android (PID 1 child)      │
│ Interface: Web UI (port 8080)       │
│ RSS: ~5MB | Binary: ~4MB            │
└─────────────────────────────────────┘
```

---

#gap-analysis #planning #roadmap
