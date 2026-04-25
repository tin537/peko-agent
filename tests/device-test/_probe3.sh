#!/system/bin/sh
# Phase 3 device probe — wifi reachability checks.
# Times out every external command; never streams.

set -u
TIMEOUT="${TIMEOUT:-5}"
have_timeout=0
command -v timeout >/dev/null 2>&1 && have_timeout=1
run() {
    if [ "$have_timeout" = "1" ]; then timeout "$TIMEOUT" "$@" 2>/dev/null
    else "$@" 2>/dev/null; fi
}
emit() { echo "$1=$2"; }

emit codename "$(getprop ro.product.device 2>/dev/null)"
emit rom "$(getprop ro.build.flavor 2>/dev/null)"

# cmd wifi presence + status
if command -v cmd >/dev/null 2>&1; then
    emit cmd_present 1
    out=$(run cmd wifi status 2>/dev/null)
    if [ -n "$out" ]; then
        emit cmd_wifi_works 1
        ssid=$(echo "$out" | sed -nE 's/.*Wifi is connected to "([^"]+)".*/\1/p' | head -1)
        [ -n "$ssid" ] && emit cmd_wifi_ssid "$ssid"
        rssi=$(echo "$out" | sed -nE 's/.*RSSI: (-?[0-9]+).*/\1/p' | head -1)
        [ -n "$rssi" ] && emit cmd_wifi_rssi "$rssi"
        ip=$(echo "$out" | sed -nE 's|.*IP: /([0-9.]+).*|\1|p' | head -1)
        [ -n "$ip" ] && emit cmd_wifi_ip "$ip"
    else
        emit cmd_wifi_works 0
    fi
    n_scan=$(run cmd wifi list-scan-results 2>/dev/null | tail -n +2 | wc -l | tr -d ' ')
    [ -n "$n_scan" ] && emit cmd_wifi_scan_count "$n_scan"
    n_saved=$(run cmd wifi list-networks 2>/dev/null | tail -n +2 | awk '{print $1}' | sort -u | wc -l | tr -d ' ')
    [ -n "$n_saved" ] && emit cmd_wifi_saved_count "$n_saved"
else
    emit cmd_present 0
fi

# wpa_supplicant socket
for path in \
    /data/vendor/wifi/wpa/sockets/wlan0 \
    /data/vendor/wifi/wpa/sockets/wpa_ctrl_global \
    /data/misc/wifi/sockets/wpa_ctrl
do
    if [ -e "$path" ]; then
        emit wpa_socket_path "$path"
        # Check if it's actually a socket
        if [ "$(ls -la "$path" 2>/dev/null | cut -c1)" = "s" ]; then
            emit wpa_socket_is_socket 1
        fi
        break
    fi
done

emit done 1
