#!/system/bin/sh
# service.sh — runs after boot_completed, when the system is fully up.
# Good place to start user-space daemons that need Android services available.

MODDIR=${0%/*}
LOG=/data/peko/peko.log

# Small delay so framework services (connectivity, power manager) are stable.
sleep 8

# Keep peko binaries out of Doze / App Standby so the life loop keeps ticking
# and the web UI stays reachable when the screen is off.
cmd deviceidle whitelist +bin.peko-agent >/dev/null 2>&1
cmd deviceidle whitelist +bin.peko-llm-daemon >/dev/null 2>&1

# If the Peko overlay app shipped with the module (installed as priv-app by
# Magisk's systemless mount), auto-grant SYSTEM_ALERT_WINDOW so the user
# doesn't have to trek through Settings after a fresh install.
if pm list packages 2>/dev/null | grep -q '^package:com.peko.overlay$'; then
    appops set com.peko.overlay SYSTEM_ALERT_WINDOW allow >/dev/null 2>&1 || true

    # Grant the runtime perms the overlay wants. POST_NOTIFICATIONS is
    # required on Android 13+ for the FGS notification to show; without
    # it the service still runs but has no shade entry. Cheap to grant
    # unconditionally.
    pm grant com.peko.overlay android.permission.POST_NOTIFICATIONS >/dev/null 2>&1 || true

    # Phase 5 audio bridge — RECORD_AUDIO is dangerous. priv-app status
    # alone doesn't grant runtime perms; pm grant is what flips it.
    # AudioBridgeService refuses to start AudioRecord without it.
    pm grant com.peko.overlay android.permission.RECORD_AUDIO >/dev/null 2>&1 || true

    # Kick the overlay now. Belt-and-braces on top of the app's own
    # BootReceiver: the receiver races with this appops grant, so on a
    # cold first boot canDrawOverlays() can return false when BOOT_
    # COMPLETED fires and the receiver silently bails. By the time we
    # reach this line service.sh has already waited `sleep 8`, which
    # is plenty for the appops write to settle. Starting MainActivity
    # re-runs the same permission-check → service-start flow, so it's
    # idempotent if the overlay is already up.
    am start-foreground-service --user 0 -n com.peko.overlay/.OverlayService >/dev/null 2>&1 \
        || am start -n com.peko.overlay/.MainActivity >/dev/null 2>&1 || true
    # Phase 5: also kick the audio bridge so it's ready as soon as the
    # device finishes boot. PekoOverlayApp.onCreate() also starts it
    # but that path requires the app's process to be alive first.
    am start-foreground-service --user 0 -n com.peko.overlay/.AudioBridgeService >/dev/null 2>&1 || true
fi

# If the Peko SMS shim shipped alongside, make it the default SMS app
# and grant its runtime permissions at boot. This is load-bearing:
#
#   - SEND_SMS on Android 13 is hard-restricted. Even for a priv-app
#     with privapp-permissions.xml entries, `pm grant` silently no-ops
#     and SmsManager.sendTextMessage throws SecurityException.
#   - The ONLY way to lift APPLY_RESTRICTION on this ROM is to hold
#     android.app.role.SMS. RoleController grants the exempt flags
#     automatically when the role is assigned.
#   - Once the role is held, peko can send SMS. Incoming SMS flow
#     through SmsDeliverReceiver which writes them to content://sms/
#     inbox, so the user's existing Messaging app keeps showing them.
#
# If the user manually revokes the SMS role via Settings, SMS sending
# will start failing again on the next boot. In that case, re-run this
# service.sh or reboot.
if pm list packages 2>/dev/null | grep -q '^package:com.peko.shim.sms$'; then
    # First, grant the permissions that are "normal" enough that pm
    # grant will actually stick. These lift independently of SMS role.
    for perm in \
        android.permission.READ_PHONE_STATE \
        android.permission.READ_PHONE_NUMBERS \
        android.permission.POST_NOTIFICATIONS; do
        pm grant com.peko.shim.sms "$perm" >/dev/null 2>&1 || true
    done

    # Now the SMS role. This is the magic step — once granted,
    # RoleController stamps SEND_SMS / RECEIVE_SMS / READ_SMS with
    # RESTRICTION_SYSTEM_EXEMPT + GRANTED_BY_DEFAULT, which is exactly
    # the combo the stock Messaging app gets and what we need for
    # SmsManager.sendTextMessage to work.
    #
    # We wait briefly for RoleService to come up — on cold boot it's
    # not always ready when service.sh runs.
    for attempt in 1 2 3 4 5; do
        if cmd role add-role-holder android.app.role.SMS com.peko.shim.sms >/dev/null 2>&1; then
            break
        fi
        sleep 2
    done

    # Also grant the dangerous SMS perms explicitly so they're active
    # immediately (role grant can race with pm's grant propagation
    # on some ROMs).
    for perm in \
        android.permission.SEND_SMS \
        android.permission.RECEIVE_SMS \
        android.permission.READ_SMS; do
        pm grant com.peko.shim.sms "$perm" >/dev/null 2>&1 || true
    done

    # AppOps matches — package-level AND uid-level so runtime checks
    # can't fall back to "ignore" for the shim's UID.
    SHIM_UID=$(dumpsys package com.peko.shim.sms 2>/dev/null | awk -F'=' '/userId=/ {print $2; exit}' | awk '{print $1}')
    for op in SEND_SMS RECEIVE_SMS READ_SMS READ_PHONE_NUMBERS; do
        appops set com.peko.shim.sms "$op" allow >/dev/null 2>&1 || true
        [ -n "$SHIM_UID" ] && appops set --uid "$SHIM_UID" "$op" allow >/dev/null 2>&1 || true
    done

    # Call-recording pipeline: RECORD_AUDIO + READ_CALL_LOG are
    # dangerous runtime perms. Grant at boot so CallRecorderService
    # doesn't SecurityException the first time PHONE_STATE fires.
    # RECORD_AUDIO gates MediaRecorder; READ_CALL_LOG gates the
    # EXTRA_INCOMING_NUMBER extra on Android 13+.
    # CAPTURE_AUDIO_OUTPUT is signature|privileged — `pm grant`
    # no-ops for runtime, but the privapp XML entry is what matters
    # and that took effect at boot. We also flip the RECORD_AUDIO
    # appops to allow both at package and uid level to match the
    # stock Phone app's state.
    for perm in \
        android.permission.RECORD_AUDIO \
        android.permission.READ_CALL_LOG \
        android.permission.PROCESS_OUTGOING_CALLS; do
        pm grant com.peko.shim.sms "$perm" >/dev/null 2>&1 || true
    done
    for op in RECORD_AUDIO READ_CALL_LOG; do
        appops set com.peko.shim.sms "$op" allow >/dev/null 2>&1 || true
        [ -n "$SHIM_UID" ] && appops set --uid "$SHIM_UID" "$op" allow >/dev/null 2>&1 || true
    done

    # The shim writes result files inside its own private storage
    # (/data/data/com.peko.shim.sms/files/sms_out/) — apps are sandboxed
    # out of /data/peko/ regardless of UNIX perms. peko-agent reads
    # those files as root, so no shared dir is needed. The old
    # /data/peko/sms_out/ setup was a dead end; left intentionally
    # unseeded here so nothing hints at a path that won't work.
    # sms_in.log remains as peko-agent's audit trail for messages it
    # has seen arrive; shim can't write to it directly but peko-agent
    # queries the content provider periodically anyway.
fi

# Rotate the log so we don't bloat — keep last 5 runs.
# Shift oldest-to-newest: .4→.5, .3→.4, ... .1→.2, then current→.1
if [ -f "$LOG" ]; then
    [ -f "$LOG.4" ] && mv "$LOG.4" "$LOG.5"
    [ -f "$LOG.3" ] && mv "$LOG.3" "$LOG.4"
    [ -f "$LOG.2" ] && mv "$LOG.2" "$LOG.3"
    [ -f "$LOG.1" ] && mv "$LOG.1" "$LOG.2"
    mv "$LOG" "$LOG.1"
fi

# Tesseract's data path. The bundled traineddata files live under
# /system/etc/tessdata (mounted from the Magisk module's
# system/etc/tessdata/). Setting TESSDATA_PREFIX here means
# peko-agent's `ocr` tool — which exec's tesseract — picks up the
# right language data without needing every invocation to pass
# --tessdata-dir. If the user didn't bundle tesseract, this var is
# harmless: tesseract isn't installed, the OCR tool returns its
# typed "not found" error.
export TESSDATA_PREFIX=/system/etc/tessdata

# Start the LLM daemon first (abstract UDS @peko-llm). Fail-safe — if the
# user hasn't pushed a GGUF model, the daemon will error and peko-agent
# will route everything to cloud via the dual-brain fallback.
MODEL=/data/peko/models/local.gguf
if [ -f "$MODEL" ]; then
    nohup /system/bin/peko-llm-daemon \
        --model "$MODEL" \
        --socket "@peko-llm" \
        > /data/peko/daemon.log 2>&1 &
fi

# Start peko-agent. `--config` picks up /data/peko/config.toml (seeded by
# post-fs-data.sh on first install).
nohup /system/bin/peko-agent \
    --config /data/peko/config.toml \
    --port 8080 \
    > "$LOG" 2>&1 &

echo "[peko] started at $(date)" >> "$LOG"
