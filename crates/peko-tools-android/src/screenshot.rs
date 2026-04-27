use peko_core::tool::{Tool, ToolResult};
use peko_hal::{auto_capture, probe_drm_default, DisplayCapture, FbdevCapture, ScreencapCapture};
use serde_json::json;
use std::future::Future;
use std::io::Cursor;
use std::pin::Pin;

use crate::screen_state::ensure_awake;

/// Max dimension for LLM — resize larger screenshots to save tokens and bandwidth.
/// 720p is enough for UI element recognition while keeping base64 under ~100KB.
const MAX_DIMENSION: u32 = 720;

pub struct ScreenshotTool;

impl ScreenshotTool {
    pub fn new() -> Self { Self }

    /// Backwards-compat: callers used to pass a FramebufferDevice. The
    /// actual device is now resolved per-call from `auto_capture`, so
    /// this constructor is a no-op shim. Kept so the existing
    /// `ScreenshotTool::new(fb)` call sites in main.rs don't need to
    /// change before Phase 2.
    pub fn with_framebuffer(_device: peko_hal::FramebufferDevice) -> Self { Self }

    pub fn unavailable() -> Self { Self }
}

impl Default for ScreenshotTool {
    fn default() -> Self { Self::new() }
}

/// Resize image to fit within MAX_DIMENSION and encode as JPEG quality 80.
/// Returns (base64, width, height, size_kb).
fn resize_and_encode(img: &image::DynamicImage) -> (String, u32, u32, usize, &'static str) {
    let (orig_w, orig_h) = (img.width(), img.height());

    let resized = if orig_w > MAX_DIMENSION || orig_h > MAX_DIMENSION {
        let scale = MAX_DIMENSION as f32 / orig_w.max(orig_h) as f32;
        let new_w = (orig_w as f32 * scale) as u32;
        let new_h = (orig_h as f32 * scale) as u32;
        img.resize_exact(new_w, new_h, image::imageops::FilterType::Triangle)
    } else {
        img.clone()
    };

    let (w, h) = (resized.width(), resized.height());

    let mut bytes = Vec::new();
    let mut cursor = Cursor::new(&mut bytes);

    // Try JPEG first (smaller, lossy is fine for vision LLM input).
    // Fall back to PNG only if JPEG encoding fails (rare — typically
    // alpha-channel weirdness on certain framebuffer sources).
    let format = if resized.write_to(&mut cursor, image::ImageFormat::Jpeg).is_ok() {
        "image/jpeg"
    } else {
        bytes.clear();
        cursor = Cursor::new(&mut bytes);
        match resized.write_to(&mut cursor, image::ImageFormat::Png) {
            Ok(_) => "image/png",
            Err(_) => {
                // Both encoders failed — return an empty payload with
                // a marker MIME so the caller can detect it explicitly
                // instead of seeing a "successful" empty JPEG.
                bytes.clear();
                "image/x-encode-failed"
            }
        }
    };

    let size_kb = bytes.len() / 1024;
    let b64 = base64::Engine::encode(
        &base64::engine::general_purpose::STANDARD,
        &bytes,
    );

    (b64, w, h, size_kb, format)
}

fn capture_via_backend(mut backend: Box<dyn DisplayCapture>) -> anyhow::Result<ToolResult> {
    let name = backend.backend_name();
    let buf = backend
        .capture()
        .map_err(|e| anyhow::anyhow!("capture via {name}: {e}"))?;
    let (w, h) = (buf.width, buf.height);
    if buf.data.len() < (w as usize) * (h as usize) * 4 {
        anyhow::bail!("backend {name} returned undersized buffer");
    }
    let img = image::RgbaImage::from_raw(w, h, buf.data)
        .ok_or_else(|| anyhow::anyhow!("backend {name} returned mis-sized buffer"))?;
    let dyn_img = image::DynamicImage::ImageRgba8(img);
    let (b64, ow, oh, size_kb, mime) = resize_and_encode(&dyn_img);
    if mime == "image/x-encode-failed" {
        anyhow::bail!("backend {name} produced a buffer that neither JPEG nor PNG encoder accepted");
    }
    Ok(ToolResult::with_image(
        format!(
            "Screenshot via {name} ({ow}x{oh}, {size_kb}KB, {mime}). The image is attached."
        ),
        b64,
        mime.to_string(),
    ))
}

fn capture_with_mode(mode: &str) -> anyhow::Result<ToolResult> {
    match mode {
        "info" => Ok(ToolResult::success(diagnostics_text())),
        "fb" | "fbdev" => {
            let cap = FbdevCapture::open_default()
                .map_err(|e| anyhow::anyhow!("fbdev unavailable: {e}"))?;
            capture_via_backend(Box::new(cap))
        }
        "screencap" => {
            let cap = ScreencapCapture::new()
                .map_err(|e| anyhow::anyhow!("screencap unavailable: {e}"))?;
            capture_via_backend(Box::new(cap))
        }
        "drm" => {
            // DRM pixel capture lands in Phase 8 (Lane A, frameworkless).
            // Until then we report what DRM tells us so the agent can
            // see the device exists, even if we can't read its pixels yet.
            let info = probe_drm_default()
                .map_err(|e| anyhow::anyhow!("DRM probe failed: {e}"))?;
            match info {
                Some(d) => Ok(ToolResult::error(format!(
                    "DRM capture not yet implemented (Phase 8). Device {} runs driver '{}' \
                     with {} connector(s). Use mode='auto' or 'fb' for now.",
                    d.device_path,
                    d.driver,
                    d.connectors.len()
                ))),
                None => Ok(ToolResult::error(
                    "no /dev/dri device found; DRM capture impossible".to_string(),
                )),
            }
        }
        "auto" | "" => {
            let cap = auto_capture(Some("auto"))
                .map_err(|e| anyhow::anyhow!("no display backend available: {e}"))?;
            capture_via_backend(cap)
        }
        other => Ok(ToolResult::error(format!(
            "unknown screenshot mode '{}'. valid: auto, fb, screencap, drm, info",
            other
        ))),
    }
}

/// One-shot diagnostics dump for the agent to understand the display
/// stack. Designed to be cheap and never fail — every probe degrades to
/// "unavailable: <reason>" rather than erroring out the whole call.
fn diagnostics_text() -> String {
    let mut out = String::from("Display capture diagnostics:\n");

    out.push_str(&format!("  fbdev: "));
    match FbdevCapture::open_default() {
        Ok(c) => {
            let (w, h) = c.dimensions();
            out.push_str(&format!("OK ({}x{}, rotation={:?})\n", w, h, c.rotation()));
        }
        Err(e) => out.push_str(&format!("unavailable ({e})\n")),
    }

    out.push_str(&format!("  screencap: "));
    match ScreencapCapture::new() {
        Ok(_) => out.push_str("OK (binary present)\n"),
        Err(e) => out.push_str(&format!("unavailable ({e})\n")),
    }

    out.push_str(&format!("  DRM: "));
    match probe_drm_default() {
        Ok(Some(info)) => {
            out.push_str(&format!(
                "OK (device={}, driver={}, kernel={}, connectors={})\n",
                info.device_path,
                info.driver,
                info.kernel_version,
                info.connectors.len()
            ));
            for c in &info.connectors {
                let modes = c
                    .modes
                    .iter()
                    .take(2)
                    .map(|m| format!("{}x{}@{}", m.width, m.height, m.refresh_hz))
                    .collect::<Vec<_>>()
                    .join(", ");
                out.push_str(&format!(
                    "    connector#{} {} {} {}\n",
                    c.id,
                    c.kind,
                    if c.connected { "CONNECTED" } else { "disconnected" },
                    modes
                ));
            }
        }
        Ok(None) => out.push_str("not present (no /dev/dri/card*)\n"),
        Err(e) => out.push_str(&format!("probe failed ({e})\n")),
    }

    out
}

impl Tool for ScreenshotTool {
    fn name(&self) -> &str { "screenshot" }

    fn description(&self) -> &str {
        "Capture the current screen. Returns a resized JPEG (720p max). \
         mode='auto' (default) tries the best backend; 'fb' for direct framebuffer, \
         'screencap' for SurfaceFlinger, 'drm' for DRM (Phase 8), 'info' for diagnostics."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "mode": {
                    "type": "string",
                    "enum": ["auto", "fb", "screencap", "drm", "info"],
                    "description": "Backend selection. Default 'auto'."
                }
            },
            "required": []
        })
    }

    fn execute(
        &self,
        args: serde_json::Value,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<ToolResult>> + Send + '_>> {
        Box::pin(async move {
            let mode = args["mode"].as_str().unwrap_or("auto").to_string();
            // Diagnostics never need wake — they're synthetic.
            if mode != "info" {
                ensure_awake();
            }
            // Run the blocking capture on a dedicated thread so we don't
            // park the tokio reactor. Capture itself takes 80–300ms on
            // sdm845 and blocks in mmap / fork+exec.
            tokio::task::spawn_blocking(move || capture_with_mode(&mode))
                .await
                .map_err(|e| anyhow::anyhow!("capture task panicked: {e}"))?
        })
    }
}
