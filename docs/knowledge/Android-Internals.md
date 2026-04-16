# Android Internals

> Understanding what Peko Agent bypasses — and what it keeps.

---

## Android Boot Stack

```
Bootloader
  │
  ▼
Linux Kernel
  │
  ▼
/init (PID 1) ──────────────────── Peko Agent lives HERE
  │
  ├── ueventd (device nodes)
  ├── logd (logging)
  ├── servicemanager (Binder)
  ├── peko-agent ◄───────────── class core, before main
  │
  ├── Zygote ────────────────────── Everything below is BYPASSED
  │   ├── SystemServer
  │   │   ├── ActivityManager
  │   │   ├── WindowManager
  │   │   ├── PackageManager
  │   │   ├── InputManager
  │   │   ├── TelephonyManager
  │   │   └── ... (100+ services)
  │   │
  │   └── SurfaceFlinger
  │
  └── (App processes forked from Zygote)
```

## What init Does

Android's `/init` is not like Linux's systemd or sysvinit. It's a custom init system that:

1. **First-stage init**: Mounts essential filesystems, loads [[SELinux-Policy|SELinux]], transitions to second stage
2. **Second-stage init**: Parses `.rc` files, starts services, handles property triggers
3. **Main loop**: Monitors child processes, handles property changes, executes triggers

### .rc File Language

```ini
# Service definition
service <name> <path> [<arguments>]
    class <class>        # core, main, late_start
    user <uid>
    group <gid> [<gid>]*
    capabilities <cap>+
    seclabel <label>
    socket <name> <type> <perm> <uid> <gid>
    oneshot              # Don't restart on exit
    disabled             # Don't start automatically

# Triggers
on <trigger>
    <command>
    <command>

# Property triggers
on property:<name>=<value>
    <command>
```

### Service Classes (Boot Order)

| Class | When | Examples |
|---|---|---|
| `core` | `on boot` trigger | ueventd, logd, **peko-agent** |
| `main` | `class_start main` | Zygote, SurfaceFlinger, netd |
| `late_start` | After framework ready | Update services, optional daemons |

**Key insight**: `core` services start before `main`. By placing Peko Agent in `core`, it runs before Zygote ever starts.

## What Zygote Does (and Why We Don't Need It)

Zygote is the parent process for all Android apps. It:

1. Starts the ART virtual machine
2. Preloads common Java classes and resources
3. Forks to create each app process (copy-on-write, fast)
4. Starts SystemServer as its first child

**Why Peko Agent doesn't need it**: We're a native Rust binary, not a Java app. No ART, no classes to preload, no apps to fork.

## What SystemServer Does (and How We Replace It)

SystemServer runs 100+ system services. The ones relevant to agent tasks:

| SystemServer Service | What it does | Peko Agent replacement |
|---|---|---|
| InputManagerService | Routes touch/key events | Direct [[Touch-Input-System\|evdev]] writes |
| WindowManagerService | Manages app windows | Not needed — no windows |
| ActivityManagerService | App lifecycle | Not needed — no apps |
| TelephonyManager | Phone calls, SMS | Direct [[Telephony-AT-Commands\|AT commands]] via serial |
| ConnectivityService | Network management | Kernel TCP/IP stack directly |
| DisplayManagerService | Screen management | Direct [[Screen-Capture\|framebuffer/DRM]] |
| AudioService | Audio routing | Direct ALSA/TinyALSA (if needed) |

## Binder IPC

Android's primary IPC mechanism. Services register with `servicemanager`, clients look them up and make RPC calls through the Binder kernel driver (`/dev/binder`).

**Peko Agent typically doesn't need Binder** because it accesses hardware directly. However, for hybrid mode (framework partially running), it can optionally use raw Binder ioctls to communicate with HAL services.

## Android Properties

Key-value store for system state. Peko Agent can read them via `getprop`:

```
sys.boot_completed=1        # Framework boot done
ro.build.version.sdk=34     # API level
ro.hardware=qcom            # Hardware platform
persist.sys.timezone=...    # Timezone
```

Useful for [[../implementation/peko-config|configuration]] and hardware detection.

## Hybrid Mode vs Full Frameworkless

| Mode | Framework running? | Use case |
|---|---|---|
| **Hybrid** | Yes (Zygote + SystemServer active) | Agent coexists with normal Android. Can use `uiautomator`, `screencap` binary, etc. |
| **Frameworkless** | No (skip `class_start main`) | Maximum performance. Agent is the only user-space process. Must use direct kernel interfaces for everything. |

The [[../architecture/Boot-Sequence|boot sequence]] supports both modes.

## Related

- [[../architecture/Boot-Sequence]] — How Peko Agent fits into the boot process
- [[SELinux-Policy]] — Security policy for the custom domain
- [[../01-Vision]] — Why we bypass the framework
- [[../knowledge/Rust-On-Android]] — Rust's role in AOSP

---

#knowledge #android #internals #boot
