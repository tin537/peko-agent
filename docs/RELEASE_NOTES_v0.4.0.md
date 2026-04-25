# Peko Agent v0.4.0 — "Agent-as-OS, honest matrix"

Released 2026-04-25.

## What this release means

v0.4.0 closes the audit gap between the README's "agent-as-OS" claim
and what was actually implemented. Eight phases of focused, kernel-
direct work, every capability tagged in
[`docs/CAPABILITY_MATRIX.md`](CAPABILITY_MATRIX.md), every Lane B/A
distinction documented in [`docs/AGENT_AS_OS_STATUS.md`](AGENT_AS_OS_STATUS.md),
and every claim verified on real hardware (OnePlus 6T sdm845 +
LineageOS 20 + Magisk).

If the matrix disagrees with marketing, the matrix wins.

## Headline numbers

- **6 new HAL modules + 1 new crate** in `peko-hal` and the new
  `peko-renderer`.
- **5 new agent tools** (`screenshot` rewrite, `sensors`, `wifi`,
  `audio`, `draw`) — bringing the count to 18.
- **175 unit tests**, all passing.
- **4 on-device test phases** (display, sensors, wifi, audio) all
  PASS on the reference device, runnable any time via
  `make device-test PHASE=all`.
- **2 device profiles** committed
  (`device-profiles/fajita-lineage-20.toml` and
  `device-profiles/OnePlus6T-lineage_fajita-userdebug.toml`,
  auto-calibrated from EVIOCGABS measurements).
- **Lane A scaffold shipped** — `--frameworkless` flag, init.rc
  service, framebuffer blit code, and an empirical finding that
  fb0 on sdm845 is a phantom AOD plane (DRM is the live scanout).

## Per-phase summary

### Phase 1 — Display capture + input observation

- `peko_hal::display::DisplayCapture` trait with `FbdevCapture` +
  `ScreencapCapture` backends. Sysfs-driven rotation detection,
  CPU-rotated RgbaBuffer.
- `peko_hal::display_info` enumerates DRM devices, connectors,
  modes via `DRM_IOCTL_VERSION` + `MODE_GETRESOURCES` +
  `GETCONNECTOR` (no master required, safe in Lane B).
- `InputDevice::poll_for_event` for input *observation* (not just
  injection).
- Per-device TOML profile loader in `peko-config`.

### Phase 2 — Sensors + battery + light/proximity

- IIO subsystem reader with offset+scale conversion to SI units.
- `/sys/class/power_supply/battery/*` parser with typed
  `ChargeStatus` and `BatteryHealth` enums.
- Light/proximity through input subsystem, with sysfs fallback.
- `dumpsys sensorservice` parser as the Lane B path for Qualcomm
  SLPI sensors that aren't kernel-reachable. Fixture-tested against
  a real OnePlus 6T capture.
- `sensors` tool with action enum
  `{battery, accel, gyro, magnetometer, pressure, ambient_temp, light, proximity}`.

### Phase 3 — Wi-Fi control

- `WifiBackend` trait with `CmdWifiBackend` (Lane B) +
  `WpaSupplicantBackend` (Lane A) — speaks the `wpa_supplicant`
  control protocol over UNIX SOCK_DGRAM at
  `/data/vendor/wifi/wpa/sockets/wlan0`.
- `wifi` tool with status / scan / list_networks / connect /
  disconnect / enable / disable.
- Parsers handle Unicode SSIDs, dedupe networks across security
  variants.

### Phase 4 — ALSA topology + tinymix + media volume

- `alsa_topology()` enumerates `/proc/asound` cards + `/dev/snd`
  PCM nodes.
- `tinymix` wrapper for the kernel mixer (~2900 controls on sdm845).
- `media_volume_get()` shells `cmd audio` for stream-volume.
- PCM record/playback deliberately deferred — call-recorder
  shim already covers the agent's primary need.

### Phase 5 — `peko-renderer` crate

- New crate with `Canvas` over `RgbaBuffer`: rect, line, text via
  embedded 5x7 ASCII font (~83 hand-authored glyphs).
- `Rgba::from_hex` for `#RGB` / `#RRGGBB` / `#RRGGBBAA`.
- Word-wrap, alpha blend, scale support.
- `draw` tool consumes a JSON op list, returns a PNG.

### Phase 6 — Test runner + honest README

- `tests/device-test/run-all.sh` runs phases 1–4 in sequence with
  per-phase logs and a PASS/FAIL summary.
- README replaced "no framework" marketing with the lane model and
  pointers to the matrix as source-of-truth.
- New `docs/AGENT_AS_OS_STATUS.md` documents the Lane B / Lane A
  boundary, the impossibles we're honest about (camera + GPS
  binder HALs), and the verification procedure.

### Phase 7 — Lane A scaffold

- `peko_renderer::blit::blit_to_framebuffer` writes RgbaBuffers
  to `/dev/graphics/fb0` honouring stride + pixel format
  (BGRA8888 / RGBA8888 / RGB565 detected from
  `FBIOGET_VSCREENINFO`).
- `--frameworkless` CLI flag sets `PEKO_FRAMEWORKLESS=1` so
  every framework-fallback chain skips Lane B paths up front.
- `wifi.rs::is_frameworkless()` is the reference pattern; other
  tools follow.
- New `docs/architecture/lane-a-boot.md` walkthrough for the full
  stripped-AOSP path.

### Phase 8 — Magisk-based Lane A simulation

- Standalone `peko-blit-test` ARM64 binary (270 KB) for one-shot
  fb experiments.
- `magisk/peko-module/lane-a-toggle.sh` flips an installed module
  between Lane B and Lane A at runtime.
- **Real-device finding on sdm845**: fb0 is a phantom 640×400
  AOD plane; blit succeeds (1,024,000 bytes written) but never
  reaches the panel. Stopping SurfaceFlinger to test it triggers
  a ~60s framework-restart cycle that self-recovers.
  → Lane A on sdm845 needs DRM, not fbdev. Fully documented
  in `docs/architecture/lane-a-sdm845-finding.md`.
- Test script gated behind `BLIT_OK=1` so re-running is safe by
  default on Qualcomm hardware.

## Hardware verified

| Device | SoC | ROM | Result |
|--------|-----|-----|--------|
| OnePlus 6T (`fajita`) | Qualcomm sdm845 | LineageOS 20 / Android 13 | Phases 1–4 PASS, Phase 8 finding documented |

## Known limits (won't fix)

- **Camera HAL** — vendor binder, no kernel path.
- **GPS / GNSS HAL** — vendor binder.
- **Qualcomm SLPI sensors in Lane A** — DSP firmware is signed.
- **Framebuffer blit on Qualcomm SoCs** — fb0 is a phantom plane;
  Lane A on these SoCs requires the Phase 9+ DRM master path.

## Migration from v0.3.x

- No config file changes required. All new behaviour is opt-in.
- New TOML profile path (`/data/peko/device_profile.toml` or
  `device-profiles/<codename>-*.toml` next to the binary). Loading
  is best-effort; missing profile = auto-detection.
- Tools `screenshot` and `key_event` changed behaviour subtly —
  screenshot now picks the best backend instead of always going
  fbdev-first; key_event prefers shell `input keyevent` to fix
  HOME/BACK/POWER on real devices. No API change.

## Upgrading

For Magisk users:

```bash
make build-magisk     # rebuilds the .zip
adb push dist/peko-module-v0.4.0.zip /sdcard/
# then flash via Magisk app → Modules → Install from storage
```

## Verification

```bash
make test                    # 175 unit tests
make device-test PHASE=all   # 4 device-tests, ~90s on the OnePlus 6T
```

## Roadmap (not in this release)

- **Phase 9** — DRM master + dumb buffer write for Lane A on
  Qualcomm SoCs. Becomes possible the moment SurfaceFlinger isn't
  holding master, which is exactly the Lane A condition.
- **PCM record/playback** — deferred; call-recorder shim covers
  the agent's primary audio use case.
- **Emulator CI** — `aosp_x86_64-userdebug` runner that flashes
  the Magisk module and runs the full test suite on every PR.

## Contributors

- Tanuphat Chainaloewong — design, implementation, on-device validation
- Claude Opus 4.7 (1M context) — co-author across phases 1–10
