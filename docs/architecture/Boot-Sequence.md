# Boot Sequence

> From power button to agent loop in four stages.

---

## Overview

Peko Agent starts **before the Android framework**. It's a `class core` init service, meaning it launches alongside fundamental daemons like `ueventd` and `logd`, well before Zygote forks or SystemServer initializes.

```
Power On
  │
  ▼
Bootloader (loads kernel)
  │
  ▼
Linux Kernel (PID 0 → launches /init as PID 1)
  │
  ▼
┌─────────────────────────────────────┐
│ Stage 1: First-stage init           │
│  • Mount /dev, /proc, /sys          │
│  • Mount system, vendor partitions  │
│  • Load SELinux policy              │
└──────────────┬──────────────────────┘
               ▼
┌─────────────────────────────────────┐
│ Stage 2: Second-stage init          │
│  • Parse init.rc scripts            │
│  • class_start core ◄── HERE        │
│  •   └─ peko-agent starts        │
│  • class_start main (Zygote, etc.)  │ ◄── Can be skipped entirely
└──────────────┬──────────────────────┘
               ▼
┌─────────────────────────────────────┐
│ Stage 3: Peko Agent init         │
│  • Parse config.toml                │
│  • Start tokio runtime              │
│  • Open SQLite database             │
│  • Register tools                   │
│  • Probe hardware devices           │
│  • Listen on control socket         │
└──────────────┬──────────────────────┘
               ▼
┌─────────────────────────────────────┐
│ Stage 4: Agent loop running         │
│  • Waiting for commands on socket   │
│  • Or executing startup task        │
└─────────────────────────────────────┘
```

## Stage 1 — Kernel to init

The bootloader loads the Linux kernel, which:

1. Initializes interrupt controllers, memory protections, caches, scheduling
2. Sets up virtual memory and mounts root filesystem
3. Launches `/init` as PID 1

Android's init process runs first-stage initialization:
- Mounts `/dev`, `/proc`, `/sys`
- Early-mounts `system` and `vendor` partitions
- **Compiles and loads [[../knowledge/SELinux-Policy|SELinux policy]]** — this is critical, Peko Agent needs its own SELinux domain

## Stage 2 — init.rc Service Definition

Peko Agent is declared as a service in `/system/etc/init/peko-agent.rc`:

```ini
service peko-agent /system/bin/peko-agent \
    --config /data/peko/config.toml
    class core
    user root
    group root input graphics audio radio inet net_raw
    capabilities NET_RAW NET_ADMIN SYS_PTRACE
    seclabel u:r:peko_agent:s0
    socket peko stream 0660 root root
    writepid /dev/cpuset/foreground/tasks
    oneshot
    disabled
```

### Key directives explained

| Directive | Purpose |
|---|---|
| `class core` | Starts during `on boot`, before `class_start main` |
| `user root` | Runs as root for kernel device access |
| `group input graphics audio radio inet net_raw` | Access to input devices, display, audio, modem, network |
| `capabilities NET_RAW NET_ADMIN SYS_PTRACE` | Raw sockets, network config, process tracing |
| `seclabel u:r:peko_agent:s0` | Custom [[../knowledge/SELinux-Policy\|SELinux domain]] |
| `socket peko stream 0660 root root` | Unix domain socket for external control |
| `oneshot` | Don't restart automatically on exit |
| `disabled` | Don't start until triggered |

### Trigger mechanisms

**Option A — Property trigger** (coexist with framework):
```ini
on property:sys.peko.start=1
    start peko-agent
```

**Option B — Full frameworkless mode** (skip Zygote entirely):
```ini
on late-init
    trigger early-fs
    trigger fs
    trigger post-fs
    trigger post-fs-data
    trigger peko-boot

on peko-boot
    class_start core
    # Deliberately omit: class_start main
    start peko-agent
```

Option B is the "pure" Agent-as-OS mode — the device has no Android framework at all. See [[../01-Vision]] for why this matters.

## Stage 3 — Peko Agent Initialization

When the binary starts, it performs this sequence:

```
1. Parse config ──────► /data/peko/config.toml
2. Init tokio ─────────► Multi-threaded async runtime
3. Init SQLite ────────► /data/peko/state.db (with FTS5)
4. Register tools ─────► ToolRegistry populated
5. Probe hardware ─────► Enumerate /dev/input/event*
                         Check /dev/ttyACM* (modem)
                         Verify framebuffer/DRM
6. Open socket ────────► /dev/socket/peko (Unix domain)
7. Enter loop ─────────► Ready for commands
```

### Hardware probing details

- **Input devices**: Iterate `/dev/input/event*`, call `ioctl(EVIOCGNAME)` on each to identify the touchscreen vs keyboard vs other devices. See [[../knowledge/Touch-Input-System]].
- **Display**: Check `/dev/graphics/fb0` existence, fall back to `/dev/dri/card0` for DRM. See [[../knowledge/Screen-Capture]].
- **Modem**: Scan `/sys/class/tty/` for serial devices, test with `AT` command. See [[../knowledge/Telephony-AT-Commands]].

## Stage 4 — Agent Loop

The agent enters one of two modes:

1. **Socket mode** (default) — Listens on the Unix socket for JSON-RPC commands
2. **Startup task mode** — Immediately executes a pre-configured task from config

See [[../implementation/peko-agent-binary]] for the control socket protocol.

## Dependency Chain

What Peko Agent **requires** from the boot process:

```
Linux kernel (always present)
├── /dev mounted (first-stage init)
├── /proc, /sys mounted (first-stage init)
├── system partition mounted (first-stage init)
├── data partition mounted (post-fs-data)
├── SELinux policy loaded (first-stage init)
├── Network stack initialized (kernel)
└── Input subsystem ready (kernel)
```

What it does **NOT** require:

- Zygote / ART / Dalvik
- SystemServer
- SurfaceFlinger
- Any Java/Kotlin runtime
- Any Android SDK component
- Binder service manager

## Related

- [[../knowledge/Android-Internals]] — Deep dive into what init.rc does
- [[../knowledge/SELinux-Policy]] — The custom policy needed
- [[Architecture-Overview]] — Where boot fits in the big picture
- [[../implementation/peko-agent-binary]] — What runs after boot

---

#architecture #boot #android
