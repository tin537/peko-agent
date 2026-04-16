# device_peko.mk — Device makefile snippet for Peko Agent
#
# Include this from your device's main makefile:
#   $(call inherit-product, device/<vendor>/<device>/peko/device_peko.mk)

# Binary and init scripts
PRODUCT_PACKAGES += \
    peko-agent

# Copy default config to /data on first boot
PRODUCT_COPY_FILES += \
    device/$(VENDOR)/$(DEVICE)/peko/config.example.toml:$(TARGET_COPY_OUT_SYSTEM)/etc/peko/config-default.toml

# SELinux policy
BOARD_SEPOLICY_DIRS += \
    device/$(VENDOR)/$(DEVICE)/peko/sepolicy

# System properties
PRODUCT_PROPERTY_OVERRIDES += \
    sys.peko.start=0 \
    persist.peko.frameworkless=0 \
    sys.peko.restart=0

# For frameworkless mode, also add:
# PRODUCT_PROPERTY_OVERRIDES += persist.peko.frameworkless=1

# Ensure data directory is created with correct labels
PRODUCT_DEFAULT_DEV_CERTIFICATE := build/target/product/security/testkey
