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
| `DRM` pixel readback | ❌ | ⏳ | — | Requires DRM master → Lane A only (Phase 8) |
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
| Wi-Fi control | ⏳ | ⏳ | 3 | `wpa_supplicant` ctrl socket |
| Audio capture/playback | ⏳ | ⏳ | 4 | tinyalsa over `/dev/snd/pcmC*` |
| Self-rendered overlay UI | ⏳ | ⏳ | 5 | Text+rect renderer to fbdev/DRM |
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
| Stripped AOSP (Lane A) | 🔧 | Phase 8 |
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
