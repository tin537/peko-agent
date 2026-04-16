# Device Requirements

> What hardware you need to build and test Peko Agent.

---

## Development Machine

Any macOS or Linux machine with:
- Rust toolchain (stable, latest)
- Android NDK (r25+)
- ADB installed
- Internet access (for LLM API calls)

Phases 1-2 can be completed entirely on the development machine.

## Target Android Device

### Minimum Requirements

| Requirement | Minimum | Recommended |
|---|---|---|
| Architecture | ARM64 (aarch64) | ARM64 |
| Android version | 10+ (API 29) | 12+ (API 31) |
| RAM | 2 GB | 4 GB+ |
| Root access | **Required** | Required |
| Bootloader | Unlockable | Unlocked |
| Display | Any | 1080x1920+ for better vision |
| Connectivity | WiFi | WiFi + LTE (for SMS/calls) |
| SIM slot | Optional | Required (for telephony tools) |

### Root Access

**Root is mandatory.** The agent needs:
- Write access to `/dev/input/*` (touch injection)
- Read access to `/dev/graphics/fb0` or `/dev/dri/*` (screenshots)
- Access to `/dev/ttyACM*` (modem)
- Ability to modify `/system` partition (for init.rc deployment)

Root methods:
- **Magisk** — recommended, most compatible
- **KernelSU** — alternative for newer devices
- **Custom ROM** — full control, can build Peko Agent into the ROM

### Recommended Devices (Budget-Friendly)

| Device | Price range | Why |
|---|---|---|
| Google Pixel 3a/4a | $50-100 used | Easy bootloader unlock, great Magisk support, standard hardware |
| Xiaomi Redmi Note series | $80-150 | Unlockable bootloader, common Qualcomm chipset |
| OnePlus 6/7/8 | $80-150 used | Developer-friendly, easy root |
| Samsung Galaxy A series | $100-200 | Common, but Knox makes rooting harder |

**Best first device**: Google Pixel 3a or 4a — cheap, well-documented, standard Android with easy root.

### Device-Specific Considerations

| Chipset | Modem interface | Framebuffer | Input device names |
|---|---|---|---|
| Qualcomm Snapdragon | `/dev/smd7` or QMI | `/dev/graphics/fb0` | Varies by OEM |
| MediaTek | `/dev/ttyACM0` | `/dev/graphics/fb0` | `mtk-tpd` |
| Samsung Exynos | `/dev/ttyACM0` | `/dev/graphics/fb0` | `sec_touchscreen` |
| Google Tensor | `/dev/ttyACM0` | DRM only | `fts_ts` |

### Frameworkless Mode Requirements

For the full Agent-as-OS experience (no Zygote):

- Device must be functional with modified init.rc
- Display must work without SurfaceFlinger (framebuffer/DRM direct access)
- Network must work without ConnectivityService (kernel TCP/IP)
- Modem must be accessible via AT commands (not all QMI modems support this)

## Emulator Option (Limited)

Android Emulator via Android Studio can be used for Phase 1-2 development:

```bash
# Create an emulator with API 31
avdmanager create avd -n test_peko -k "system-images;android-31;google_apis;x86_64"
emulator -avd test_peko -writable-system
```

**Limitations**:
- No real touchscreen → evdev testing limited
- No real modem → telephony tools don't work
- Framebuffer behavior differs from real hardware
- Performance doesn't reflect real mobile hardware

**Best for**: Testing the agent loop, transport, and shell/file tools. Not for HAL or full integration.

## SIM Card

For telephony tools (SMS, calls):
- Any active SIM with voice + SMS plan
- Prepaid SIMs are cheapest for testing
- Consider a dedicated test SIM to avoid accidental charges

## Related

- [[Implementation-Roadmap]] — When you need the device
- [[Phase-3-Hardware]] — First phase requiring a device
- [[Phase-6-Android-Deploy]] — Full deployment
- [[../knowledge/Cross-Compilation]] — Building for the device

---

#roadmap #hardware #devices
