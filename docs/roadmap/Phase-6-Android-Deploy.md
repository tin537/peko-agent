# Phase 6: Android Deploy

> From standalone binary to on-device service. Three deploy paths shipped
> — pick based on how committed you are to "Peko replaces the OS."

---

## Three deploy paths

| Path | What you install | Effort | Control |
|---|---|---|---|
| **A. Magisk module** ✅ shipped | `.zip` on top of any ROM with Magisk | 15 min | Coexists with Android |
| **B. LineageOS overlay** ✅ scaffolded | Custom ROM with peko baked in | 2-6 hr build | Agent boots as `class core` |
| **C. Stripped AOSP** (future) | Minimal ROM, peko IS the userspace | weeks | Agent-as-OS |

**Start with A.** Get peko running on a real device today. Promote to B when
you want boot-speed guarantees and process class. Reserve C for a v2 device.

### Path A: Magisk module (recommended for dev iteration)

Artifacts live at [[../../magisk/]]:
- `peko-module/module.prop` — metadata for Magisk's module manager
- `peko-module/post-fs-data.sh` — early boot: seed `/data/peko/{config.toml,SOUL.md}`
- `peko-module/service.sh` — after boot_completed: deviceidle whitelist, log
  rotation, start `peko-llm-daemon` (if GGUF present) + `peko-agent`
- `peko-module/sepolicy.rule` — minimal allows under the magisk domain
- `peko-module/system/etc/peko/config.toml` — defaults shipped with install
- `build-module.sh` — cross-builds cargo + stages binaries + zips

Flash flow:
```bash
./magisk/build-module.sh --install   # cross-build + adb push + print install steps
# On phone: Magisk app → Modules → Install from storage → pick the zip → Reboot
```

After reboot:
```bash
adb forward tcp:8080 tcp:8080 && open http://localhost:8080
adb shell cat /data/peko/detected_hardware.json   # what modem was auto-probed
```

Verified on OnePlus 6T (fajita) + LineageOS 21.

### Path B: LineageOS overlay

Artifacts live at [[../../rom/lineage-fajita/]]. Pulls the official fajita
device tree, overlays peko as `device/peko/common`, inherits from
`lineage_fajita.mk`. Uses `Docker` (or a Linux build host) via
`rom/lineage-fajita/Dockerfile` + `docker-compose.yml`.

```bash
docker compose -f rom/lineage-fajita/docker-compose.yml run --rm builder
# inside container:
./rom/lineage-fajita/build.sh --init   # repo sync (~80 GB, 30-60 min)
./rom/lineage-fajita/build.sh          # mka bacon (2-12 hr)
adb sideload out/target/product/fajita/lineage-21.0-*-fajita.zip
```

Overlay components:
- `local_manifest.xml` — pulls fajita device + peko overlay into `.repo/`
- `peko_overlay.mk` — product makefile inherited by lineage_fajita.mk
- `remove_apps.mk` — strips ~25 AOSP/LOS packages (Calendar, Gallery, Email, Music…)
- `boot_tuning.mk` — dex2oat=speed, dalvik heap, zram/lz4, SurfaceFlinger offsets, doze whitelist
- `peko-performance.rc` — runtime init service: schedutil governor, deviceidle whitelist, `peko-llm-daemon` with 5s crash backoff

### Path C: Stripped AOSP (future)

Frameworkless mode — peko replaces SystemUI + Launcher3. Scaffolding at
`rom/agent-os/` exists; not yet flashed on hardware. Covered by the
original Goal / Prerequisites section below.

---

## Goal (Paths B and C)

Peko Agent boots from Android's init process, runs as a system daemon with proper SELinux policy, and survives reboots.

## Prerequisites

- [[Phase-5-Integration]] completed (working binary)
- Rooted device with ability to modify `/system` partition
- Basic SELinux knowledge (see [[../knowledge/SELinux-Policy]])

## Tasks

### 6.1 Install Binary to System

- [ ] Remount `/system` read-write
- [ ] Copy binary to `/system/bin/peko-agent`
- [ ] Set permissions: `chmod 755`, `chown root:root`
- [ ] Set SELinux label: `chcon u:object_r:peko_agent_exec:s0`
- [ ] Create data directory: `mkdir -p /data/peko`
- [ ] Copy config: `cp config.toml /data/peko/`

### 6.2 init.rc Service File

- [ ] Create `/system/etc/init/peko-agent.rc`
- [ ] Configure as `class core` service
- [ ] Set user, groups, capabilities, seclabel
- [ ] Define socket for control interface
- [ ] Add property trigger: `on property:sys.peko.start=1`
- [ ] Test: `setprop sys.peko.start 1` → service starts

See [[../architecture/Boot-Sequence]] for the full .rc file.

### 6.3 SELinux Policy

- [ ] Create `peko_agent.te` type enforcement file
- [ ] Create `file_contexts` for binary and data paths
- [ ] Compile policy: `checkpolicy` or via AOSP build
- [ ] Load policy on device
- [ ] Run binary, collect denials: `adb logcat | grep avc`
- [ ] Iterate: add rules for each denial, recompile, reload
- [ ] Test in enforcing mode (not just permissive)

See [[../knowledge/SELinux-Policy]] for the policy files.

### 6.4 Boot Verification

- [ ] Reboot device
- [ ] Verify `peko-agent` starts automatically
- [ ] Verify socket is available at `/dev/socket/peko`
- [ ] Send a test command via socket
- [ ] Check logs: `adb logcat -s peko-agent`

### 6.5 Frameworkless Mode (Advanced)

- [ ] Modify `init.rc` to skip `class_start main` (no Zygote)
- [ ] Verify device boots to kernel + peko-agent only
- [ ] Verify all tools work without framework (framebuffer direct, evdev direct, AT commands)
- [ ] Measure memory usage: should be < 100 MB total system
- [ ] Measure boot-to-agent-ready time

### 6.6 Reliability

- [ ] Test `SIGTERM` handling (simulate `adb shell kill -TERM <pid>`)
- [ ] Test crash recovery (kill -9, verify init restarts if `oneshot` is removed)
- [ ] Test config reload via `SIGHUP`
- [ ] Run for 24 hours continuously — check for memory leaks, file handle leaks
- [ ] Stress test: 100 consecutive tasks

## Deployment Script

```bash
#!/bin/bash
# deploy.sh — push Peko Agent to a rooted device

BINARY="target/aarch64-linux-android/release/peko-agent"
DEVICE_BIN="/system/bin/peko-agent"
DEVICE_RC="/system/etc/init/peko-agent.rc"
DEVICE_DATA="/data/peko"

# Remount system
adb shell su -c "mount -o remount,rw /system"

# Push files
adb push $BINARY /data/local/tmp/peko-agent
adb shell su -c "cp /data/local/tmp/peko-agent $DEVICE_BIN"
adb shell su -c "chmod 755 $DEVICE_BIN"
adb shell su -c "chcon u:object_r:peko_agent_exec:s0 $DEVICE_BIN"

adb push peko-agent.rc /data/local/tmp/
adb shell su -c "cp /data/local/tmp/peko-agent.rc $DEVICE_RC"

# Create data directory
adb shell su -c "mkdir -p $DEVICE_DATA"
adb push config.toml /data/local/tmp/
adb shell su -c "cp /data/local/tmp/config.toml $DEVICE_DATA/"
adb shell su -c "chcon -R u:object_r:peko_data_file:s0 $DEVICE_DATA"

# Remount read-only
adb shell su -c "mount -o remount,ro /system"

echo "Deployed. Reboot or run: adb shell su -c setprop sys.peko.start 1"
```

## Definition of Done

After a clean reboot:
1. `peko-agent` is running as a system process
2. `adb shell su -c getprop init.svc.peko-agent` returns `running`
3. Control socket accepts commands
4. Agent successfully completes a multi-step task (screenshot → touch → verify)
5. SELinux is in enforcing mode with no denials
6. Memory usage < 50 MB for the peko-agent process

## Related

- [[Phase-5-Integration]] — Previous phase
- [[../architecture/Boot-Sequence]] — init.rc integration design
- [[../knowledge/SELinux-Policy]] — Policy details
- [[Device-Requirements]] — Hardware needs
- [[Challenges-And-Risks]] — Deployment challenges

---

#roadmap #phase-6 #deploy #android #init
