#!/system/bin/sh
# Device-side probe for Phase 1.
#
# Runs entirely on the Android device under root. Prints a deterministic
# key=value report on stdout that the host-side phase1.sh parses. Every
# probe is bounded by `timeout` so a stuck driver can't hang the host.
#
# CRITICAL: do NOT use `getevent -l <node>` without `-p`. Without `-p`,
# getevent streams events forever — `head -1` does not reliably SIGPIPE
# through `adb shell 'su -c "..."'` and the host hangs. Use sysfs for
# names and `getevent -lp` for capabilities (it prints and exits).

set -u

TIMEOUT="${TIMEOUT:-3}"

# Some BusyBox builds don't ship `timeout`; fall back to a no-op.
have_timeout=0
command -v timeout >/dev/null 2>&1 && have_timeout=1
run() {
    if [ "$have_timeout" = "1" ]; then
        timeout "$TIMEOUT" "$@" 2>/dev/null
    else
        "$@" 2>/dev/null
    fi
}

emit() { echo "$1=$2"; }

# ----------------------------------------------------------------------
# Identity
# ----------------------------------------------------------------------
emit codename     "$(getprop ro.product.device 2>/dev/null)"
emit rom          "$(getprop ro.build.flavor 2>/dev/null)"
emit android_ver  "$(getprop ro.build.version.release 2>/dev/null)"

# ----------------------------------------------------------------------
# Framebuffer (fbdev)
# ----------------------------------------------------------------------
if [ -e /dev/graphics/fb0 ]; then
    emit fbdev_present 1
    if [ -r /sys/class/graphics/fb0/virtual_size ]; then
        emit fbdev_virtual_size "$(cat /sys/class/graphics/fb0/virtual_size 2>/dev/null | tr -d ' ')"
    fi
    for f in /sys/class/graphics/fb0/rotate /sys/class/graphics/fb0/rotation; do
        if [ -r "$f" ]; then
            emit fbdev_rotation "$(cat "$f" 2>/dev/null | tr -d ' ')"
            break
        fi
    done
else
    emit fbdev_present 0
fi

# ----------------------------------------------------------------------
# screencap
# ----------------------------------------------------------------------
if command -v screencap >/dev/null 2>&1; then
    emit screencap_present 1
    mkdir -p /data/local/tmp/peko-phase1 2>/dev/null
    if run screencap -p /data/local/tmp/peko-phase1/cap.png; then
        sz=$(wc -c < /data/local/tmp/peko-phase1/cap.png 2>/dev/null | tr -d ' ')
        emit screencap_size_bytes "${sz:-0}"
    else
        emit screencap_size_bytes 0
    fi
else
    emit screencap_present 0
fi

# ----------------------------------------------------------------------
# DRM
# ----------------------------------------------------------------------
drm=""
for c in /dev/dri/card0 /dev/dri/card1; do
    if [ -e "$c" ]; then
        drm="$c"
        break
    fi
done
if [ -n "$drm" ]; then
    emit drm_present 1
    emit drm_path "$drm"
else
    emit drm_present 0
fi

# ----------------------------------------------------------------------
# Touchscreen evdev — sysfs name scan (no streaming!)
# ----------------------------------------------------------------------
touch_node=""
for n in /sys/class/input/event*; do
    [ -d "$n" ] || continue
    name=$(cat "$n/device/name" 2>/dev/null)
    case "$(echo "$name" | tr 'A-Z' 'a-z')" in
        *touch*|*synaptics*|*fts*|*goodix*|*atmel*|*himax*|*novatek*|*focaltech*|*ilitek*|*melfas*)
            touch_node="/dev/input/$(basename "$n")"
            emit touch_name "$name"
            break
            ;;
    esac
done
if [ -z "$touch_node" ] && [ -e /dev/input/event2 ]; then
    touch_node=/dev/input/event2
fi
if [ -n "$touch_node" ]; then
    emit touch_node "$touch_node"
    # `getevent -lp` prints capabilities and EXITS. Bounded by timeout
    # for paranoia about non-standard getevent builds.
    cap=$(run getevent -lp "$touch_node")
    abs_x=$(echo "$cap" | awk -F'max ' '/ABS_MT_POSITION_X/{print $2}' | awk '{print $1}' | tr -d ',' | head -1)
    abs_y=$(echo "$cap" | awk -F'max ' '/ABS_MT_POSITION_Y/{print $2}' | awk '{print $1}' | tr -d ',' | head -1)
    pres=$(echo "$cap" | awk -F'max ' '/ABS_MT_PRESSURE/{print $2}'   | awk '{print $1}' | tr -d ',' | head -1)
    tmaj=$(echo "$cap" | awk -F'max ' '/ABS_MT_TOUCH_MAJOR/{print $2}'| awk '{print $1}' | tr -d ',' | head -1)
    [ -n "${abs_x:-}" ] && emit touch_abs_x_max "$abs_x"
    [ -n "${abs_y:-}" ] && emit touch_abs_y_max "$abs_y"
    [ -n "${pres:-}" ] && emit touch_pressure_max "$pres"
    [ -n "${tmaj:-}" ] && emit touch_major_max "$tmaj"
fi

# ----------------------------------------------------------------------
# Display dimensions
# ----------------------------------------------------------------------
wm=$(run wm size 2>/dev/null)
size=$(echo "$wm" | grep -oE '[0-9]+x[0-9]+' | head -1)
[ -n "$size" ] && emit wm_size "$size"

# ----------------------------------------------------------------------
# wpa_supplicant control socket
#
# wpa_supplicant on Android creates a control socket at one of:
#   - <dir>/wpa_ctrl_global       — global interface (some ROMs)
#   - <dir>/<iface>                — interface-specific (LineageOS 20)
#   - <dir>/wpa_ctrl_<pid>-<n>    — per-client sockets (ignore)
#
# We probe both the well-known global path and any interface-named
# socket inside the sockets dir, then fall back to reporting the dir
# itself so Phase 3 can pick the right entry at runtime.
# ----------------------------------------------------------------------
wpa_path=""
for path in \
    /data/vendor/wifi/wpa/sockets/wpa_ctrl_global \
    /data/misc/wifi/sockets/wpa_ctrl_global \
    /data/vendor/wifi/wpa/sockets/wpa_ctrl \
    /data/misc/wifi/sockets/wpa_ctrl
do
    if [ -e "$path" ]; then
        wpa_path="$path"
        break
    fi
done
if [ -z "$wpa_path" ]; then
    for d in /data/vendor/wifi/wpa/sockets /data/misc/wifi/sockets; do
        [ -d "$d" ] || continue
        for iface in wlan0 wlan1 p2p0 ap0; do
            if [ -e "$d/$iface" ]; then
                wpa_path="$d/$iface"
                break 2
            fi
        done
    done
fi
if [ -z "$wpa_path" ]; then
    for d in /data/vendor/wifi/wpa/sockets /data/misc/wifi/sockets; do
        if [ -d "$d" ]; then
            emit wpa_ctrl_dir "$d"
            break
        fi
    done
else
    emit wpa_ctrl_path "$wpa_path"
fi

emit done 1
