# Screen Capture

> Reading the display via framebuffer and DRM/KMS.

---

## Why Screen Capture Matters

The agent "sees" by taking screenshots and sending them to vision-capable LLMs. This is the primary perception mechanism — without it, the agent is blind.

Two kernel interfaces provide screen access:

| Interface | Device | Era | Status |
|---|---|---|---|
| Framebuffer | `/dev/graphics/fb0` | Legacy | Deprecated but widely available |
| DRM/KMS | `/dev/dri/card0` | Modern | Required on newer devices |

[[../implementation/peko-hal|peko-hal]] tries framebuffer first, falls back to DRM.

## Framebuffer (`/dev/graphics/fb0`)

### How It Works

The framebuffer is a memory region that maps directly to what's on screen. Reading it gives you raw pixel data.

```
1. Open /dev/graphics/fb0
2. ioctl(FBIOGET_VSCREENINFO) → resolution, bpp, color offsets
3. ioctl(FBIOGET_FSCREENINFO) → line length, total memory size
4. mmap() the framebuffer memory
5. Read pixels directly from the mapped memory
```

### Variable Screen Info

```rust
struct fb_var_screeninfo {
    xres: u32,           // e.g., 1080
    yres: u32,           // e.g., 2400
    bits_per_pixel: u32, // usually 32 (RGBA)
    red: fb_bitfield,    // { offset: 0, length: 8 }
    green: fb_bitfield,  // { offset: 8, length: 8 }
    blue: fb_bitfield,   // { offset: 16, length: 8 }
    transp: fb_bitfield, // { offset: 24, length: 8 }
    // ...
}
```

The color offsets tell you the pixel format (RGBA, BGRA, ABGR, etc.). Don't assume — query.

### Fixed Screen Info

```rust
struct fb_fix_screeninfo {
    line_length: u32,    // bytes per line (may include padding)
    smem_len: u32,       // total framebuffer size
    // ...
}
```

`line_length` may be larger than `xres * bytes_per_pixel` due to alignment padding. Always use `line_length` when calculating row offsets.

### Capture Code

```rust
pub fn capture(&self) -> Result<RgbaBuffer> {
    let size = self.height as usize * self.stride as usize;
    let pixels = unsafe {
        std::slice::from_raw_parts(self.mmap, size)
    };

    let mut rgba = Vec::with_capacity((self.width * self.height * 4) as usize);
    for y in 0..self.height as usize {
        let row_start = y * self.stride as usize;
        for x in 0..self.width as usize {
            let offset = row_start + x * (self.bits_per_pixel / 8) as usize;
            // Reorder based on pixel_format
            let (r, g, b, a) = self.pixel_format.decode(&pixels[offset..]);
            rgba.extend_from_slice(&[r, g, b, a]);
        }
    }

    Ok(RgbaBuffer { data: rgba, width: self.width, height: self.height })
}
```

### Limitations

- Some modern devices don't expose `fb0` at all
- If SurfaceFlinger is compositing (hybrid mode), `fb0` might not reflect the actual screen
- No V-sync — may capture mid-frame (tearing)

## DRM/KMS (`/dev/dri/card0`)

### How It Works

DRM (Direct Rendering Manager) is the modern Linux display interface. Screen capture requires:

```
1. Open /dev/dri/card0
2. Enumerate resources (connectors, CRTCs, encoders)
3. Find the active connector + CRTC
4. Get the current framebuffer ID from the CRTC
5. Create a dumb buffer and map it
6. Use DRM_IOCTL_MODE_MAP_DUMB to read the screen
```

### Resource Enumeration

```
DRM Resources:
├── Connectors (physical outputs: HDMI, DSI, eDP)
│   └── Find one with status = CONNECTED
├── Encoders (signal converters)
│   └── Links connector to CRTC
├── CRTCs (display controllers)
│   └── Has the current framebuffer_id
└── Framebuffers (pixel buffers)
    └── Contains the actual screen content
```

### Capture Flow

```rust
pub fn capture(&self) -> Result<RgbaBuffer> {
    // Get current CRTC state
    let crtc = drm_mode_get_crtc(self.fd, self.crtc_id)?;

    // Create a dumb buffer matching screen size
    let create = drm_mode_create_dumb {
        width: self.width,
        height: self.height,
        bpp: 32,
        ..Default::default()
    };
    ioctl(self.fd, DRM_IOCTL_MODE_CREATE_DUMB, &create)?;

    // Map buffer to userspace
    let map = drm_mode_map_dumb { handle: create.handle, ..Default::default() };
    ioctl(self.fd, DRM_IOCTL_MODE_MAP_DUMB, &map)?;

    let pixels = mmap(create.size, self.fd, map.offset)?;
    // Read pixels...

    // Cleanup
    ioctl(self.fd, DRM_IOCTL_MODE_DESTROY_DUMB, &create.handle)?;
    Ok(buffer)
}
```

### Advantages Over Framebuffer

- Works on modern devices where fb0 is absent
- Supports multiple displays
- V-sync aware (can capture at frame boundaries)
- More accurate screen content (handles overlays properly)

## PNG Encoding

After capturing raw RGBA pixels, encode to PNG for the LLM:

```rust
use image::{ImageBuffer, Rgba};

let img = ImageBuffer::<Rgba<u8>, _>::from_raw(
    width, height, rgba_data
).unwrap();

let mut png_bytes = Vec::new();
img.write_to(&mut Cursor::new(&mut png_bytes), image::ImageFormat::Png)?;

let base64 = base64::encode(&png_bytes);
```

### Size Considerations

| Resolution | Raw RGBA | PNG (typical) | Base64 PNG |
|---|---|---|---|
| 1080x2400 | ~10 MB | ~500 KB | ~670 KB |
| 720x1280 | ~3.5 MB | ~200 KB | ~270 KB |

Screenshots are the largest tool results by far. Consider:
- Downscaling before encoding (720p is often sufficient for LLM vision)
- JPEG instead of PNG for smaller sizes (lossy but much smaller)
- Storing screenshots externally, not in the conversation (see [[../implementation/Session-Persistence]])

## Fallback: `screencap` Binary

In hybrid mode (framework running), shell out to Android's `screencap`:

```bash
screencap -p /dev/stdout | base64
```

This goes through SurfaceFlinger and captures the properly composited screen, but adds ~100ms latency.

## Related

- [[../implementation/peko-hal]] — Framebuffer and DrmDisplay structs
- [[../implementation/peko-tools-android]] — ScreenshotTool
- [[Linux-Kernel-Interfaces]] — All kernel interfaces overview
- [[SELinux-Policy]] — Permissions for graphics devices
- [[../research/Computer-Use-Agents]] — How vision agents interpret screenshots

---

#knowledge #screen #framebuffer #drm #display
