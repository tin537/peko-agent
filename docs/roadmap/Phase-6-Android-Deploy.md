# Phase 6: Android Deploy

> From standalone binary to init.rc service — the Agent-as-OS.

---

## Goal

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
