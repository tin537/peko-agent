# Peko Agent-OS ROM

A minimal Android ROM that runs ONLY the Peko Agent agent.
No Zygote. No ART. No framework. No apps. Just kernel + agent.

## What's in the ROM

```
Boot image (kernel + ramdisk):
  kernel          ← Device-specific (prebuilt)
  ramdisk/
    init          ← Android init binary
    init.rc       ← Agent-only boot sequence
    init.peko.rc
    default.prop
    sepolicy      ← Compiled SELinux policy

System image:
  /system/
    bin/
      peko-agent   ← The agent (~8MB)
      sh              ← Shell (for debugging + ShellTool)
      toybox          ← Minimal coreutils (ls, cat, cp, etc.)
      linker64        ← Dynamic linker
      screencap       ← Screen capture (optional)
    lib64/
      libc.so         ← C library
      libm.so         ← Math library
      libdl.so        ← Dynamic loading
      liblog.so       ← Android logging
    etc/
      init/
        peko-agent.rc
      peko/
        config.toml   ← Default config
```

Total system image: ~20-30MB (vs ~5GB standard Android)

## Build

```bash
# 1. Place device kernel at:
cp your-kernel rom/agent-os/prebuilt/kernel

# 2. Extract minimal libs from stock ROM:
./rom/agent-os/scripts/extract_from_stock.sh /path/to/stock-system.img

# 3. Build peko-agent for Android:
cargo build --target aarch64-linux-android --release

# 4. Build the ROM images:
./rom/agent-os/scripts/build_rom.sh

# 5. Flash:
./rom/agent-os/scripts/flash.sh
```

## Memory Target

| Component | RAM usage |
|---|---|
| Linux kernel | ~15-20 MB |
| init process | ~1 MB |
| peko-agent | ~20-30 MB |
| System overhead | ~5 MB |
| **Total** | **~40-50 MB** |

vs standard Android: ~800-1200 MB at idle
