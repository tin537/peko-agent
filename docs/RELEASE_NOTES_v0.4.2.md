# Peko Agent v0.4.2 — "Lane B complete: speech in, speech out"

Released 2026-04-26.

## What this release means

v0.4.0 declared "agent-as-OS, honest matrix" but PCM record / playback
sat at ⏳ — the last green-field gap in Lane B. v0.4.2 closes it.

After this release, every non-structural row in
[`docs/CAPABILITY_MATRIX.md`](CAPABILITY_MATRIX.md) is ✅ for Lane B.
The remaining ❌ rows (camera, GPS, deep audio-HAL routing) are
binder/vendor-blob only on every modern Android — there is no kernel
path for us to reach for, so they're explicitly out of scope.

## Headline change: Phase 5 — Audio surface

`audio_pcm` tool, three actions:

- **`record { duration_ms, sample_rate?:16000, channels?:1, source? }`**
  — captures mic audio, returns `/data/peko/audio/<id>.wav` (16-bit
  little-endian PCM). Sources: `mic` / `voice_recognition` /
  `voice_communication`.
- **`play_wav { wav_path }`** — pipes the WAV through `AudioTrack` on
  `STREAM_MUSIC`. Blocks until playback finishes.
- **`tts { text, lang?:"en", rate?, pitch? }`** — synthesises speech
  via `TextToSpeech.synthesizeToFile`, then auto-plays.

### Why a priv-app bridge

Stock Android's `audioserver` owns `/dev/snd/pcmC*D*c`. peko-agent
running as root still gets EBUSY when opening those nodes, because
the kernel locking is per-fd-by-process. The only userspace path in
is `AudioRecord` / `AudioTrack`, which talk to `audioserver` via
binder — and binder is reachable only from a real Android app context.

`PekoOverlay` is already a priv-app and already runs continuously, so
it's the natural home. This release adds an `AudioBridgeService` to it
that watches a file-based RPC channel under the priv-app's private
files dir.

### Wire protocol

```
peko-agent (root) writes:
  /data/data/com.peko.overlay/files/audio/in/<id>.json    request
  /data/data/com.peko.overlay/files/audio/in/<id>.wav     input PCM (play_wav only)
  /data/data/com.peko.overlay/files/audio/in/<id>.start   sentinel — service picks up

bridge writes:
  /data/data/com.peko.overlay/files/audio/out/<id>.wav    output PCM (record/tts)
  /data/data/com.peko.overlay/files/audio/out/<id>.json   metadata
  /data/data/com.peko.overlay/files/audio/out/<id>.done   sentinel — agent picks up
```

Sentinel rename = atomic handoff. Both sides only read after the peer's
sentinel appears, so neither sees a half-written file. Mirrors the
existing `CallRecorderService` pattern, which is the proven precedent
for cross-domain file passing on this device.

### Verified on-device

| Test | Result |
|---|---|
| 1.5s mic record → 48,044 byte 16-bit PCM WAV | ✅ |
| TTS "Hello from Peko. Phase five working." → 122,638 byte WAV in <2s | ✅ |
| Bridge timeout when service absent | ✅ (graceful error) |

## Carryover from v0.4.1

This branch also carries Phases 21 + 22 (bg persistence + mid-run
resume) and the lockscreen prompt nudge. See
[`RELEASE_NOTES_v0.4.1.md`](RELEASE_NOTES_v0.4.1.md) for that story.

## Permissions added

```
android.permission.RECORD_AUDIO                       (dangerous; granted at boot via service.sh)
android.permission.FOREGROUND_SERVICE_MICROPHONE      (FGS type)
android.permission.FOREGROUND_SERVICE_MEDIA_PLAYBACK  (FGS type)
android.permission.MODIFY_AUDIO_SETTINGS              (volume / route control)
```

## Migration

Drop in the new PekoOverlay.apk (already in
`magisk/peko-module/system/priv-app/PekoOverlay/`) and the new
peko-agent binary. service.sh grants RECORD_AUDIO automatically on
next boot. Append `"audio_pcm"` to `[telegram].allowed_tools` if you
want the agent to use it over Telegram.

## Capability snapshot

Every Lane B ✅:

- Display capture, rotation, evdev injection (touch + key + text)
- UI inspect via uiautomator
- All 19 agent tools (was 18; +1 for `audio_pcm`)
- Sensors / battery / Wi-Fi / audio mixer / **PCM + TTS** / draw
- All cognition layers (Reflector, Life Loop, Motivation, Curiosity,
  Goal generator, Memory gardener)
- All persistence stores (memory, brain, bg with checkpoint resume,
  sessions, skills, calls, user model)
- All UX surfaces (web, overlay APK, Telegram)
- Magisk module + LineageOS overlay deploy

The ❌ rows are camera, GPS, deep audio HAL routing — all binder/
vendor-blob with no kernel path. Out of scope.

## What's next

- **Phase 23 — bg → Telegram completion notifications** (close the
  polling gap; agents that fire bg jobs over the bot get a ping when
  the result is ready).
- **Auto-prune `bg.db`** via the gardener (terminal jobs >7d).
- **Voice loop**: chain `audio_pcm record` → cloud Whisper → agent →
  `audio_pcm tts`. The pieces all exist; just needs an orchestration
  skill.
- **Lane A audio**: today the bridge needs `audioserver` (Lane B
  framework), so Lane A is 🟡. Either transplant the priv-app into a
  minimal AOSP image, or implement a tinyalsa direct path. Phase 7+
  scope.
