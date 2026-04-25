#!/usr/bin/env bash
# Phase 9: DRM master + dumb buffer paint test for Lane A on Qualcomm.
#
# Default mode: --enumerate only. Reports the connector / encoder /
# CRTC the agent would target plus the preferred mode. Does NOT
# acquire master. Safe to run while the framework is up.
#
# Paint mode: requires both BLIT_OK=1 AND --paint to actually try to
# scan out a canvas. SET_MASTER will return EBUSY when SurfaceFlinger
# holds the device, which is the entire framework. So this only
# produces visible output in Lane A boot or with SF stopped (the
# latter is unsafe on sdm845; see lane-a-sdm845-finding.md).

set -uo pipefail
ADB=${ADB:-adb}
BIN=/data/local/tmp/peko-drm-test
HOLD_MS=${HOLD_MS:-5000}
TEXT=${TEXT:-AGENT BOOTED}

step() { printf "\n\033[1;34m▶ %s\033[0m\n" "$*"; }
ok()   { printf "  \033[1;32m✓\033[0m %s\n" "$*"; }
warn() { printf "  \033[1;33m!\033[0m %s\n" "$*"; }

step "Preflight"
"$ADB" get-state >/dev/null 2>&1 || { echo "no adb device"; exit 1; }
"$ADB" shell "test -x $BIN" \
    || { echo "push peko-drm-test to $BIN first"; exit 1; }
ok "device + binary ready"

step "Enumerate (safe, no DRM master)"
"$ADB" shell "su -c $BIN" 2>&1 | sed 's/^/    /'

if [[ "${BLIT_OK:-0}" != "1" ]]; then
    warn "BLIT_OK not set — skipping --paint."
    warn "Paint cycle requires Lane A boot or SF stopped (unsafe on sdm845)."
    exit 0
fi

step "Paint cycle (DANGER on sdm845 with SF running)"
"$ADB" shell "su -c '$BIN --paint --i-know-what-im-doing --hold-ms $HOLD_MS --text \"$TEXT\"'" 2>&1 | sed 's/^/    /'
