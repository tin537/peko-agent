# peko_overlay.mk — OnePlus 6T (fajita) LineageOS build, peko-optimized.
#
# Append to device/oneplus/fajita/lineage_fajita.mk OR inherit explicitly:
#   $(call inherit-product, device/peko/common/lineage-fajita/peko_overlay.mk)
#
# Produces a LineageOS-fajita build with:
#   - peko-agent + peko-llm-daemon in /system/bin, autostarted at boot
#   - AOSP bloat stripped (see remove_apps.mk)
#   - Performance-oriented prop overrides (see boot_tuning.mk)
#   - SELinux policy from rom/sepolicy/

PEKO_COMMON := device/peko/common

# ─── Pull in peko subsystems ────────────────────────────────
$(call inherit-product, $(PEKO_COMMON)/rom/layout/device_peko.mk)
$(call inherit-product, $(PEKO_COMMON)/rom/lineage-fajita/remove_apps.mk)
$(call inherit-product, $(PEKO_COMMON)/rom/lineage-fajita/boot_tuning.mk)

# ─── Packages to install ────────────────────────────────────
PRODUCT_PACKAGES += \
    peko-agent \
    peko-llm-daemon \
    peko-config-default \
    peko_agent_sepolicy

# ─── Ship a default model in /system/etc/peko/models/ (optional) ─
# Models are large; prefer pulling from /data on first boot via OTA
# channel. Uncomment only if you want to ship a model in-image.
#
# PRODUCT_COPY_FILES += \
#     $(PEKO_COMMON)/models/qwen2.5-1.5b-q4.gguf:$(TARGET_COPY_OUT_SYSTEM)/etc/peko/models/local.gguf

# ─── Ship SOUL.md default persona ─────────────────────────
PRODUCT_COPY_FILES += \
    $(PEKO_COMMON)/SOUL.md:$(TARGET_COPY_OUT_SYSTEM)/etc/peko/SOUL.md

# ─── SELinux additional policy directories ─────────────────
BOARD_SEPOLICY_DIRS += $(PEKO_COMMON)/rom/sepolicy

# ─── Init scripts registered automatically via Android.bp ──
# peko-agent.rc + peko-frameworkless.rc come from
# $(PEKO_COMMON)/rom/layout/Android.bp via cc_prebuilt_binary { init_rc: [...] }

# ─── Product identifiers — so this shows up distinct from stock LOS ─
PRODUCT_NAME         := lineage_fajita_peko
PRODUCT_DEVICE       := fajita
PRODUCT_MANUFACTURER := OnePlus
PRODUCT_BRAND        := OnePlus
PRODUCT_MODEL        := OnePlus 6T (Peko)
