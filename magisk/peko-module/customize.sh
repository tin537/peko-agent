#!/system/bin/sh
# customize.sh — runs inside Magisk during module install. Used for
# compatibility gates and install-time diagnostics. Magisk exports:
#   $MODPATH  — module staging dir
#   $ARCH     — arm / arm64 / x86 / x64
#   $API      — SDK level of the running system
#   $ZIPFILE  — path to the installing zip
# See https://topjohnwu.github.io/Magisk/guides.html#customization

SKIPUNZIP=0

ui_print "*************************************"
ui_print "     Peko Agent — AI-as-OS"
ui_print "     Autonomous agent for Android"
ui_print "*************************************"
ui_print ""

# ─── Arch check ──────────────────────────────────────────────────
# OnePlus 6T (fajita), Pixels, most modern devices are arm64.
if [ "$ARCH" != "arm64" ]; then
    ui_print "! Detected arch: $ARCH"
    ui_print "! This module ships arm64-v8a binaries only."
    ui_print "! Aborting install to avoid exec-format crashes on boot."
    abort "! Unsupported architecture"
fi
ui_print "- arch: arm64-v8a  OK"

# ─── API level check ─────────────────────────────────────────────
# peko-agent is built against android-31 NDK sysroot but the overlay
# and runtime features only need API 26 (Android 8).
if [ "$API" -lt 26 ]; then
    ui_print "! Android API $API is below the minimum supported (26 = Android 8)."
    abort "! Android too old"
fi
ui_print "- android API: $API  OK"

# ─── Binary presence ─────────────────────────────────────────────
if [ ! -f "$MODPATH/system/bin/peko-agent" ]; then
    abort "! peko-agent binary missing from module — rebuild with magisk/build-module.sh"
fi
set_perm "$MODPATH/system/bin/peko-agent" 0 0 0755

if [ -f "$MODPATH/system/bin/peko-llm-daemon" ]; then
    set_perm "$MODPATH/system/bin/peko-llm-daemon" 0 0 0755
    ui_print "- local LLM daemon: bundled"
else
    ui_print "- local LLM daemon: NOT bundled (cloud-only mode)"
fi

# Priv-app overlay is optional.
if [ -f "$MODPATH/system/priv-app/PekoOverlay/PekoOverlay.apk" ]; then
    set_perm "$MODPATH/system/priv-app/PekoOverlay/PekoOverlay.apk" 0 0 0644
    ui_print "- floating overlay app: bundled"
fi

# ─── Config seeding note ─────────────────────────────────────────
ui_print ""
ui_print "* After reboot:"
ui_print "  1) Edit /data/peko/config.toml to set your API keys"
ui_print "     (or export ANTHROPIC_API_KEY etc. before service.sh runs)"
ui_print "  2) adb forward tcp:8080 tcp:8080"
ui_print "  3) Open  http://localhost:8080  in a browser"
ui_print ""
ui_print "* Uninstall: remove the module from Magisk → Modules."
ui_print ""
