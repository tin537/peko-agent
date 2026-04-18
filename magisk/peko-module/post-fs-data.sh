#!/system/bin/sh
# post-fs-data.sh — runs very early boot, before data partition is decrypted.
# Used only for setup that MUST happen before Zygote. Keep minimal to avoid
# boot delays.

MODDIR=${0%/*}

# Ensure /data/peko exists with right perms the instant /data is mounted.
# Runtime (peko-agent) will create subdirs, but having the root here means
# detected_hardware.json can be written on first probe.
mkdir -p /data/peko
chmod 0750 /data/peko
chown 0:0 /data/peko

# If the module ships a default config and /data/peko/config.toml doesn't
# exist yet, seed it. This only runs once per install.
if [ ! -f /data/peko/config.toml ] && [ -f "$MODDIR/system/etc/peko/config.toml" ]; then
    cp "$MODDIR/system/etc/peko/config.toml" /data/peko/config.toml
    chmod 0640 /data/peko/config.toml
fi

# Same for SOUL.md
if [ ! -f /data/peko/SOUL.md ] && [ -f "$MODDIR/system/etc/peko/SOUL.md" ]; then
    cp "$MODDIR/system/etc/peko/SOUL.md" /data/peko/SOUL.md
    chmod 0640 /data/peko/SOUL.md
fi

# Model dir — models are large, user places them here manually via
#   adb push <model.gguf> /data/peko/models/
mkdir -p /data/peko/models
chmod 0750 /data/peko/models
