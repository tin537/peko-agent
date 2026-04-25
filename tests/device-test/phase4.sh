#!/usr/bin/env bash
set -euo pipefail
REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
ADB=${ADB:-adb}
PROBE_SRC="$REPO_ROOT/tests/device-test/_probe4.sh"
PROBE_DST=/data/local/tmp/peko_probe4.sh

step() { printf "\n\033[1;34m▶ %s\033[0m\n" "$*"; }
ok()   { printf "  \033[1;32m✓\033[0m %s\n" "$*"; }
warn() { printf "  \033[1;33m!\033[0m %s\n" "$*"; }
fail() { printf "  \033[1;31m✗\033[0m %s\n" "$*"; FAIL=1; }
FAIL=0

step "Preflight"
"$ADB" get-state >/dev/null 2>&1 || { echo "no device"; exit 1; }
ok "device $("$ADB" get-serialno | tr -d '\r')"

step "Push probe"
"$ADB" push "$PROBE_SRC" "$PROBE_DST" >/dev/null
"$ADB" shell "chmod +x $PROBE_DST" >/dev/null
ok "probe at $PROBE_DST"

step "Run probe"
REPORT=$("$ADB" shell "su -c $PROBE_DST 2>/dev/null || $PROBE_DST" 2>/dev/null | tr -d '\r' || true)
echo "$REPORT" | grep -q '^done=1$' || { fail "probe did not finish"; echo "$REPORT" | sed 's/^/    /'; exit 1; }
ok "probe completed"

get() { echo "$REPORT" | awk -F= -v k="$1" '$1==k{print substr($0, length($1)+2); exit}'; }

step "ALSA topology"
if [[ "$(get asound_present)" == "1" ]]; then
    n=$(get asound_card_count)
    ok "/proc/asound present, $n card(s)"
    [[ -n "$(get asound_card0)" ]] && ok "  card0 = $(get asound_card0 | tr '|' ' ')"
    pb=$(get pcm_playback_nodes); cap=$(get pcm_capture_nodes)
    [[ -n "$pb" ]] && ok "  $pb playback PCM node(s), $cap capture PCM node(s)"
else
    fail "/proc/asound missing — kernel without ALSA"
fi

step "tinymix"
if [[ "$(get tinymix_present)" == "1" ]]; then
    n=$(get tinymix_control_count)
    ok "tinymix present, $n control(s)"
else
    warn "tinymix missing on this build"
fi

step "Media volume"
v=$(get music_volume)
if [[ -n "$v" ]]; then
    ok "music stream volume: $v"
else
    warn "cmd audio get-volume music unavailable"
fi

step "Summary"
if [[ "$FAIL" -eq 0 ]]; then
    printf "\n\033[1;32mPhase 4 PASS\033[0m\n"; exit 0
else
    printf "\n\033[1;31mPhase 4 FAIL\033[0m\n"; exit 1
fi
