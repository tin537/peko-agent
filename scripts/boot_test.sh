#!/bin/bash
# boot_test.sh — Verify Peko Agent boot, stability, and memory footprint
#
# Usage:
#   ./scripts/boot_test.sh              # Run all tests
#   ./scripts/boot_test.sh --quick      # Quick smoke test only
#   ./scripts/boot_test.sh --stress     # Extended stability test (1 hour)
#
# Prerequisites:
#   - Peko Agent deployed on device (run deploy.sh first)
#   - Device connected via ADB

set -euo pipefail

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
CYAN='\033[0;36m'
NC='\033[0m'

PASS=0
FAIL=0
WARN=0

pass() { echo -e "  ${GREEN}PASS${NC} $1"; ((PASS++)); }
fail() { echo -e "  ${RED}FAIL${NC} $1"; ((FAIL++)); }
warn() { echo -e "  ${YELLOW}WARN${NC} $1"; ((WARN++)); }
section() { echo -e "\n${CYAN}═══ $1 ═══${NC}"; }

adb_su() { adb shell su -c "$1" 2>/dev/null; }

RSS_LIMIT_MB=50
QUICK=false
STRESS=false
for arg in "$@"; do
    case $arg in
        --quick) QUICK=true ;;
        --stress) STRESS=true ;;
    esac
done

echo -e "${CYAN}Peko Agent — Boot & Stability Test${NC}"
echo "Device: $(adb shell getprop ro.product.model | tr -d '\r')"
echo "Android: $(adb shell getprop ro.build.version.release | tr -d '\r')"
echo ""

# ═══════════════════════════════════════════════════════════
section "1. Binary Verification"
# ═══════════════════════════════════════════════════════════

if adb_su "test -f /system/bin/peko-agent" | grep -q "" 2>/dev/null; then
    BIN_SIZE=$(adb_su "ls -la /system/bin/peko-agent" | awk '{print $5}')
    BIN_SIZE_MB=$((BIN_SIZE / 1024 / 1024))
    if [ "$BIN_SIZE_MB" -lt 15 ]; then
        pass "Binary exists (${BIN_SIZE_MB}MB < 15MB target)"
    else
        warn "Binary exists but ${BIN_SIZE_MB}MB exceeds 15MB target"
    fi
else
    fail "Binary not found at /system/bin/peko-agent"
fi

# Check init.rc
adb_su "test -f /system/etc/init/peko-agent.rc" && \
    pass "init.rc installed" || fail "init.rc missing"

# Check data directory
adb_su "test -d /data/peko" && \
    pass "/data/peko exists" || fail "/data/peko missing"

# Check config
adb_su "test -f /data/peko/config.toml" && \
    pass "config.toml present" || fail "config.toml missing"

# ═══════════════════════════════════════════════════════════
section "2. Service Start"
# ═══════════════════════════════════════════════════════════

# Start the service
echo "  Starting peko-agent..."
adb_su "setprop sys.peko.start 1"
sleep 3

# Check if running
PID=$(adb_su "pidof peko-agent" | tr -d '\r')
if [ -n "$PID" ]; then
    pass "Service running (PID: $PID)"
else
    fail "Service failed to start"
    echo "  Checking logcat for errors..."
    adb logcat -d -t 20 | grep -i peko | tail -10
fi

# ═══════════════════════════════════════════════════════════
section "3. Memory Footprint"
# ═══════════════════════════════════════════════════════════

if [ -n "$PID" ]; then
    RSS_KB=$(adb_su "cat /proc/$PID/statm" | awk '{print $2}')
    PAGE_SIZE=$(adb_su "getconf PAGESIZE")
    RSS_MB=$(( RSS_KB * PAGE_SIZE / 1024 / 1024 ))

    if [ "$RSS_MB" -lt "$RSS_LIMIT_MB" ]; then
        pass "RSS: ${RSS_MB}MB < ${RSS_LIMIT_MB}MB target"
    else
        fail "RSS: ${RSS_MB}MB exceeds ${RSS_LIMIT_MB}MB target"
    fi

    # Detailed memory info
    echo "  Memory details:"
    adb_su "cat /proc/$PID/status" | grep -E "^(VmRSS|VmSize|VmPeak|Threads|FDSize)" | \
        sed 's/^/    /'

    # System memory
    TOTAL_MB=$(adb_su "cat /proc/meminfo" | grep MemTotal | awk '{print int($2/1024)}')
    FREE_MB=$(adb_su "cat /proc/meminfo" | grep MemAvailable | awk '{print int($2/1024)}')
    echo "  System: ${FREE_MB}MB free / ${TOTAL_MB}MB total"
fi

# ═══════════════════════════════════════════════════════════
section "4. Web UI"
# ═══════════════════════════════════════════════════════════

DEVICE_IP=$(adb shell ip route | grep wlan0 | awk '{print $9}' | tr -d '\r')
if [ -n "$DEVICE_IP" ]; then
    # Forward port via ADB
    adb forward tcp:8080 tcp:8080 2>/dev/null || true

    if curl -s --max-time 5 "http://localhost:8080/api/status" >/dev/null 2>&1; then
        STATUS=$(curl -s "http://localhost:8080/api/status")
        pass "Web UI responding at http://$DEVICE_IP:8080"
        echo "    Status: $STATUS"
    else
        fail "Web UI not responding on port 8080"
    fi
else
    warn "No WiFi IP — cannot test web UI"
fi

# ═══════════════════════════════════════════════════════════
section "5. Control Socket"
# ═══════════════════════════════════════════════════════════

SOCKET_EXISTS=$(adb_su "test -S /dev/socket/peko && echo yes" | tr -d '\r')
if [ "$SOCKET_EXISTS" = "yes" ]; then
    pass "Unix socket exists at /dev/socket/peko"
else
    warn "Socket not found (may be using web UI only mode)"
fi

# ═══════════════════════════════════════════════════════════
section "6. SELinux"
# ═══════════════════════════════════════════════════════════

SELINUX_MODE=$(adb_su "getenforce" | tr -d '\r')
echo "  SELinux mode: $SELINUX_MODE"

if [ "$SELINUX_MODE" = "Enforcing" ]; then
    # Check for recent denials
    DENIALS=$(adb_su "dmesg | grep peko_agent | grep 'avc:  denied' | wc -l" | tr -d '\r')
    if [ "$DENIALS" = "0" ]; then
        pass "No SELinux denials for peko_agent"
    else
        fail "$DENIALS SELinux denials found"
        echo "  Recent denials:"
        adb_su "dmesg | grep peko_agent | grep 'avc:  denied' | tail -5" | sed 's/^/    /'
    fi
else
    warn "SELinux is $SELINUX_MODE — test in Enforcing mode for production"
fi

if [ "$QUICK" = true ]; then
    section "Quick test complete"
else
    # ═══════════════════════════════════════════════════════
    section "7. Functional Tests"
    # ═══════════════════════════════════════════════════════

    if curl -s --max-time 5 "http://localhost:8080/api/status" >/dev/null 2>&1; then
        # Test session listing
        SESSIONS=$(curl -s "http://localhost:8080/api/sessions")
        pass "Session API responds: $SESSIONS"

        # Test config API
        CONFIG=$(curl -s "http://localhost:8080/api/config")
        if echo "$CONFIG" | grep -q "provider"; then
            pass "Config API responds with provider info"
        else
            fail "Config API returned unexpected response"
        fi
    fi

    # ═══════════════════════════════════════════════════════
    section "8. Hardware Access"
    # ═══════════════════════════════════════════════════════

    # Input devices
    INPUT_COUNT=$(adb_su "ls /dev/input/event* 2>/dev/null | wc -l" | tr -d '\r')
    echo "  Input devices: $INPUT_COUNT"
    [ "$INPUT_COUNT" -gt "0" ] && pass "Input devices accessible" || warn "No input devices"

    # Display
    if adb_su "test -e /dev/graphics/fb0"; then
        pass "Framebuffer available (/dev/graphics/fb0)"
    elif adb_su "test -e /dev/dri/card0"; then
        pass "DRM display available (/dev/dri/card0)"
    else
        warn "No direct display access — will use screencap fallback"
    fi

    # Modem
    MODEM=$(adb_su "ls /dev/ttyACM* /dev/ttyUSB* 2>/dev/null | head -1" | tr -d '\r')
    if [ -n "$MODEM" ]; then
        pass "Modem device: $MODEM"
    else
        warn "No modem device found (telephony tools unavailable)"
    fi
fi

# ═══════════════════════════════════════════════════════════
if [ "$STRESS" = true ]; then
    section "9. Stability Test (1 hour)"
    echo "  Monitoring RSS every 60 seconds..."

    for i in $(seq 1 60); do
        sleep 60
        PID=$(adb_su "pidof peko-agent" | tr -d '\r')
        if [ -z "$PID" ]; then
            fail "Process died after $i minutes!"
            break
        fi

        RSS_KB=$(adb_su "cat /proc/$PID/statm" | awk '{print $2}')
        PAGE_SIZE=$(adb_su "getconf PAGESIZE" 2>/dev/null || echo 4096)
        RSS_MB=$(( RSS_KB * PAGE_SIZE / 1024 / 1024 ))
        FD_COUNT=$(adb_su "ls /proc/$PID/fd 2>/dev/null | wc -l" | tr -d '\r')

        echo "  [$i min] RSS: ${RSS_MB}MB  FDs: $FD_COUNT  PID: $PID"

        if [ "$RSS_MB" -gt "$RSS_LIMIT_MB" ]; then
            warn "RSS exceeded ${RSS_LIMIT_MB}MB at minute $i"
        fi
    done

    PID=$(adb_su "pidof peko-agent" | tr -d '\r')
    if [ -n "$PID" ]; then
        pass "Process survived 1 hour stress test"
    fi
fi

# ═══════════════════════════════════════════════════════════
section "Results"
# ═══════════════════════════════════════════════════════════

echo ""
echo -e "  ${GREEN}PASS: $PASS${NC}  ${RED}FAIL: $FAIL${NC}  ${YELLOW}WARN: $WARN${NC}"
echo ""

if [ "$FAIL" -gt 0 ]; then
    echo -e "${RED}Some tests failed. Check output above.${NC}"
    exit 1
else
    echo -e "${GREEN}All critical tests passed!${NC}"
    exit 0
fi
