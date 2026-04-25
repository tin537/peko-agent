#!/system/bin/sh
# Phase 4 audio probe: ALSA topology + tinymix + cmd audio volume.
set -u
TIMEOUT="${TIMEOUT:-3}"
have_timeout=0
command -v timeout >/dev/null 2>&1 && have_timeout=1
run() {
    if [ "$have_timeout" = "1" ]; then timeout "$TIMEOUT" "$@" 2>/dev/null
    else "$@" 2>/dev/null; fi
}
emit() { echo "$1=$2"; }

emit codename "$(getprop ro.product.device 2>/dev/null)"

# /proc/asound presence + cards
if [ -d /proc/asound ]; then
    emit asound_present 1
    cards=$(run cat /proc/asound/cards | grep -c '^ *[0-9]')
    emit asound_card_count "$cards"
    if [ -r /proc/asound/cards ]; then
        first=$(run cat /proc/asound/cards | grep -m1 '^ *[0-9]' | sed -E 's/^ *([0-9]+) *\[([^]]*)\].*: *(.*)$/\1|\2|\3/')
        [ -n "$first" ] && emit asound_card0 "$first"
    fi
else
    emit asound_present 0
fi

# /dev/snd PCM device counts
if [ -d /dev/snd ]; then
    pb=$(ls /dev/snd 2>/dev/null | grep -c 'pcmC[0-9]*D[0-9]*p$')
    cap=$(ls /dev/snd 2>/dev/null | grep -c 'pcmC[0-9]*D[0-9]*c$')
    emit pcm_playback_nodes "$pb"
    emit pcm_capture_nodes "$cap"
fi

# tinymix availability + sample
if command -v tinymix >/dev/null 2>&1; then
    emit tinymix_present 1
    n=$(run tinymix 2>/dev/null | grep -c '^[0-9]')
    [ -n "$n" ] && emit tinymix_control_count "$n"
else
    emit tinymix_present 0
fi

# cmd audio
if command -v cmd >/dev/null 2>&1; then
    out=$(run cmd audio get-volume music 2>/dev/null)
    if [ -n "$out" ]; then
        v=$(echo "$out" | grep -oE '[0-9]+' | head -1)
        [ -n "$v" ] && emit music_volume "$v"
    fi
fi

emit done 1
