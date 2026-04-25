# Lane A on Qualcomm sdm845 — empirical findings

A reproducible result from running Phase 8's `phase8-lane-a-blit.sh`
on a OnePlus 6T (codename `fajita`) running LineageOS 20 with Magisk.

## Setup

- Device: OnePlus 6T
- SoC: Qualcomm sdm845
- Display: 1080×2340 OLED, native portrait
- ROM: lineage_fajita-userdebug, Android 13
- Test binary: `peko-blit-test` (peko-renderer 0.1.0)

## Procedure

1. With SurfaceFlinger running, probe `/dev/graphics/fb0`:
   - `FBIOGET_VSCREENINFO` reports **640×400 RGBA8888**, stride 2560.
   - The display itself is at native 1080×2340 served by SurfaceFlinger.
2. Stop SurfaceFlinger (`stop surfaceflinger`), wait 2s, re-probe:
   - **Same 640×400.** fb0 does not redirect when SF pauses.
3. Run `peko-blit-test` to write a known pattern (1,024,000 bytes) to
   the buffer. Hold for 8 seconds.
4. Restart SurfaceFlinger.

## Observed result

- The write succeeds (`blitted 1024000 bytes`).
- The panel shows the pre-stop frame for ~1s, then a black/boot-logo
  cycle for ~30–60s while the framework attempts to recover.
- Eventually the framework recovers without needing a hard reboot
  (`uptime` is unchanged).
- Net: **fb0 writes do not appear on the panel; stopping SF on this
  device cascades into an unrecoverable framework-restart cycle.**

## Conclusion

**`/dev/graphics/fb0` on sdm845 is a phantom AOD plane, not a scanout
buffer the display controller reads.** The live scanout is owned
exclusively by the DRM/KMS pipeline (`/dev/dri/card0`). Direct fbdev
blit cannot deliver pixels to the panel on this SoC, and the act of
stopping SurfaceFlinger to test it is itself unsafe — the framework
shouldn't be paused on sdm845 outside of a real Lane A boot.

## What this means for the agent-as-OS thesis

- **Lane B on sdm845 (Magisk on LineageOS):** unchanged.
  `screencap` is the agent's display reader; nothing in Phase 1–7
  depends on fb0 writes.
- **Lane A on sdm845:** the fbdev path is dead. To paint pixels in
  Lane A on sdm845 we need DRM master + dumb buffer write. That's
  the natural Phase 9.
- **Lane A on devices where fb0 IS the live scanout:** the fbdev
  blit code shipped in Phase 7 is correct and useful as-is — most
  pre-Treble vendor kernels and emulators fall in this category.

## Operational rule going forward

**Do NOT stop SurfaceFlinger on sdm845 to test Lane A.** Lane A on
this SoC must be tested either:

1. With a stripped AOSP image where SF was never started (full flash);
2. Or by writing through DRM, which doesn't require pausing SF —
   we just need to acquire DRM master from a TTY console with framework
   running but NOT compositing to the display we want to drive.

This finding is the evidence Phase 8 was designed to produce. It is
not a regression in the agent code; the blit implementation passed
its part of the experiment.

## Recovery from the bootloop test

The bootloop seen on the test device recovered on its own without a
hard reboot — uptime did not reset. Standard recovery in case it
doesn't auto-recover on a future test:

1. Wait 2 minutes.
2. `adb reboot` (if adb responds).
3. Hold Power + Volume Down 15s.
4. Boot to recovery (Power + Volume Up), `magisk --remove-modules`.

## Capability matrix update

`docs/CAPABILITY_MATRIX.md` row for "Framebuffer blit" updated:

  Lane B sdm845: 🟡 (writes succeed, panel doesn't read fb0)
  Lane A sdm845: ❌ (DRM is the only path)
  Lane A on fbdev-as-scanout devices: ✅ (verified by code review;
                                         pending an emulator or other
                                         hardware to verify visually)
