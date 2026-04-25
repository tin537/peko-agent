#!/usr/bin/env bash
# Phase 1 on-device test: display capture + input observation.
#
# Pushes _probe.sh to the device, runs it under root, parses the
# key=value report, and refreshes the device profile from the measured
# values. All on-device work is `timeout`-bounded so a stuck driver
# can never hang the host.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
ADB=${ADB:-adb}
PROBE_SRC="$REPO_ROOT/tests/device-test/_probe.sh"
PROBE_DST=/data/local/tmp/peko_probe.sh
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
command -v "$ADB" >/dev/null || { echo "adb not in PATH (override with ADB=...)"; exit 1; }
"$ADB" get-state >/dev/null 2>&1 || { echo "no adb device connected/authorised"; exit 1; }
SERIAL=$("$ADB" get-serialno | tr -d '\r')
ok "device $SERIAL connected"

# ------------------------------------------------------------------------------
# Push + run probe
# ------------------------------------------------------------------------------

step "Push probe script"
"$ADB" push "$PROBE_SRC" "$PROBE_DST" >/dev/null
"$ADB" shell "chmod +x $PROBE_DST" >/dev/null
ok "probe at $PROBE_DST"

step "Run probe (root, 30s wall-clock cap)"
# Try `adb root` once, then fall back to `su -c`. We pipe the output
# back and parse below — never inspect or react to the trailing
# stream from any single command.
REPORT=$(
    "$ADB" shell "su -c $PROBE_DST 2>/dev/null || $PROBE_DST" 2>/dev/null \
        | tr -d '\r' \
        || true
)
if ! echo "$REPORT" | grep -q '^done=1$'; then
    fail "probe did not complete (no done=1 marker)"
    echo "$REPORT" | sed 's/^/    /'
    exit 1
fi
ok "probe completed"

# ------------------------------------------------------------------------------
# Parse report
# ------------------------------------------------------------------------------

get() { echo "$REPORT" | awk -F= -v k="$1" '$1==k{print substr($0, length($1)+2); exit}'; }

CODENAME=$(get codename)
ROM=$(get rom)
ANDROID_VER=$(get android_ver)

step "Identity"
ok "codename=$CODENAME rom=$ROM android=$ANDROID_VER"

# ------------------------------------------------------------------------------
# Framebuffer
# ------------------------------------------------------------------------------

step "Framebuffer (fbdev)"
if [[ "$(get fbdev_present)" == "1" ]]; then
    ok "/dev/graphics/fb0 exists"
    [[ -n "$(get fbdev_virtual_size)" ]] && ok "virtual_size=$(get fbdev_virtual_size)"
    rot=$(get fbdev_rotation)
    if [[ -n "$rot" ]]; then
        ok "rotation=$rot (sysfs)"
    else
        warn "rotation not exposed via sysfs (will default to R0)"
    fi
else
    fail "/dev/graphics/fb0 missing — fbdev backend will not work"
fi

# ------------------------------------------------------------------------------
# screencap
# ------------------------------------------------------------------------------

step "screencap"
if [[ "$(get screencap_present)" == "1" ]]; then
    ok "screencap binary present"
    sz=$(get screencap_size_bytes)
    if [[ -n "$sz" && "$sz" -gt 1024 ]]; then
        ok "screencap produced ${sz} bytes"
    else
        fail "screencap output suspiciously small (${sz:-0} bytes)"
    fi
else
    fail "screencap binary missing (frameworkless device? — run Phase 8 tests instead)"
fi

# ------------------------------------------------------------------------------
# DRM
# ------------------------------------------------------------------------------

step "DRM"
if [[ "$(get drm_present)" == "1" ]]; then
    ok "$(get drm_path) present"
else
    warn "no /dev/dri/card* (DRM not in use; OK for fbdev-only kernels)"
fi

# ------------------------------------------------------------------------------
# Touchscreen evdev
# ------------------------------------------------------------------------------

step "Touchscreen evdev"
TOUCH_NODE=$(get touch_node)
TOUCH_NAME=$(get touch_name)
ABS_X_MAX=$(get touch_abs_x_max)
ABS_Y_MAX=$(get touch_abs_y_max)
PRESSURE_MAX=$(get touch_pressure_max)
TOUCH_MAJOR_MAX=$(get touch_major_max)

if [[ -n "$TOUCH_NODE" ]]; then
    ok "touchscreen at $TOUCH_NODE${TOUCH_NAME:+ ($TOUCH_NAME)}"
else
    warn "no touchscreen evdev identified"
fi

if [[ -n "$ABS_X_MAX" && -n "$ABS_Y_MAX" ]]; then
    ok "ABS_MT_POSITION_X.max=$ABS_X_MAX  ABS_MT_POSITION_Y.max=$ABS_Y_MAX"
else
    warn "could not parse ABS_MT_POSITION_{X,Y} from getevent"
fi
[[ -n "$PRESSURE_MAX" ]] && ok "ABS_MT_PRESSURE.max=$PRESSURE_MAX"
[[ -n "$TOUCH_MAJOR_MAX" ]] && ok "ABS_MT_TOUCH_MAJOR.max=$TOUCH_MAJOR_MAX"

WM_SIZE=$(get wm_size)
[[ -n "$WM_SIZE" ]] && ok "wm size = $WM_SIZE"

# ------------------------------------------------------------------------------
# wpa_supplicant socket
# ------------------------------------------------------------------------------

step "wpa_supplicant socket"
WPA_FOUND=$(get wpa_ctrl_path)
if [[ -n "$WPA_FOUND" ]]; then
    ok "wpa_ctrl socket at $WPA_FOUND"
else
    warn "wpa_ctrl socket not found at known paths (may be OK on this ROM)"
fi

# ------------------------------------------------------------------------------
# Refresh device profile
# ------------------------------------------------------------------------------

step "Refresh device profile"
if [[ -n "$ABS_X_MAX" && -n "$ABS_Y_MAX" && -n "$CODENAME" ]]; then
    ROM_TAG=$(echo "$ROM" | sed -E 's/[^a-zA-Z0-9._-]/-/g' | head -c 32)
    [[ -z "$ROM_TAG" ]] && ROM_TAG="unknown"
    PROFILE="$PROFILES_DIR/$CODENAME-$ROM_TAG.toml"
    mkdir -p "$PROFILES_DIR"
    {
        echo "# Auto-refreshed by tests/device-test/phase1.sh on $(date -u +%Y-%m-%dT%H:%M:%SZ)"
        echo "# Device: $CODENAME  ROM: $ROM  Android: $ANDROID_VER"
        echo
        echo "device = \"$CODENAME\""
        echo "rom = \"$ROM\""
        echo
        echo "[display]"
        if [[ -n "$WM_SIZE" ]]; then
            echo "width = ${WM_SIZE%x*}"
            echo "height = ${WM_SIZE#*x}"
        fi
        rot=$(get fbdev_rotation)
        if [[ -n "$rot" ]]; then
            rot_deg=$rot
            # Quarter-turns 0..3 → 0,90,180,270
            if [[ "$rot" =~ ^[0-3]$ ]]; then
                rot_deg=$((rot * 90))
            fi
            echo "rotation_deg = $rot_deg"
        fi
        echo "prefer = \"auto\""
        echo
        echo "[touch]"
        echo "abs_x_max = $ABS_X_MAX"
        echo "abs_y_max = $ABS_Y_MAX"
        [[ -n "$PRESSURE_MAX" ]] && echo "pressure_default = $((PRESSURE_MAX / 4))"
        [[ -n "$TOUCH_MAJOR_MAX" ]] && echo "touch_major_default = $((TOUCH_MAJOR_MAX / 16 + 1))"
        [[ -n "$TOUCH_NODE" ]] && echo "device_path = \"$TOUCH_NODE\""
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
