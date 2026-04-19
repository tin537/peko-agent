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
fi

# If the Peko SMS shim shipped alongside, grant its runtime permissions at
# boot. These are NOT granted automatically even for priv-apps — they're
# "hard-restricted" dangerous permissions (SEND_SMS and friends). The
# privapp-permissions-peko.xml whitelists them so they're RESTRICTION_-
# SYSTEM_EXEMPT and `pm grant` can actually stick; without that XML,
# this grant silently no-ops. Make the result dir world-writable too
# so the shim's app-UID can write JSON status files that peko-agent polls.
if pm list packages 2>/dev/null | grep -q '^package:com.peko.shim.sms$'; then
    for perm in \
        android.permission.SEND_SMS \
        android.permission.RECEIVE_SMS \
        android.permission.READ_SMS \
        android.permission.READ_PHONE_STATE \
        android.permission.READ_PHONE_NUMBERS; do
        pm grant com.peko.shim.sms "$perm" >/dev/null 2>&1 || true
    done
    # AppOps matches — package-level AND uid-level so runtime checks
    # can't fall back to "ignore" for the shim's UID.
    SHIM_UID=$(dumpsys package com.peko.shim.sms 2>/dev/null | awk -F'=' '/userId=/ {print $2; exit}' | awk '{print $1}')
    for op in SEND_SMS RECEIVE_SMS READ_SMS READ_PHONE_NUMBERS; do
        appops set com.peko.shim.sms "$op" allow >/dev/null 2>&1 || true
        [ -n "$SHIM_UID" ] && appops set --uid "$SHIM_UID" "$op" allow >/dev/null 2>&1 || true
    done
    # /data/peko/sms_out is peko-agent's result directory; shim needs
    # to write to it as an app UID. 0777 is fine on a rooted device —
    # the directory only contains ephemeral JSON status files, and
    # peko-agent cleans its own stale ones.
    mkdir -p /data/peko/sms_out
    chmod 0777 /data/peko/sms_out
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
