# peko-sms-shim

A headless priv-app that exists for exactly one reason: **give peko-agent a path to send SMS on modern phones where the modem's AT channel is owned by RILD**.

## Why this exists

`peko-tools-android::SmsTool` (the older tool) opens `/dev/smd11` and sends `AT+CMGS=...`. This works on a handful of developer boards and old Qualcomm dev kits. It does *not* work on:

- Any OnePlus device (OxygenOS / LineageOS)
- Any Pixel ≥ Pixel 3
- Any Samsung Galaxy ≥ Android 10
- Any Xiaomi ≥ MIUI 12

On all of these, RILD (Radio Interface Layer Daemon) holds an exclusive lock on `/dev/smd*`. As root you can `open()` the node, but every `read()` blocks forever and every `write()` gets dropped on the floor.

The standard Android way to send SMS from an app is `SmsManager.sendTextMessage()`. That calls `ISms.sendTextForSubscriber()` over binder into `system_server`, which forwards through `TelephonyManager` to RILD over the RIL socket — the same code path the stock Messages app uses. No AT. No modem device access. Just works.

**The catch**: `SmsManager.sendTextMessage()` requires the `SEND_SMS` permission, which for a non-default-SMS app on Android 10+ is considered privileged. A regular user-installed APK can't get it at runtime.

A **system priv-app**, however, can. Install a tiny APK at `/system/priv-app/PekoSmsShim/PekoSmsShim.apk` together with a `privapp-permissions-peko.xml` at `/system/etc/permissions/`, and PackageManager grants the permission automatically at boot. Magisk's systemless mount lets us stage both files from a module without modifying the real `/system` partition.

That's what this shim is.

## What it does

Single Kotlin app. No Activities, no notifications, no UI. Just two BroadcastReceivers:

| Receiver | Job |
|---|---|
| `SmsCommandReceiver` (exported, `permission="SEND_SMS"`) | Accepts `am broadcast -a com.peko.shim.sms.SEND --es id <uuid> --es to <phone> --es body <text>`, calls `SmsManager.sendTextMessage(to, null, body, sentPI, deliveredPI)`, writes status to `/data/peko/sms_out/<id>.json`. |
| `SmsResultReceiver` (not exported) | Fires when the radio reports the message was sent/delivered via the PendingIntents from the first receiver. Updates the same JSON file. |

peko-agent polls that file and returns the final state to the LLM.

## Result file protocol

```json
{
  "id": "<uuid>",
  "status": "queued" | "sent" | "delivered" | "error",
  "ts": 1776604123456,
  "to": "+66812345678",
  "body_len": 42,
  "error": "...present only on status=error..."
}
```

`queued` is written synchronously inside `onReceive` the moment `SmsManager` accepts the call. `sent` / `delivered` / `error` replace it later via atomic `tmp → rename`. peko-agent treats `sent` as a success terminal — some carriers never deliver the `delivered` ACK.

## Build

Needs JDK 17 + Android SDK 35 + Gradle 8.7+.

```bash
# one-time: brew install gradle && gradle wrapper --gradle-version 8.10
./gradlew :app:assembleRelease
```

Output: `app/build/outputs/apk/release/app-release-unsigned.apk` (~80 KB — it really is this small).

The APK is **intentionally unsigned**. Magisk priv-app installation doesn't require a platform signature; placement under `/system/priv-app/` is the trust boundary. If you ever want to sideload this normally (not via Magisk), you'd need to sign it.

## Install via the Magisk module

```bash
./magisk/build-module.sh --with-sms-shim
```

That flag runs `gradle :app:assembleRelease` here, then copies the APK into `magisk/peko-module/system/priv-app/PekoSmsShim/PekoSmsShim.apk`, and zips the whole module. The matching `privapp-permissions-peko.xml` is already tracked inside the module tree.

Push the zip, install via Magisk app, **reboot** (priv-app changes only take effect after a restart — PackageManagerService only re-scans `/system/priv-app/` at boot).

After reboot, verify:

```bash
adb shell 'su -c "pm list packages | grep peko.shim"'         # → package:com.peko.shim.sms
adb shell 'su -c "dumpsys package com.peko.shim.sms | grep SEND_SMS"'
#   → granted=true  (if it says granted=false, privapp-permissions XML
#                    wasn't loaded — check logcat for PackageManager errors)
```

Then in peko's Chat tab: `"send an SMS to +... saying hello"`. The agent will call the `sms` tool, which `am broadcast`s to this shim.

## Security model

1. **Caller can't be a random app.** The receiver's `android:permission="android.permission.SEND_SMS"` attribute on the intent filter means only callers holding that permission can fire the broadcast. Shell (uid 2000) has it by default; regular apps don't.
2. **Caller must be root or shell.** Inside `onReceive` we additionally check `Binder.getCallingUid()` against `0` (root) and `2000` (shell). Belt + braces.
3. **peko-agent rate-limits before broadcasting.** See `crates/peko-tools-android/src/sms_framework.rs` — config `[tools.sms_config]` caps the send rate per hour / per day. Default 5/hour, 20/day. Audit log at `/data/peko/sms_sent.log`.
4. **Messages aren't stored in the inbox DB** because this shim isn't the default SMS app. That's a feature, not a bug — peko's sends don't show up in the user's Messages app. Reads still work (we query the provider DB directly in `src/web/device.rs`).
5. **Never reads incoming SMS.** Declared `RECEIVE_SMS` for future use, but no receiver listens for incoming. If/when peko learns to respond to SMS, we'll add one behind a separate config flag.

## Known limits

- **No MMS.** `SmsManager.sendMultimediaMessage()` requires more ceremony (configuring the carrier MMSC, building the PDU). Not implemented.
- **No delivery-receipt guarantees.** `status=delivered` depends on the carrier honouring the SMS-STATUS-REPORT. Many don't. `sent` is the realistic terminal state.
- **Single SIM by default.** Multi-SIM devices can route by passing `sub_id` in the broadcast; peko-tools-android doesn't expose that parameter yet.
- **No send-as-default-app semantics.** Android considers us a "background SMS app" for policy purposes. The OS may rate-limit us globally after ~30 messages in a short window regardless of our own config — that's the `RESULT_ERROR_LIMIT_EXCEEDED` case in `SmsResultReceiver`.
