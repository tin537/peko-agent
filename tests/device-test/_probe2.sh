#!/system/bin/sh
# Device-side probe for Phase 2: sensors + battery.
#
# Walks /sys/bus/iio (modern path), /sys/class/power_supply, and the
# input subsystem looking for devices that expose sensor data. Emits
# key=value pairs + a `done=1` sentinel like phase 1. Bounded by
# timeout-per-probe; nothing here streams.

set -u
TIMEOUT="${TIMEOUT:-2}"
have_timeout=0
command -v timeout >/dev/null 2>&1 && have_timeout=1
run() {
    if [ "$have_timeout" = "1" ]; then timeout "$TIMEOUT" "$@" 2>/dev/null
    else "$@" 2>/dev/null; fi
}
emit() { echo "$1=$2"; }

# ----------------------------------------------------------------------
# Identity (cheap, lets phase2.sh sanity-check the same device)
# ----------------------------------------------------------------------
emit codename "$(getprop ro.product.device 2>/dev/null)"
emit rom "$(getprop ro.build.flavor 2>/dev/null)"

# ----------------------------------------------------------------------
# IIO subsystem
# ----------------------------------------------------------------------
iio_count=0
for d in /sys/bus/iio/devices/iio:device*; do
    [ -d "$d" ] || continue
    name=$(cat "$d/name" 2>/dev/null)
    [ -z "$name" ] && continue
    iio_count=$((iio_count + 1))
    base=$(basename "$d")
    emit "iio_${base}_name" "$name"
    emit "iio_${base}_path" "$d"
    # Channels: in_<chan>_raw + in_<chan>_scale + in_<chan>_offset
    for f in "$d"/in_*_raw "$d"/in_*_input; do
        [ -e "$f" ] || continue
        key=$(basename "$f" | sed -E 's/^in_//; s/_raw$//; s/_input$//')
        v=$(cat "$f" 2>/dev/null | tr -d ' ')
        [ -n "$v" ] && emit "iio_${base}_${key}_value" "$v"
        scale_file="$d/in_${key}_scale"
        if [ ! -e "$scale_file" ]; then
            # Some drivers share a scale across all axes of a sensor type
            type=$(echo "$key" | sed -E 's/_(x|y|z)$//')
            scale_file="$d/in_${type}_scale"
        fi
        if [ -e "$scale_file" ]; then
            sv=$(cat "$scale_file" 2>/dev/null | tr -d ' ')
            [ -n "$sv" ] && emit "iio_${base}_${key}_scale" "$sv"
        fi
    done
done
emit iio_device_count "$iio_count"

# ----------------------------------------------------------------------
# /sys/class/power_supply — battery + charger probes
# ----------------------------------------------------------------------
ps_count=0
for d in /sys/class/power_supply/*; do
    [ -d "$d" ] || continue
    name=$(basename "$d")
    type=$(cat "$d/type" 2>/dev/null)
    emit "ps_${name}_type" "${type:-Unknown}"
    ps_count=$((ps_count + 1))
    for k in capacity status health voltage_now current_now temp present technology charge_type; do
        if [ -r "$d/$k" ]; then
            v=$(cat "$d/$k" 2>/dev/null | tr -d ' ')
            [ -n "$v" ] && emit "ps_${name}_${k}" "$v"
        fi
    done
done
emit ps_count "$ps_count"

# ----------------------------------------------------------------------
# /sys/class/sensors — vendor sensor HAL exposure (Qualcomm)
# ----------------------------------------------------------------------
vs_count=0
for d in /sys/class/sensors/*; do
    [ -d "$d" ] || continue
    name=$(basename "$d")
    vs_count=$((vs_count + 1))
    emit "vs_${name}_path" "$d"
    if [ -r "$d/name" ]; then
        emit "vs_${name}_name" "$(cat "$d/name" 2>/dev/null)"
    fi
    if [ -r "$d/type" ]; then
        emit "vs_${name}_type" "$(cat "$d/type" 2>/dev/null | tr -d ' ')"
    fi
done
emit vs_count "$vs_count"

# ----------------------------------------------------------------------
# Input subsystem: light / proximity / accelerometer-as-input
# ----------------------------------------------------------------------
input_count=0
for n in /sys/class/input/event*; do
    [ -d "$n" ] || continue
    name=$(cat "$n/device/name" 2>/dev/null)
    base=$(basename "$n")
    case "$(echo "$name" | tr 'A-Z' 'a-z')" in
        *prox*|*als*|*light*|*accel*|*gyro*|*magnet*|*compass*|*sensor*)
            input_count=$((input_count + 1))
            emit "input_${base}_name" "$name"
            emit "input_${base}_node" "/dev/input/$base"
            ;;
    esac
done
emit input_sensor_count "$input_count"

emit done 1
