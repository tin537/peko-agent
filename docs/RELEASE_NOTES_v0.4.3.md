# Peko Agent v0.4.3 — "Vendor surfaces, via the framework we already trust"

Released 2026-04-26.

## What this release does

Closes the four "vendor binder" rows (camera, GPS, telephony, audio
routing) — and adds an event bus for streaming sources — by extending
the same priv-app shim pattern that already worked for SMS, calls, and
PCM/TTS. No SELinux policy edits, no AIDL re-implementations, no
binder-from-Rust. The agent talks to PekoOverlay over file-RPC; the
priv-app uses the Android SDK to drive the underlying HAL.

## Architecture

Two planes:

```
peko-agent (root)
  │
  │  ── control plane: file-RPC under /data/data/com.peko.overlay/files/<topic>/
  │
  ▼
PekoOverlay priv-app
  ├── RpcDispatcher (shared FileObserver)
  ├── LocationBridgeService    → LocationManager     → gnss HAL
  ├── TelephonyBridgeService   → TelephonyManager    → radio HAL
  ├── CameraBridgeService      → Camera2 + ImageReader → camera HAL
  └── AudioBridgeService       → AudioRecord/Track/TTS/AudioManager → audio HAL
                                       │
  ── data plane: events.db SQLite ◀────┘
                                       │
peko-agent reads via rusqlite ◀────────┘
```

Streaming sources (camera frames, GPS samples, ambient-audio features)
do NOT round-trip through file-RPC — that would be terrible at 5+ Hz.
They write into a single shared **events.db** SQLite file inside the
priv-app, and peko-agent reads via rusqlite directly (root crosses UID).

## New tools

| Tool | Actions |
|---|---|
| `gps` | `fix`, `start_stream`, `stop_stream` |
| `telephony` | `info`, `signal`, `cells` |
| `camera` | `capture`, `start_stream`, `stop_stream` |
| `events` | poll filtered events from the shared DB |
| `audio_pcm` (extended) | + `route_get`, `route_set`, `start_ambient`, `stop_ambient` |

## Verified on-device (OnePlus 6T)

| Surface | Result |
|---|---|
| `telephony info` | `SIM_STATE_READY`, carrier `TRUE-H`, country `th`, LTE, phone number returned |
| `camera capture` (back, 720p) | 1280×720 JPEG, 178KB, ~3s end-to-end |
| `gps fix` (outdoor) | lat 13.638598, lon 100.849054, ±9.6m via gps, fresh fix |
| Ambient stream | 1Hz windowed RMS/peak/ZC features into events.db, three windows in 3s |

## Why no on-device ML yet (deliberate)

Streaming pipes are now in place; the actual classifiers (object
detection on frames, sound classification on ambient windows) will land
in Phase 24 either as bundled TFLite (MobileNet SSD ~25MB + YAMNet ~5MB)
or as cloud pipeline calls. The agent can ALREADY ship frames to a
vision LLM and audio features to anything — the only thing missing is
in-shim labelling, which is a model-load swap-in, not infrastructure
work.

## Capability matrix delta

Five rows changed:

| Row | Before | After |
|---|---|---|
| Camera | ❌ Lane B / ❌ Lane A | ✅ Lane B / 🟡 Lane A |
| GPS | ❌ / ❌ | ✅ / ❌ (gnss is binder, no kernel path) |
| Telephony info | (not listed) | ✅ / ❌ |
| Audio routing | (not listed) | ✅ / 🟡 |
| Ambient sound stream | (not listed) | ✅ / 🟡 |

Lane A still ❌ for GPS / telephony because gnss + radio HALs are pure
vendor binder — there's no kernel ABI to fall back on. This isn't
fixable with the framework path (the whole point of Lane A is no
framework). Camera + audio Lane A drop from ❌ to 🟡 because the
existing shim works in any AOSP build that has audioserver +
cameraserver up; the pure-Lane-A pure-tinyalsa / pure-DRM path stays a
Phase 7+ research item.

## Permissions added

```
android.permission.CAMERA
android.permission.FOREGROUND_SERVICE_CAMERA
android.permission.ACCESS_FINE_LOCATION
android.permission.ACCESS_COARSE_LOCATION
android.permission.ACCESS_BACKGROUND_LOCATION
android.permission.FOREGROUND_SERVICE_LOCATION
android.permission.READ_PHONE_STATE
android.permission.READ_PHONE_NUMBERS
android.permission.ACCESS_NETWORK_STATE
```

`service.sh` `pm grant`s all dangerous ones at boot.

## Code shape (LOC)

| Layer | Lines |
|---|---|
| New Kotlin (RpcDispatcher + EventStore + 3 bridge services + audio extensions) | ~1100 |
| New Rust (bridge_client + 4 tools + audio_pcm extensions) | ~600 |
| Manifest + service.sh + main.rs wiring | ~80 |

## Migration

Drop in the new PekoOverlay.apk + the new peko-agent binary. Reboot
once for service.sh to grant the new dangerous perms and start every
bridge. Append `"gps"`, `"telephony"`, `"camera"`, `"events"` to
`[telegram].allowed_tools`.

## What's next (Phase 24 candidates)

- **TFLite-in-shim**: bundle MobileNet SSD + YAMNet, write detections /
  sound classes back into events.db with the rest. Closes the "real
  on-device object detection / ambient classification" gap entirely.
- **Voice loop**: `audio_pcm record → cloud Whisper → agent reasoning
  → audio_pcm tts` as a single bound skill.
- **Camera vision skill**: `camera capture → vision LLM` as the agent's
  default reach when "look at X" is asked.
