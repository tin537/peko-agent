# Peko Agent Capability Matrix

Source-of-truth for "what actually works" vs. "what the README claims."
Updated as part of every phase merge. If a row in this table doesn't
match reality, the bug is in the **table** or the **code**, not in the
README — the table is what we measure against.

**Lane B** = hybrid mode (Magisk on stock or LineageOS, framework running).
**Lane A** = frameworkless mode (PID-1 supervisor, no SurfaceFlinger).

Status legend:
- ✅ implemented + tested (unit + on-device)
- 🟡 implemented but limited (works in some configs, see notes)
- 🔧 in progress (current phase)
- ⏳ planned (future phase)
- ❌ not planned

---

## Display capture

| Capability | Lane B | Lane A | Test                        | Notes |
|------------|:------:|:------:|-----------------------------|-------|
| `screencap` (SurfaceFlinger) | ✅ | ❌ | unit + phase1.sh | Disappears in Lane A |
| `fbdev` mmap (`/dev/graphics/fb0`) | 🟡 | ✅ | unit + phase1.sh | Stale on sdm845 in Lane B; primary in Lane A |
| `DRM` enumeration (no master) | ✅ | ✅ | unit + phase1.sh | Diagnostics only |
| `DRM` pixel readback | ❌ | ✅ | 9 | `peko_renderer::drm` ships SET_MASTER + CREATE_DUMB + MAP_DUMB + ADDFB2 + SETCRTC. EBUSY in Lane B (SF holds master) — code path verified by unit tests + struct-size sanity checks |
| Framebuffer blit (write canvas → fb0) | 🟡 | 🟡 | 7 | Code shipped + tested. **sdm845 finding:** fb0 is a phantom AOD plane; blit succeeds but pixels never reach the panel — Lane A on sdm845 must use DRM. See `docs/architecture/lane-a-sdm845-finding.md` |
| Framebuffer blit on devices where fb0 IS scanout | ✅ | ✅ | 7 | Pre-Treble vendor kernels + emulators; verified by unit tests, awaits hardware visual confirmation |
| DRM master + dumb buffer write (Lane A on sdm845) | ❌ | ✅ | 9 | Code shipped; `--enumerate` runs while SF is up, `--paint` requires Lane A or SF stopped. Visual confirmation pending Lane A boot |
| Display rotation detection | ✅ | ✅ | unit + phase1.sh | sysfs `rotate` + device profile override |
| `auto_capture()` backend pick | ✅ | ✅ | unit | screencap → fbdev fallback |

## Input

| Capability | Lane B | Lane A | Test | Notes |
|------------|:------:|:------:|------|-------|
| evdev tap injection | ✅ | ✅ | unit + on-device | ABS_MT_SLOT, BTN_TOOL_FINGER, EVIOCGABS scaling |
| evdev swipe injection | ✅ | ✅ | unit + on-device | |
| key event (HOME/BACK/POWER) | ✅ | 🟡 | on-device | Shell-first in Lane B; Lane A relies on raw evdev to a key-capable node |
| Input event observation (`poll_for_event`) | ✅ | ✅ | unit | New in Phase 1 |
| uiautomator dump | ✅ | ❌ | on-device | Framework-only (Phase 5 ships fbdev-based fallback) |

## Hardware HAL

| Capability | Lane B | Lane A | Phase | Notes |
|------------|:------:|:------:|:-----:|-------|
| Modem AT (`/dev/tty*`) | 🟡 | 🟡 | shipped | Blocked by RILD on stock devices |
| Sensors (accel/gyro/mag/pressure/temp) | ✅ | 🟡 | 2 | IIO sysfs first, dumpsys sensorservice fallback. Lane A only sees IIO; Qualcomm SLPI sensors are dark without binder client |
| Light + proximity | ✅ | 🟡 | 2 | input subsystem → `/sys/class/sensors/*` → dumpsys. Same Lane A caveat |
| Battery (capacity/status/health/V/I/T) | ✅ | ✅ | 2 | `/sys/class/power_supply/battery/*` — fully kernel-direct |
| Wi-Fi control (status/scan/saved/connect/disconnect/enable/disable) | ✅ | ✅ | 3 | `cmd wifi` (Lane B) → wpa_supplicant ctrl socket (Lane A). Both backends, `WifiBackend` trait |
| Audio topology + mixer + media volume | ✅ | 🟡 | 4 | `/proc/asound`, `tinymix`, `cmd audio get-volume`. Lane A only sees ALSA + tinymix; media volume needs framework |
| PCM record / playback | ⏳ | ⏳ | 5 | Overlay APK shim (AudioRecord/AudioTrack) — cleaner than re-implementing tinyalsa |
| Self-rendered overlay UI (`draw` tool) | ✅ | ✅ | 5 | `peko-renderer` crate: rect/line/text via embedded 5x7 font, returns PNG. Lane A blits to fbdev |
| Camera | ❌ | ❌ | — | Camera HAL is binder/vendor-blob only |
| GPS | ❌ | ❌ | — | gnss HAL binder-only |

## LLM runtime

| Capability | Status | Test |
|------------|:------:|------|
| Cloud providers (Anthropic/OpenAI/etc.) | ✅ | unit + integration |
| Local llama.cpp daemon (CPU) | ✅ | CI build + manual |
| Local daemon — Vulkan (Adreno/Mali) | ✅ | CI build |
| Dual-brain router | ✅ | unit |

## Agent loop

| Capability | Status | Notes |
|------------|:------:|-------|
| ReAct loop | ✅ | runtime.rs |
| Reflector (Phase A) | ✅ | post-task evaluation |
| Life Loop (Phase B) | ✅ | 60–300s heartbeat |
| Motivation drives (Phase D) | ✅ | curiosity/competence/social/coherence |
| Curiosity (Phase E) | ✅ | unseen-tool proposals |
| Goal generator (Phase F) | ✅ | pattern-driven |
| Memory gardener (Phase G) | ✅ | daily cron |
| Token / proposal budgets | ✅ | 24h cap, propose-only default |

## Persistence

| Capability | Status |
|------------|:------:|
| Memory (SQLite + FTS5) | ✅ |
| Skills | ✅ |
| Calls (audio + transcripts) | ✅ |
| Sessions | ✅ |
| Device profiles (`device_profile.toml`) | ✅ (Phase 1) |
| User model | ✅ |

## UX surfaces

| Capability | Status |
|------------|:------:|
| Web UI (axum :8080) | ✅ |
| Floating overlay APK | ✅ |
| Telegram bot | ✅ |
| Self-rendered system overlay | ⏳ Phase 5 |

## Deploy paths

| Path | Status | Test |
|------|:------:|------|
| Magisk module | ✅ | CI build + manual flash |
| LineageOS overlay (OnePlus 6T) | ✅ | manual |
| Stripped AOSP (Lane A) | 🟡 | Phase 7 ships boot scaffold (`init.peko.rc` + `--frameworkless` flag + fb blit). Real-device flash + boot still maintainer-managed |
| Rooted ADB push | ✅ | manual |

## CI

| Check | Status |
|-------|:------:|
| Workspace build | ✅ |
| Unit tests | ✅ (~100) |
| C++ daemon build (CPU + Vulkan) | ✅ |
| APK builds (overlay, sms-shim) | ✅ |
| Magisk module assembly | ✅ |
| On-device integration tests | 🟡 local-only via `make device-test` |
| Emulator integration tests | ⏳ Phase 6 |

---

## How to update this table

1. Implement the capability behind a `Tool` impl or HAL module.
2. Add at least one unit test (parser/protocol logic).
3. Add a row or update an existing one in this file.
4. Add an on-device check to `tests/device-test/phaseN.sh`.
5. Run `make device-test PHASE=N` against a real device before merging.
