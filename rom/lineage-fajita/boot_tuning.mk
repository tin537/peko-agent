# boot_tuning.mk — Performance-oriented build + runtime props for peko.
#
# Focus: keep peko-agent + peko-llm-daemon responsive on Snapdragon 845,
# 8GB RAM. Local LLM inference is the critical path.
#
# Included from peko_overlay.mk.

# ─── DEX optimization (faster warmup, larger /system) ──────
# "speed" = full AOT compile at build time. Trade disk for launch latency.
PRODUCT_DEX_PREOPT_DEFAULT_COMPILER_FILTER := speed

# ─── Kernel / Zygote tuning (runtime props) ────────────────
PRODUCT_PROPERTY_OVERRIDES += \
    \
    ro.config.low_ram=false \
    dalvik.vm.heapgrowthlimit=256m \
    dalvik.vm.heapsize=512m \
    dalvik.vm.heapstartsize=16m \
    \
    \
    ro.sys.fw.bg_apps_limit=8 \
    ro.config.zram=1 \
    \
    \
    ro.config.dmverity=false \
    \
    \
    persist.sys.purgeable_assets=1 \
    dalvik.vm.dex2oat-threads=4 \
    dalvik.vm.dex2oat-filter=speed \
    \
    \
    persist.radio.apm_sim_not_pwdn=1 \
    \
    \
    debug.sf.disable_backpressure=1 \
    debug.sf.early_phase_offset_ns=1500000 \
    debug.sf.early_app_phase_offset_ns=1500000 \
    debug.sf.early_gl_phase_offset_ns=3000000 \
    \
    \
    persist.peko.cpu_boost=1 \
    persist.peko.autostart=1 \
    persist.peko.frameworkless=0

# ─── Doze / App Standby bypass for peko-agent ──────────────
# So the life loop keeps ticking when the screen is off.
PRODUCT_PROPERTY_OVERRIDES += \
    ro.peko.doze_whitelist=com.peko.agent,bin.peko-agent,bin.peko-llm-daemon

# ─── Default CPU governor (set at boot by init.peko.rc) ────
# schedutil is usually best; switch to performance for shakedown if you
# want predictable latency at the cost of battery.
PRODUCT_PROPERTY_OVERRIDES += \
    persist.peko.governor=schedutil
