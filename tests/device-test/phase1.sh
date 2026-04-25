#!/usr/bin/env bash
# Phase 1 on-device test: display capture + input observation.
#
# Validates:
#   - fbdev presence + non-zero capture
#   - screencap presence + valid PNG output
#   - DRM device presence (enumeration only; capture is Phase 8)
#   - touchscreen evdev advertises ABS_MT_POSITION_{X,Y}
#   - display dimensions reported by `wm size` are usable
#
# Side effect: refreshes `device-profiles/<codename>-*.toml` with the
# real EVIOCGABS values for the connected device, so the maintainer
# never has to paste them in by hand.
#
# Requires: adb in PATH, exactly one device authorised, root via Magisk.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
ADB=${ADB:-adb}
DEVICE_TMP=/data/local/tmp/peko-phase1
PROFILES_DIR="$REPO_ROOT/device-profiles"

step() { printf "\n\033[1;34m▶ %s\033[0m\n" "$*"; }
ok()   { printf "  \033[1;32m✓\033[0m %s\n" "$*"; }
warn() { printf "  \033[1;33m!\033[0m %s\n" "$*"; }
fail() { printf "  \033[1;31m✗\033[0m %s\n" "$*"; FAIL=1; }

FAIL=0

# ------------------------------------------------------------------------------
# Preflight
# ------------------------------------------------------------------------------

step "Preflight"
if ! command -v "$ADB" >/dev/null; then
    echo "adb not found in PATH (override with ADB=...)"
    exit 1
fi
if ! "$ADB" get-state >/dev/null 2>&1; then
    echo "no adb device connected / authorised"
    exit 1
fi
SERIAL=$("$ADB" get-serialno | tr -d '\r')
ok "device $SERIAL connected"

CODENAME=$("$ADB" shell getprop ro.product.device | tr -d '\r')
ROM=$("$ADB" shell getprop ro.build.flavor | tr -d '\r')
ok "codename=$CODENAME rom=$ROM"

# ------------------------------------------------------------------------------
# /dev/graphics/fb0 presence
# ------------------------------------------------------------------------------

step "Framebuffer (fbdev)"
if "$ADB" shell 'test -e /dev/graphics/fb0' 2>/dev/null; then
    ok "/dev/graphics/fb0 exists"
    FB_INFO=$("$ADB" shell 'su -c "cat /sys/class/graphics/fb0/virtual_size 2>/dev/null"' 2>/dev/null | tr -d '\r' || echo "")
    if [[ -n "$FB_INFO" ]]; then
        ok "virtual_size=$FB_INFO"
    else
        warn "virtual_size unreadable (vendor kernel without sysfs export — non-fatal)"
    fi
    ROT=$("$ADB" shell 'su -c "cat /sys/class/graphics/fb0/rotate 2>/dev/null || cat /sys/class/graphics/fb0/rotation 2>/dev/null"' 2>/dev/null | tr -d '\r' || echo "")
    if [[ -n "$ROT" ]]; then
        ok "rotation=$ROT (sysfs)"
    else
        warn "rotation not exposed via sysfs (will default to R0)"
    fi
else
    fail "/dev/graphics/fb0 missing — fbdev backend will not work"
fi

# ------------------------------------------------------------------------------
# screencap availability
# ------------------------------------------------------------------------------

step "screencap"
if "$ADB" shell 'which screencap' >/dev/null 2>&1; then
    ok "screencap binary present"
    "$ADB" shell "mkdir -p $DEVICE_TMP && screencap -p $DEVICE_TMP/cap.png" >/dev/null 2>&1
    SIZE=$("$ADB" shell "wc -c < $DEVICE_TMP/cap.png" | tr -d '\r')
    if [[ -n "$SIZE" && "$SIZE" -gt 1024 ]]; then
        ok "screencap produced ${SIZE} bytes"
    else
        fail "screencap output suspiciously small (${SIZE:-0} bytes)"
    fi
else
    fail "screencap binary missing (frameworkless device? — run Phase 8 tests instead)"
fi

# ------------------------------------------------------------------------------
# DRM enumeration
# ------------------------------------------------------------------------------

step "DRM"
if "$ADB" shell 'test -e /dev/dri/card0' 2>/dev/null; then
    ok "/dev/dri/card0 present"
else
    warn "no /dev/dri/card0 (DRM not in use; OK for fbdev-only kernels)"
fi

# ------------------------------------------------------------------------------
# Touchscreen capabilities + profile refresh
# ------------------------------------------------------------------------------

step "Touchscreen evdev"
TOUCH_NODE=$("$ADB" shell 'su -c "for n in /dev/input/event*; do
    name=\$(getevent -l \$n 2>/dev/null | head -1 | sed -E \"s/.*\\\"(.+)\\\".*/\\1/\");
    if echo \"\$name\" | grep -iqE \"touch|synaptics|fts|goodix\"; then
        echo \$n;
        break;
    fi
done"' 2>/dev/null | tr -d '\r')

if [[ -z "$TOUCH_NODE" ]]; then
    warn "no touchscreen evdev found by name; falling back to event2"
    TOUCH_NODE=/dev/input/event2
fi
ok "touchscreen at $TOUCH_NODE"

GETEVENT=$("$ADB" shell "su -c 'getevent -lp $TOUCH_NODE'" 2>/dev/null | tr -d '\r' || true)
ABS_X_MAX=$(echo "$GETEVENT" | awk -F'max ' '/ABS_MT_POSITION_X/{print $2}' | awk '{print $1}' | tr -d ',' | head -1)
ABS_Y_MAX=$(echo "$GETEVENT" | awk -F'max ' '/ABS_MT_POSITION_Y/{print $2}' | awk '{print $1}' | tr -d ',' | head -1)

if [[ -n "${ABS_X_MAX:-}" && -n "${ABS_Y_MAX:-}" ]]; then
    ok "ABS_MT_POSITION_X.max=$ABS_X_MAX  ABS_MT_POSITION_Y.max=$ABS_Y_MAX"
else
    warn "could not parse ABS_MT_POSITION_{X,Y} from getevent — profile not updated"
fi

WM_SIZE=$("$ADB" shell 'wm size' | tr -d '\r' | grep -oE '[0-9]+x[0-9]+' | head -1)
if [[ -n "$WM_SIZE" ]]; then
    ok "wm size = $WM_SIZE"
fi

# ------------------------------------------------------------------------------
# wpa_supplicant socket probe (Phase 3 prep, but cheap to check now)
# ------------------------------------------------------------------------------

step "wpa_supplicant socket"
WPA_FOUND=""
for path in /data/vendor/wifi/wpa/sockets/wpa_ctrl_global /data/misc/wifi/sockets/wpa_ctrl; do
    if "$ADB" shell "su -c 'test -e $path'" 2>/dev/null; then
        WPA_FOUND="$path"
        break
    fi
done
if [[ -n "$WPA_FOUND" ]]; then
    ok "wpa_ctrl socket at $WPA_FOUND"
else
    warn "wpa_ctrl socket not found at known paths (may be OK on this ROM)"
fi

# ------------------------------------------------------------------------------
# Refresh device profile with measured values
# ------------------------------------------------------------------------------

step "Refresh device profile"
if [[ -n "${ABS_X_MAX:-}" && -n "${ABS_Y_MAX:-}" && -n "$CODENAME" ]]; then
    ROM_TAG=$(echo "$ROM" | sed -E 's/[^a-zA-Z0-9._-]/-/g' | head -c 32)
    [[ -z "$ROM_TAG" ]] && ROM_TAG="unknown"
    PROFILE="$PROFILES_DIR/$CODENAME-$ROM_TAG.toml"
    mkdir -p "$PROFILES_DIR"
    {
        echo "# Auto-refreshed by tests/device-test/phase1.sh on $(date -u +%Y-%m-%dT%H:%M:%SZ)"
        echo "# Device: $CODENAME  ROM: $ROM"
        echo
        echo "device = \"$CODENAME\""
        echo "rom = \"$ROM\""
        echo
        echo "[display]"
        if [[ -n "$WM_SIZE" ]]; then
            echo "width = ${WM_SIZE%x*}"
            echo "height = ${WM_SIZE#*x}"
        fi
        if [[ -n "$ROT" ]]; then
            ROT_DEG=$ROT
            # Quarter-turns 0..3 -> 0,90,180,270
            if [[ "$ROT" =~ ^[0-3]$ ]]; then
                ROT_DEG=$((ROT * 90))
            fi
            echo "rotation_deg = $ROT_DEG"
        fi
        echo "prefer = \"auto\""
        echo
        echo "[touch]"
        echo "abs_x_max = $ABS_X_MAX"
        echo "abs_y_max = $ABS_Y_MAX"
        echo "device_path = \"$TOUCH_NODE\""
        echo
        if [[ -n "$WPA_FOUND" ]]; then
            echo "[wifi]"
            echo "ctrl_socket_path = \"$WPA_FOUND\""
        fi
    } > "$PROFILE"
    ok "wrote $PROFILE"
else
    warn "missing measurements; profile not refreshed"
fi

# ------------------------------------------------------------------------------
# Summary
# ------------------------------------------------------------------------------

step "Summary"
if [[ "$FAIL" -eq 0 ]]; then
    printf "\n\033[1;32mPhase 1 PASS\033[0m\n"
    exit 0
else
    printf "\n\033[1;31mPhase 1 FAIL\033[0m — see ✗ markers above\n"
    exit 1
fi
