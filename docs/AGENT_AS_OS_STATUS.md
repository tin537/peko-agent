# Agent-as-OS Status

A direct, honest answer to "is Peko really an OS?" — kept current as
each phase merges. If this disagrees with `README.md`, this file wins.

## TL;DR

Peko runs as a **kernel-direct AI agent** on Android. It talks to the
hardware through Linux device files (`/dev/input/event*`,
`/dev/graphics/fb0`, `/dev/dri/card0`, `/sys/class/power_supply/`,
`/dev/snd/pcmC*`, `wpa_supplicant` ctrl socket, AT-modem `/dev/ttyACM*`)
without going through Java framework APIs.

It is **not** a full operating system on its own — Lane B (the
production path) keeps Android's framework + SurfaceFlinger running
because some hardware still has no kernel-level interface (camera,
GPS, the SLPI sensor DSP on Qualcomm). Lane A (frameworkless) replaces
everything reachable via kernel interfaces, and the capability matrix
documents exactly what works in each lane.

## The two lanes

### Lane B — hybrid (this is what users actually run)

- Android boots normally. SurfaceFlinger + RIL + audioserver + sensorservice are alive.
- Peko runs as a system service injected via Magisk module or LineageOS overlay.
- Peko owns the *UX surface* (web :8080, Telegram bot, floating overlay, Phase 5 self-rendered overlays), the *agent loop*, and *autonomy*.
- For each capability, Peko prefers the kernel-direct path; falls back to the framework only when the kernel layer is genuinely empty (e.g., Qualcomm SLPI sensors via `dumpsys sensorservice`).
- **All 16 tools work, including SMS, voice calls, and Wi-Fi.**

### Lane A — frameworkless (research / dev mode)

- No SurfaceFlinger, no Zygote, no ART, no SystemServer.
- Peko is PID-1's only child (init service). Holds DRM master.
- Renders its own UI via `peko-renderer` → `/dev/graphics/fb0` (Phase 7 wires the blit step).
- Reads sensors only where IIO exposes them (battery, RRADC). SLPI motion/light/prox sensors are dark.
- Camera + GPS dark (HAL is binder/vendor-blob).
- Wi-Fi via direct `wpa_supplicant` ctrl socket.

## Capability snapshot

See [CAPABILITY_MATRIX.md](CAPABILITY_MATRIX.md) for the source of truth.
This is a one-line summary per capability:

- ✅ Display capture (fbdev + screencap, DRM enumeration; DRM read in Phase 7)
- ✅ Display rotation detection
- ✅ Touch + key injection (evdev with EVIOCGABS scaling, ABS_MT_SLOT, BTN_TOOL_FINGER, unique tracking IDs, shell `input keyevent` for HOME/BACK/POWER)
- ✅ Input event observation (`poll_for_event`)
- ✅ Sensors (Lane B via IIO + dumpsys parser; Lane A via IIO only)
- ✅ Battery (kernel-direct, both lanes)
- ✅ Wi-Fi control (Lane B `cmd wifi`; Lane A `wpa_supplicant` ctrl socket)
- ✅ Audio topology + tinymix + media volume
- ✅ Self-rendered overlay UI (peko-renderer, embedded 5x7 font)
- ⏳ PCM record/play (Phase 7 via overlay APK shim)
- ⏳ DRM master + frameworkless boot (Phase 7)
- ❌ Camera (HAL is binder/vendor-blob; never planned)
- ❌ GPS (gnss HAL is binder; never planned)

## Verification

The numbers below are real, measured on a OnePlus 6T (codename
`fajita`) running LineageOS 20 with Magisk:

| Phase | Test                       | Result       |
|-------|----------------------------|--------------|
| 1     | display + input observation| PASS         |
| 2     | sensors + battery          | PASS         |
| 3     | wifi backends              | PASS         |
| 4     | audio topology + mixer     | PASS         |

Re-run anytime: `make device-test PHASE=all`.

Unit tests: 170 across 7 crates, all green.

## What "100%" means

The phrase "100% agent as OS" doesn't mean Peko *replaces* Android on
every device. It means:

1. **Every kernel-reachable surface** has a typed Rust path.
2. **Every framework-only surface** has a documented fallback (or is
   marked impossible — camera, GPS).
3. **Lane A is bootable** for one specific reference device (sdm845
   OnePlus 6T) so the architecture is proven, not just theorised.

Phases 1-6 satisfy (1) and (2). Phase 7 (Lane A scaffold) is what
turns (3) into reality.

## Known limits we won't fix

- **Qualcomm SLPI sensors are dark in Lane A.** The DSP firmware is
  signed and the binder bridge is the only userspace path. Lane B
  uses `dumpsys` to surface them.
- **Camera and GPS are binder-only on every modern Android.** Vendor
  HALs are closed binaries. We don't pretend to ship these.
- **Audio HAL routing** (deep DSP setup for hands-free, USB audio,
  HDMI) is binder. We expose tinymix for kernel-level mixer control,
  not the higher-level routing.
