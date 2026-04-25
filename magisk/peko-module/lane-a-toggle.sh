#!/system/bin/sh
# lane-a-toggle.sh — flip the agent between Lane B (default, framework UI
# active) and Lane A (framework UI dormant, agent owns display).
#
# Usage on device (after Magisk module is installed):
#   /data/adb/modules/peko-module/lane-a-toggle.sh on
#   /data/adb/modules/peko-module/lane-a-toggle.sh off
#   /data/adb/modules/peko-module/lane-a-toggle.sh status
#
# Lane A here = "framework boots but is dormant for the UX surface":
#   - SurfaceFlinger is stopped; agent paints to /dev/graphics/fb0.
#   - SystemUI + the default launcher are stopped.
#   - RIL / audioserver / sensorservice stay up so dumpsys/cmd-wifi
#     fallbacks still work.
#
# To recover from a stuck Lane A: `lane-a-toggle.sh off` over adb,
# or hold power 15s for a hard reboot.

set -u
MODE_FILE=/data/peko/lane.mode

case "${1:-status}" in
    on)
        echo "[lane-a] stopping SurfaceFlinger + SystemUI + Launcher"
        stop surfaceflinger 2>/dev/null
        stop systemui 2>/dev/null
        am force-stop com.android.launcher3 2>/dev/null
        am force-stop com.google.android.apps.nexuslauncher 2>/dev/null
        # Re-launch peko-agent with --frameworkless. Doesn't kill the
        # existing instance — just adds a Lane A worker process.
        if pgrep -f 'peko-agent.*--frameworkless' >/dev/null; then
            echo "[lane-a] frameworkless agent already running"
        else
            nohup /system/bin/peko-agent \
                --frameworkless \
                --config /data/peko/config.toml \
                --port 8081 \
                > /data/peko/peko-lane-a.log 2>&1 &
            echo "[lane-a] frameworkless agent started on :8081"
        fi
        echo "lane-a" > "$MODE_FILE"
        ;;
    off)
        echo "[lane-a] killing frameworkless agent"
        pkill -f 'peko-agent.*--frameworkless' 2>/dev/null
        echo "[lane-a] starting SurfaceFlinger + SystemUI"
        start surfaceflinger 2>/dev/null
        start systemui 2>/dev/null
        echo "lane-b" > "$MODE_FILE"
        ;;
    status)
        if [ -r "$MODE_FILE" ]; then
            cat "$MODE_FILE"
        else
            echo "lane-b"
        fi
        ;;
    *)
        echo "usage: lane-a-toggle.sh [on|off|status]" >&2
        exit 1
        ;;
esac
