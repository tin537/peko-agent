#!/usr/bin/env bash
# Phase 3 on-device test: wifi backends.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
ADB=${ADB:-adb}
PROBE_SRC="$REPO_ROOT/tests/device-test/_probe3.sh"
PROBE_DST=/data/local/tmp/peko_probe3.sh

step() { printf "\n\033[1;34m▶ %s\033[0m\n" "$*"; }
ok()   { printf "  \033[1;32m✓\033[0m %s\n" "$*"; }
warn() { printf "  \033[1;33m!\033[0m %s\n" "$*"; }
fail() { printf "  \033[1;31m✗\033[0m %s\n" "$*"; FAIL=1; }

FAIL=0

step "Preflight"
"$ADB" get-state >/dev/null 2>&1 || { echo "no adb device"; exit 1; }
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

step "cmd wifi backend"
if [[ "$(get cmd_present)" == "1" && "$(get cmd_wifi_works)" == "1" ]]; then
    ok "cmd wifi available"
    [[ -n "$(get cmd_wifi_ssid)" ]] && ok "  ssid=$(get cmd_wifi_ssid)"
    [[ -n "$(get cmd_wifi_rssi)" ]] && ok "  rssi=$(get cmd_wifi_rssi) dBm"
    [[ -n "$(get cmd_wifi_ip)" ]] && ok "  ip=$(get cmd_wifi_ip)"
    SCAN_N=$(get cmd_wifi_scan_count)
    SAVED_N=$(get cmd_wifi_saved_count)
    [[ -n "$SCAN_N" ]] && ok "  scan_results=$SCAN_N"
    [[ -n "$SAVED_N" ]] && ok "  saved_networks=$SAVED_N"
else
    fail "cmd wifi unavailable on this build"
fi

step "wpa_supplicant ctrl socket"
WPA=$(get wpa_socket_path)
if [[ -n "$WPA" ]]; then
    ok "socket present at $WPA"
    if [[ "$(get wpa_socket_is_socket)" == "1" ]]; then
        ok "  is a UNIX socket"
    fi
else
    warn "no wpa_supplicant socket reachable (Lane A backend will fall back to cmd wifi)"
fi

step "End-to-end: agent's wifi_tool via dispatch"
# Cross-compile the agent to ARM is heavy; instead we exercise the
# parser logic on the host with the *same* `cmd wifi` outputs we
# captured. The host-side cargo tests (`cargo test --workspace --lib`)
# cover that. Here we just smoke-test that the device's `cmd wifi`
# output remains shape-compatible with the parser by checking the
# tokens our parser scans for.
RAW_STATUS=$("$ADB" shell 'su -c "cmd wifi status"' 2>/dev/null | tr -d '\r')
if echo "$RAW_STATUS" | grep -q "Supplicant state: COMPLETED" \
   || echo "$RAW_STATUS" | grep -q "Wifi is disconnected"; then
    ok "cmd wifi status output is parser-compatible"
else
    fail "cmd wifi status output drifted from parser expectations"
    echo "$RAW_STATUS" | sed 's/^/    /' | head -5
fi

step "Summary"
if [[ "$FAIL" -eq 0 ]]; then
    printf "\n\033[1;32mPhase 3 PASS\033[0m\n"
    exit 0
else
    printf "\n\033[1;31mPhase 3 FAIL\033[0m\n"
    exit 1
fi
