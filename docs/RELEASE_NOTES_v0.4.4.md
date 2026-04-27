# Peko Agent v0.4.4 — "Voice in, voice out, audit-hardened"

Released 2026-04-27.

## What this release means

v0.4.4 closes the voice-interaction loop and lands a comprehensive
audit pass. After this release the agent can talk and listen entirely
offline (Thai + English + code-switching), bg jobs phone home when
they finish, and 14 audit-grade risks across CRITICAL / HIGH / MEDIUM
severity are patched.

It also rolls up the four releases that were cut on the working branch
since v0.4.0 (v0.4.1 / v0.4.2 / v0.4.3 / v0.4.4) into a single landing
on `main`.

## Headlines

### 🎙 Phase 25 — Offline STT (whisper.cpp)
- New `peko-stt` Rust crate + `peko-stt-bin` cmake project (mirrors
  `peko-llm-daemon` recipe; FetchContent v1.7.4)
- Cross-compiled `whisper-cli` (1.1 MB aarch64 ELF) shipped via the
  Magisk module; auto-discovers across `/system/bin`, the Magisk
  module dir, and `/data/local/tmp`
- `stt` agent tool: `transcribe`, `start_streaming`, `stop_streaming`,
  `streaming_status`, `info`
- `/stt` Telegram command for status (model presence, size, paths,
  threads)
- `voice_loop` skill seeded into `/data/peko/skills/` (records →
  transcribes → reasons → speaks back)
- WAV resample shim handles non-16kHz inputs (TTS output, etc.)
- Streaming carries `initial_prompt` from prior chunk's tail for
  soft-overlap context — fixes word-boundary hallucinations
- Ambient-audio events gain heuristic `label` field (silence /
  speech / tone / noise / ambient)

**Live verified on OnePlus 6T**: 3s 16kHz mono mic clip → Thai text
in ~3-5s on 4 A75 cores. End-to-end voice loop test scripted.

### 📱 Phase 23 — Vendor-binder shim
- Camera (Camera2 + ImageReader) — one-shot capture + 1-FPS frame
  stream
- GPS (LocationManager) — fix + continuous stream
- Telephony (TelephonyManager) — info / signal / cells, read-only
- Audio routing (AudioManager) — mode + speaker + SCO control
- Shared `RpcDispatcher` + shared `EventStore` SQLite for streaming
  data plane
- 5 new agent tools (`gps`, `telephony`, `camera`, `events`,
  `audio_pcm` extensions); `/data/data/com.peko.overlay/databases/events.db`

### 🔁 Phase 21+22 — bg persistence + mid-run resume
- SQLite-backed `BgStore` at `<data_dir>/bg.db`; survives restarts
- Per-iteration mid-run checkpoints; jobs resume across crashes
  (1-hour stale window; older auto-orphaned with clear reason)
- Daily token + wall-clock + iteration + concurrent caps
- Self-introspectable `bg stats` action with 10 per-day metrics
- New: bg → Telegram completion notifications on every terminal
  status (closes the polling gap)
- New: bg.db auto-prune via the gardener (terminal jobs >7d)

### 🔊 Phase 5 — PCM / TTS bridge
- AudioBridgeService priv-app with file-RPC: record / play_wav / tts
- Auto-route via AudioManager
- Continuous ambient-feature stream into events.db (RMS / peak /
  zero-crossing rate + heuristic label)

### 🛡 Audit pass — 14 fixes across CRITICAL / HIGH / MEDIUM
- **CRITICAL**: checkpoint blob slimming (image base64 stripped);
  Telegram callback rate-limit; UTF-8 char chunking in send_text;
  plan body validation against declared `tools_used`; RpcDispatcher
  always emits `.done` sentinel
- **HIGH**: Camera2 disconnect/close race; stream_id path-traversal
  validation; tool-input JSON parse errors surfaced; escalation
  budget cap; `try_lock_retry` with backoff + visible degradation;
  MCP failure counter + 30s call timeout + 5s init timeout;
  checkpoint resume snapshots system_prompt
- **MEDIUM**: TTS @Volatile + CountDownLatch; ambient audio error
  capture; FTS5 query escape; screenshot MIME mismatch + panic
  removal; cron parse loud + per-task last_error
- **Bug fix**: CleanupGuard ENOENT — fixed in `a3a5715`

### 🧹 Polish
- bridge_client / audio_pcm dedupe: ~95 LOC of duplicate file-RPC
  removed; single source of truth
- /help text gains Voice (Phase 25) block
- `scripts/voice-loop-test.sh` — one-command live verification

## Capability matrix delta

| Row | v0.4.0 | v0.4.4 |
|---|---|---|
| Camera | ❌ | ✅ Lane B / 🟡 Lane A |
| GPS | ❌ | ✅ Lane B / ❌ Lane A (gnss is binder-only) |
| Telephony info | (not listed) | ✅ Lane B / ❌ Lane A |
| Audio routing | (not listed) | ✅ Lane B / 🟡 Lane A |
| Ambient sound stream + label | (not listed) | ✅ Lane B / 🟡 Lane A |
| PCM record + playback + TTS | ⏳ | ✅ Lane B / 🟡 Lane A |
| Speech-to-text (offline, multilingual) | (not listed) | ✅ Lane B / ✅ Lane A |
| Background jobs (persistent + resumable + budgeted) | basic | ✅ full |

Every Lane B row is now ✅ except the structural ❌ ones (camera HAL
binder, GPS gnss binder, deep audio HAL routing) — out of scope by
design.

## On-device verified (OnePlus 6T fajita / LineageOS 20)

| Test | Result |
|---|---|
| `bg stats`, `bg fire`, checkpoint resume | ✅ |
| Synthetic Phase 22 orphan auto-fail | ✅ |
| Camera capture (back, 720p) → 178 KB JPEG | ✅ |
| GPS fix (outdoor) → ±9.6m via gps | ✅ |
| Telephony info → SIM_STATE_READY, TRUE-H, LTE | ✅ |
| TTS roundtrip → 122 KB WAV in <2s | ✅ |
| Mic record (3s 16kHz mono) → 48 KB WAV | ✅ |
| whisper-cli on the recording → Thai transcription | ✅ |
| voice-loop-test.sh (record → stt → tts) | ✅ |
| 11/11 background-store unit tests + 3/3 stt + 3/3 stt_tool + 3/3 gardener | ✅ |

## Permissions added across all phases

```
RECORD_AUDIO, FOREGROUND_SERVICE_MICROPHONE, FOREGROUND_SERVICE_MEDIA_PLAYBACK
CAMERA, FOREGROUND_SERVICE_CAMERA
ACCESS_FINE_LOCATION, ACCESS_COARSE_LOCATION, ACCESS_BACKGROUND_LOCATION
FOREGROUND_SERVICE_LOCATION
READ_PHONE_STATE, READ_PHONE_NUMBERS, ACCESS_NETWORK_STATE
MODIFY_AUDIO_SETTINGS
```

`service.sh` `pm grant`s every dangerous one at boot.

## New artifacts shipped via Magisk module

| Path | What |
|---|---|
| `/system/bin/peko-agent` | Rust agent (~7.9 MB) |
| `/system/bin/peko-llm-daemon` | llama.cpp UDS daemon (CPU + Vulkan) |
| `/system/bin/whisper-cli` | whisper.cpp speech-to-text (~1.1 MB) |
| `/system/bin/tesseract` | offline OCR |
| `/system/priv-app/PekoOverlay/PekoOverlay.apk` | overlay + audio + camera + GPS + telephony bridges |
| `/system/priv-app/PekoSmsShim/PekoSmsShim.apk` | SMS + call-recorder bridge |
| `/system/etc/peko/skills/voice_loop.md` | seed skill (auto-installed) |

User pushes `~150 MB ggml-base.bin` (multilingual whisper model) to
`/data/peko/models/whisper.bin` once via
`scripts/download-whisper-model.sh`.

## Migration from v0.4.0

Reflash the Magisk module. `post-fs-data.sh` seeds defaults; no manual
config required. Push the whisper model once. If you want STT or any
new tool over Telegram, append to `[telegram].allowed_tools` in
`/data/peko/config.toml`:

```toml
allowed_tools = [
    # ... existing ...
    "audio_pcm", "gps", "telephony", "camera", "events", "stt",
]
```

## Code shape (workspace, since v0.4.0)

| Layer | LOC delta |
|---|---|
| Rust (peko-stt, peko-stt-bin, peko-tools-android, peko-core, src/main, src/telegram/bot) | +~3,400 |
| Kotlin (PekoOverlay: Audio/Camera/Location/Telephony bridges, RpcDispatcher, EventStore) | +~1,400 |
| CMake / build scripts | +~120 |
| Docs + release notes | +~500 |
| Tests added | 25+ unit tests |

## What's next (post v0.4.4)

- **Phase 24 — TFLite YAMNet** for real ambient-sound classification
  (replaces the Phase 25 heuristic labels with 521-class output)
- **STT daemon** — long-running `peko-stt-daemon` to keep the model
  warm and enable true sub-2s streaming partials
- **Phase 23c — vision LLM auto-call on camera-stream events**
- **Branch push to remote** (still blocked on SSH key on this Mac)
