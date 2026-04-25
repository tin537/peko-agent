# Lane A — Frameworkless Boot

A walkthrough of how to boot Peko as PID-1's only child with no
SurfaceFlinger, no Zygote, and no SystemServer — useful for research,
demos, and proving the agent-as-OS thesis on real hardware.

> **Reality check:** Lane A is a research scaffold, not the
> recommended runtime. Lane B (Magisk on stock/LineageOS, framework
> alive) is what users should actually run. See
> [`docs/AGENT_AS_OS_STATUS.md`](../AGENT_AS_OS_STATUS.md) for the
> per-capability boundary.

## What Lane A buys you

- The agent owns DRM master and `/dev/graphics/fb0` — it can write
  pixels to the panel without going through SurfaceFlinger.
- All input devices belong to the agent — no `system_server` ahead of
  it in the chain.
- Wi-Fi via direct `wpa_supplicant` ctrl socket (no `cmd wifi`).
- Battery + the IIO sensors that exist (RRADC, etc.) work.

## What Lane A loses

- Framework-only sensors (Qualcomm SLPI accel/gyro/light/prox via
  `dumpsys`).
- Camera, GPS, NFC — vendor binder HALs only.
- Telephony — RIL-bound. Agent's call/SMS shim apk needs the
  framework to be alive.

## Booting

### 1. Build Peko for ARM64 (Magisk module / LineageOS overlay)

`scripts/build-android.sh` cross-compiles to `aarch64-linux-android`.
The artifact is `target/aarch64-linux-android/release/peko-agent`.

### 2. Drop the agent into the system image

The Magisk module already does this:
`magisk/peko-module/system/bin/peko-agent`. For a stripped AOSP image
(true Lane A), copy the binary plus its dependencies into
`/system/bin/`.

### 3. Add an init service

The reference `init.peko.rc` lives at `rom/init/peko-agent.rc`. Pin
it via Magisk overlay or merge into the system image's `init.rc`:

```
service peko-agent /system/bin/peko-agent --frameworkless --config /data/peko/config.toml
    user root
    group root system input graphics audio inet wifi
    seclabel u:r:peko_agent:s0
    oneshot
    disabled

on property:sys.boot_completed=1
    start peko-agent
```

The `--frameworkless` flag sets `PEKO_FRAMEWORKLESS=1` in the agent's
environment so every fallback chain skips framework probes (no `cmd
wifi`, no `dumpsys`, no `wm size`).

### 4. SELinux policy

Stripped AOSP builds need the policies under `selinux/peko_agent.te`
to reach `/dev/input/event*`, `/dev/graphics/fb0`, `/dev/dri/card0`,
`/data/vendor/wifi/wpa/sockets/wlan0`, and `/dev/snd/pcmC*`.

### 5. Verify

After boot, on a serial console (or via adb if adbd is alive):

```sh
peko-agent --frameworkless --task "Render a 480x80 status overlay reading 'AGENT BOOTED' to /dev/graphics/fb0"
```

The agent calls the `draw` tool with `blit=true`, which goes through
`peko_renderer::blit_to_framebuffer`. If `/dev/graphics/fb0` is the
real scanout buffer (not a stale AOD plane like Lane B's sdm845
fb0), the text appears on the panel.

## Lane A invariants worth keeping

- **No fork-and-exec to framework binaries.** Code paths in
  `peko-hal` and the tools layer are required to surface a typed
  error rather than spawning a missing-when-frameworkless command
  silently.
- **Every Lane B-only feature has a `lane_a` cell in
  `CAPABILITY_MATRIX.md`.** If a capability adds a Lane B fallback,
  the matrix gets a row update in the same commit.
- **`--frameworkless` is the only way to enter Lane A mode.** Auto-
  detection by checking for SurfaceFlinger is unreliable (it can be
  intermittent during boot) — explicit flag is safer.
