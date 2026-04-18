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
