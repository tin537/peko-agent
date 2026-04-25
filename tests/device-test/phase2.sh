#!/usr/bin/env bash
# Phase 2 on-device test: sensors + battery.
#
# Pushes _probe2.sh, runs it under root, parses key=value, then
# validates each capability lit up by Phase 2 in peko-hal.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
ADB=${ADB:-adb}
PROBE_SRC="$REPO_ROOT/tests/device-test/_probe2.sh"
PROBE_DST=/data/local/tmp/peko_probe2.sh
PROFILES_DIR="$REPO_ROOT/device-profiles"

step() { printf "\n\033[1;34m▶ %s\033[0m\n" "$*"; }
ok()   { printf "  \033[1;32m✓\033[0m %s\n" "$*"; }
warn() { printf "  \033[1;33m!\033[0m %s\n" "$*"; }
fail() { printf "  \033[1;31m✗\033[0m %s\n" "$*"; FAIL=1; }

FAIL=0

step "Preflight"
command -v "$ADB" >/dev/null || { echo "adb not in PATH"; exit 1; }
"$ADB" get-state >/dev/null 2>&1 || { echo "no adb device"; exit 1; }
SERIAL=$("$ADB" get-serialno | tr -d '\r')
ok "device $SERIAL"

step "Push probe"
"$ADB" push "$PROBE_SRC" "$PROBE_DST" >/dev/null
"$ADB" shell "chmod +x $PROBE_DST" >/dev/null
ok "probe at $PROBE_DST"

step "Run probe"
REPORT=$("$ADB" shell "su -c $PROBE_DST 2>/dev/null || $PROBE_DST" 2>/dev/null | tr -d '\r' || true)
echo "$REPORT" | grep -q '^done=1$' || { fail "probe did not finish"; echo "$REPORT" | sed 's/^/    /'; exit 1; }
ok "probe completed"

get() { echo "$REPORT" | awk -F= -v k="$1" '$1==k{print substr($0, length($1)+2); exit}'; }
list_keys() { echo "$REPORT" | awk -F= -v p="$1" 'index($1,p)==1{print $1}'; }

CODENAME=$(get codename)
ROM=$(get rom)

# ----------------------------------------------------------------------
# IIO sensors
# ----------------------------------------------------------------------
step "IIO sensors"
IIO_COUNT=$(get iio_device_count)
if [[ -z "$IIO_COUNT" || "$IIO_COUNT" -eq 0 ]]; then
    warn "no IIO devices on this kernel (sensors may live under /sys/class/sensors instead)"
else
    ok "$IIO_COUNT IIO device(s)"
    for k in $(list_keys 'iio_' | grep '_name$' | sort -u); do
        dev=${k#iio_}
        dev=${dev%_name}
        n=$(get "$k")
        path=$(get "iio_${dev}_path")
        ok "  $dev: $n  ($path)"
    done
fi

# ----------------------------------------------------------------------
# Battery
# ----------------------------------------------------------------------
step "Battery"
BATT_TYPE=$(get ps_battery_type)
if [[ -n "$BATT_TYPE" ]]; then
    cap=$(get ps_battery_capacity)
    sta=$(get ps_battery_status)
    hth=$(get ps_battery_health)
    vol=$(get ps_battery_voltage_now)
    cur=$(get ps_battery_current_now)
    tmp=$(get ps_battery_temp)
    ok "battery present (type=$BATT_TYPE)"
    [[ -n "$cap" ]] && ok "  capacity=${cap}%"
    [[ -n "$sta" ]] && ok "  status=$sta"
    [[ -n "$hth" ]] && ok "  health=$hth"
    [[ -n "$vol" ]] && ok "  voltage_now=${vol} µV"
    [[ -n "$cur" ]] && ok "  current_now=${cur} µA"
    [[ -n "$tmp" ]] && ok "  temp=${tmp} (deci-°C)"
else
    fail "no /sys/class/power_supply/battery"
fi

# ----------------------------------------------------------------------
# Vendor /sys/class/sensors (Qualcomm path)
# ----------------------------------------------------------------------
step "Vendor sensors (/sys/class/sensors)"
VS_COUNT=$(get vs_count)
if [[ -n "$VS_COUNT" && "$VS_COUNT" -gt 0 ]]; then
    ok "$VS_COUNT vendor sensor node(s)"
    for k in $(list_keys 'vs_' | grep '_name$' | sort -u); do
        dev=${k#vs_}
        dev=${dev%_name}
        n=$(get "$k")
        ok "  $dev: $n"
    done
else
    warn "no /sys/class/sensors (non-Qualcomm or stripped kernel)"
fi

# ----------------------------------------------------------------------
# Input-subsystem sensors
# ----------------------------------------------------------------------
step "Input-subsystem sensor nodes"
ISC=$(get input_sensor_count)
if [[ -n "$ISC" && "$ISC" -gt 0 ]]; then
    for k in $(list_keys 'input_event' | grep '_name$' | sort -u); do
        ev=${k#input_}
        ev=${ev%_name}
        n=$(get "$k")
        node=$(get "input_${ev}_node")
        ok "  $node: $n"
    done
else
    warn "no input devices match sensor name patterns"
fi

# ----------------------------------------------------------------------
# Refresh device profile [sensors] block
# ----------------------------------------------------------------------
step "Refresh device profile [sensors] block"
ROM_TAG=$(echo "$ROM" | sed -E 's/[^a-zA-Z0-9._-]/-/g' | head -c 32)
PROFILE="$PROFILES_DIR/$CODENAME-$ROM_TAG.toml"
if [[ -f "$PROFILE" ]]; then
    # Strip any prior [sensors] block (everything from `[sensors]` until
    # the next bracketed section or EOF), then append fresh values.
    awk '
        BEGIN { skip = 0 }
        /^\[sensors\]/ { skip = 1; next }
        /^\[/ && skip { skip = 0 }
        !skip { print }
    ' "$PROFILE" > "$PROFILE.tmp"
    {
        echo
        echo "[sensors]"
        # Map IIO devices to canonical sensor kinds when names are recognisable.
        for k in $(list_keys 'iio_' | grep '_name$' | sort -u); do
            dev=${k#iio_}
            dev=${dev%_name}
            n=$(get "$k")
            path=$(get "iio_${dev}_path")
            lower=$(echo "$n" | tr 'A-Z' 'a-z')
            kind=""
            case "$lower" in
                *accel*|bmi*|mc34*|lsm6*|kx*) kind="accel" ;;
                *gyro*|*gyr*) kind="gyro" ;;
                *mag*|*compass*|ak*|mmc*) kind="mag" ;;
                *baro*|*press*) kind="pressure" ;;
                *als*|*light*|tcs*|stk3*) kind="light" ;;
                *prox*|sx*) kind="proximity" ;;
            esac
            if [[ -n "$kind" ]]; then
                echo "$kind = { source = \"iio\", path = \"$path\", name = \"$n\" }"
            fi
        done
    } >> "$PROFILE.tmp"
    mv "$PROFILE.tmp" "$PROFILE"
    ok "wrote $PROFILE"
else
    warn "profile $PROFILE missing — run phase 1 first"
fi

step "Summary"
if [[ "$FAIL" -eq 0 ]]; then
    printf "\n\033[1;32mPhase 2 PASS\033[0m\n"
    exit 0
else
    printf "\n\033[1;31mPhase 2 FAIL\033[0m — see ✗ markers above\n"
    exit 1
fi
