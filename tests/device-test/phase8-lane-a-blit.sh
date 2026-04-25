#!/usr/bin/env bash
# Phase 8: Lane A simulation via Magisk on a live Android.
#
# Stops SurfaceFlinger, runs peko-blit-test to write a known canvas
# directly to /dev/graphics/fb0, holds for 8 seconds so the operator
# can look at the panel, then restarts SurfaceFlinger.
#
# Outcome documents whether this device's fbdev path is the live
# scanout (Lane A fbdev OK) or a permanent AOD plane (Lane A needs
# DRM).
#
# SAFETY: SurfaceFlinger is always restarted, even on script abort.
# If the operator's phone freezes, hold the power button 15s — your
# data is safe, this is a runtime experiment only.

set -uo pipefail
ADB=${ADB:-adb}
HOLD_SEC=${HOLD_SEC:-8}
TARGET=${TARGET:-/dev/graphics/fb0}
TEXT=${TEXT:-AGENT BOOTED}

step() { printf "\n\033[1;34m▶ %s\033[0m\n" "$*"; }
ok()   { printf "  \033[1;32m✓\033[0m %s\n" "$*"; }
warn() { printf "  \033[1;33m!\033[0m %s\n" "$*"; }

trap 'echo; echo "[recovery] starting surfaceflinger"; "$ADB" shell "su -c \"start surfaceflinger\"" 2>/dev/null; true' EXIT

step "Preflight"
"$ADB" get-state >/dev/null 2>&1 || { echo "no adb device"; exit 1; }
"$ADB" shell 'test -x /data/local/tmp/peko-blit-test' \
    || { echo "push peko-blit-test to /data/local/tmp/ first"; exit 1; }
ok "device + binary ready"

step "Capture fb0 baseline (Lane B, SurfaceFlinger up)"
"$ADB" shell 'su -c "/data/local/tmp/peko-blit-test --info"' | sed 's/^/    /'

step "Probe fb info only — DO NOT stop SurfaceFlinger on sdm845"
# History: stopping SurfaceFlinger on sdm845 + LineageOS 20 cascades
# into a framework-restart cycle (~60s of boot-logo loop) before
# self-recovering. fb0 is a phantom plane on this SoC, so the
# experiment doesn't even produce useful pixels. We keep this script
# as a probe-only and never pause SF. Devices where fbdev IS the
# scanout (older vendor kernels, x86_64 emulators) can still
# exercise the blit by setting BLIT_OK=1 explicitly.
if [[ "${BLIT_OK:-0}" == "1" ]]; then
    warn "BLIT_OK=1 set — proceeding to stop SF + blit. Do NOT do this on sdm845."
    "$ADB" shell "su -c 'stop surfaceflinger; sleep 2; /data/local/tmp/peko-blit-test --text \"$TEXT\" --target $TARGET 2>&1'" \
        | sed 's/^/    /'
    ok "blit dispatched, holding $HOLD_SEC s — look at the phone now"
    sleep "$HOLD_SEC"
    step "Restart SurfaceFlinger"
    "$ADB" shell 'su -c "start surfaceflinger"'
    ok "SurfaceFlinger restarted (display should recover; if not, hold power 15s)"
else
    warn "BLIT_OK not set — skipping the SF-pause + blit step"
    warn "On sdm845 this is the safe default (fb0 is a phantom plane)."
    warn "On a known-fbdev-scanout device, re-run with BLIT_OK=1."
fi

step "What did you see?"
cat <<'EOF'
  Reply to the maintainer with one of:
    1) Black screen for the whole 8s
       -> fb0 is dark when SF is paused; Lane A on this device needs DRM.
    2) Text + frame visible somewhere on the panel
       -> fb0 IS the live scanout; Lane A fbdev path works.
    3) Garbage / wrong colors
       -> pixel format detection chose wrong (BGRA vs RGBA); easy fix.
    4) Frozen previous frame
       -> fb0 is a phantom buffer; display controller ignores it.
EOF
