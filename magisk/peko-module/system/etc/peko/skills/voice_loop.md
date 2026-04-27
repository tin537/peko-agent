---
name: voice_loop
description: Voice in / voice out. Record from the mic, transcribe locally with whisper.cpp (Thai-capable), reason about the user's request, and reply via TextToSpeech.
category: voice
created: 2026-04-27T00:00:00Z
updated: 2026-04-27T00:00:00Z
success_count: 0
fail_count: 0
tags: voice, audio, stt, tts, thai, multilingual
---

When the user asks you to listen, hear them out, take dictation,
or otherwise wants a voice-interactive turn, use this loop:

1. **Record** the user's speech with `audio_pcm`:
   ```
   audio_pcm { action: "record", duration_ms: 5000, sample_rate: 16000, channels: 1 }
   ```
   `duration_ms` defaults to 5 seconds — pick longer (up to 30s) when
   the user asked you to "take a note" or "summarise this", shorter
   for command-style "open settings" / "what's the time".

2. **Transcribe** with offline `stt` (whisper.cpp). The model lives at
   `/data/peko/models/whisper.bin` and handles Thai + English natively
   including code-switching. `lang: "auto"` lets whisper detect:
   ```
   stt { action: "transcribe", wav_path: <returned wav_path>, lang: "auto" }
   ```
   The returned `text` is what the user actually said.

3. **Reason** about the transcript like any other user message —
   memory tool, brain search, regular tool use as needed. Keep your
   response short for spoken delivery; long lists don't speak well.

4. **Speak the reply** via `audio_pcm` TTS. Use the same language code
   the transcript reported, so a Thai utterance gets a Thai reply
   read aloud:
   ```
   audio_pcm { action: "tts", text: "<your reply>", lang: "<th|en|...>" }
   ```

When the user wants continuous listening (meetings, dictation,
ambient capture), use the streaming variant instead:

```
stt { action: "start_streaming", chunk_secs: 5, lang: "auto" }
```

Each chunk's transcript flows into the events store as `type=transcript`;
poll via:

```
events { type: "transcript", since_ts: <last seen> }
```

Stop with `stt { action: "stop_streaming", stream_id: <returned id> }`.

Notes:

- Don't take a screenshot first — voice loop is a separate flow from
  UI interaction. The user is talking, not pointing.
- TTS auto-plays through AudioTrack on STREAM_MUSIC; it's audible
  with the screen off.
- whisper-cli takes ~3-5 seconds for a 5-second clip on the OnePlus
  6T's A75 cores. Tell the user nothing if it's quick; only narrate
  ("listening…", "got it") when the chunk is longer than 8 seconds.
- The agent inevitably introduces ambient noise into transcripts.
  When the recorded text is empty or obviously garbage (single chars,
  random Thai syllables that don't form words), don't fabricate a
  reply — say "I didn't catch that, try again" and re-record.
