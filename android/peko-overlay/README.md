# peko-overlay — floating Peko chat for Android

A tiny native app that draws a **floating draggable cat mascot** on top of every
other app. Tap the cat to open a chat card that streams replies from
`peko-agent` running on localhost (bound by the Magisk module or a plain
`adb forward`). Long-press the cat to shut the overlay down.

```
   ┌─ collapsed ─┐      ┌───────── expanded ─────────┐
   │     😺      │  →   │  😺  peko peko…        ✕   │
   │  64dp bubble│      │  ─────────────────────────  │
   └─────────────┘      │   [peko]  hi!               │
                        │            [user]  hey      │
                        │  ─────────────────────────  │
                        │  [_ Say something…_]   ➤    │
                        └─────────────────────────────┘
```

## Prerequisites

- **Android Studio Ladybug (2024.2)** or newer, OR command-line **AGP 8.7** +
  **JDK 17** + Android SDK 35 with platform-tools.
- **Gradle 8.7+** on `PATH`, OR a materialised wrapper (`gradlew` + `gradle/wrapper/gradle-wrapper.jar`).
  The wrapper is NOT checked in — first time through, bootstrap it with:

  ```bash
  gradle wrapper --gradle-version 8.10
  ```

  After that, `./gradlew …` works as usual and other contributors pick up the
  pinned version automatically.
- Android device/emulator running **Android 8.0+ (API 26)**.

## Build

From this directory (either works):

```bash
./gradlew :app:assembleRelease   # if the wrapper has been generated
gradle   :app:assembleRelease    # plain host install
```

Output APK:

```
app/build/outputs/apk/release/app-release-unsigned.apk
```

The APK is unsigned — Magisk priv-app install doesn't need a signature. For
Play Store or a signed sideload, add a `signingConfigs` block to
`app/build.gradle.kts`.

## Install — sideload

```bash
adb install -r app/build/outputs/apk/release/app-release-unsigned.apk
```

1. Open **Peko** from the launcher.
2. Tap **Open settings** → enable "Allow display over other apps".
3. Come back — the activity finishes, the overlay service starts, a floating
   orange cat appears.
4. Tap the cat → chat card expands. Type → cat replies via SSE.
5. Long-press the cat → overlay service stops.

Clear-text loopback to `127.0.0.1:8080` is whitelisted in
`res/xml/network_security_config.xml`, so the app happily talks to
`peko-agent` without TLS on-device.

## Install — Magisk priv-app (rooted)

Build the module with the overlay bundled in:

```bash
../../magisk/build-module.sh --with-overlay
```

This runs `./gradlew :app:assembleRelease`, then stages the APK into
`system/priv-app/PekoOverlay/PekoOverlay.apk` inside the module zip. On boot,
Magisk mounts it into `/system/priv-app/…`; the module's `service.sh` runs
`appops set com.peko.overlay SYSTEM_ALERT_WINDOW allow` so the user never has
to open Settings. Priv-apps also can't be uninstalled from the launcher —
the only way to remove it is to disable the Magisk module.

## How it's wired

- **`OverlayService`** — foreground service. Creates a `ComposeView`, plumbs
  the ViewTree lifecycle / savedState / viewmodel owners (required when
  hosting Compose outside an Activity), and attaches it to `WindowManager`
  as `TYPE_APPLICATION_OVERLAY`.
- **`ChatController`** — owns the `mutableStateOf` UI state (expanded flag,
  mascot pose, activity label, message list), the SSE coroutine scope, and
  the drag-to-reposition handler that calls
  `WindowManager.updateViewLayout`. The Composables just read it.
- **`PekoClient`** — wraps `POST /api/run` with `okhttp-sse`, decoding each
  SSE frame's `type` field (`status` / `text_delta` / `thinking` /
  `tool_start` / `tool_result` / `done` / `error`) into a sealed
  `PekoEvent`. See `src/web/api.rs::run_task` for the producer side.
- **`PekoOverlay.kt`** — the Compose UI. `AnimatedContent` between a 64dp
  circular mascot bubble and a 320×440 zinc-900 chat card.

## Connecting to peko-agent

- **On-device Magisk install**: `peko-agent` binds `127.0.0.1:8080` as part
  of the module's `service.sh`. The app talks to it directly.
- **Emulator / non-rooted dev loop**: run `peko-agent` on your host and
  `adb reverse tcp:8080 tcp:8080` so the device's loopback forwards to it.
