#!/bin/bash
# collect_selinux_denials.sh — Collect and convert SELinux denials to allow rules
#
# Usage:
#   ./scripts/collect_selinux_denials.sh            # Show denials
#   ./scripts/collect_selinux_denials.sh --generate  # Generate allow rules
#
# Run this while peko-agent is running to catch all denials,
# then add the generated rules to rom/sepolicy/peko_agent.te

set -euo pipefail

echo "=== SELinux Denials for peko_agent ==="
echo ""

# Collect from dmesg
echo "--- From dmesg ---"
adb shell su -c "dmesg" 2>/dev/null | grep peko_agent | grep "avc:  denied" | tail -30

echo ""
echo "--- From logcat ---"
adb logcat -d | grep peko_agent | grep "avc:  denied" | tail -30

if [ "${1:-}" = "--generate" ]; then
    echo ""
    echo "=== Generated allow rules ==="
    echo ""

    # Collect all denials and pipe through audit2allow format
    DENIALS=$(adb shell su -c "dmesg" 2>/dev/null | grep peko_agent | grep "avc:  denied")

    if [ -z "$DENIALS" ]; then
        echo "No denials found!"
        exit 0
    fi

    # Parse denials into allow rules (simplified audit2allow)
    echo "$DENIALS" | while IFS= read -r line; do
        # Extract fields
        ACTION=$(echo "$line" | grep -oP '(?<=\{ )[^}]+')
        SCONTEXT=$(echo "$line" | grep -oP '(?<=scontext=)[^ ]+' | cut -d: -f3)
        TCONTEXT=$(echo "$line" | grep -oP '(?<=tcontext=)[^ ]+' | cut -d: -f3)
        TCLASS=$(echo "$line" | grep -oP '(?<=tclass=)[^ ]+')

        if [ -n "$SCONTEXT" ] && [ -n "$TCONTEXT" ] && [ -n "$TCLASS" ]; then
            echo "allow $SCONTEXT $TCONTEXT:$TCLASS { $ACTION};"
        fi
    done | sort -u

    echo ""
    echo "Add these to rom/sepolicy/peko_agent.te"
fi
