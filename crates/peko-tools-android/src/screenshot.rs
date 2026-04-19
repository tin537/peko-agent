use peko_core::tool::{Tool, ToolResult};
use peko_hal::FramebufferDevice;
use serde_json::json;
use std::future::Future;
use std::io::Cursor;
use std::pin::Pin;
use std::sync::Arc;
use tokio::sync::Mutex;

use crate::screen_state::ensure_awake;

/// Max dimension for LLM — resize larger screenshots to save tokens and bandwidth.
/// 720p is enough for UI element recognition while keeping base64 under ~100KB.
const MAX_DIMENSION: u32 = 720;

pub struct ScreenshotTool {
    device: Option<Arc<Mutex<FramebufferDevice>>>,
}

impl ScreenshotTool {
    pub fn new(device: FramebufferDevice) -> Self {
        Self { device: Some(Arc::new(Mutex::new(device))) }
    }

    pub fn unavailable() -> Self {
        Self { device: None }
    }

    fn capture_via_screencap() -> anyhow::Result<ToolResult> {
        let output = std::process::Command::new("screencap")
            .arg("-p")
            .output()?;

        if !output.status.success() {
            return Ok(ToolResult::error("screencap command failed"));
        }

        // Decode PNG, resize, re-encode as JPEG
        let img = image::load_from_memory(&output.stdout)?;
        let (b64, w, h, size_kb) = resize_and_encode(&img);

        Ok(ToolResult::with_image(
            format!("Screenshot captured ({}x{}, {}KB). The image is attached.", w, h, size_kb),
            b64,
            "image/jpeg".to_string(),
        ))
    }
}

/// Resize image to fit within MAX_DIMENSION and encode as JPEG quality 80.
/// Returns (base64, width, height, size_kb).
fn resize_and_encode(img: &image::DynamicImage) -> (String, u32, u32, usize) {
    let (orig_w, orig_h) = (img.width(), img.height());

    let resized = if orig_w > MAX_DIMENSION || orig_h > MAX_DIMENSION {
        // Preserve aspect ratio: scale so the largest dimension fits MAX_DIMENSION
        let scale = MAX_DIMENSION as f32 / orig_w.max(orig_h) as f32;
        let new_w = (orig_w as f32 * scale) as u32;
        let new_h = (orig_h as f32 * scale) as u32;
        img.resize_exact(new_w, new_h, image::imageops::FilterType::Triangle)
    } else {
        img.clone()
    };

    let (w, h) = (resized.width(), resized.height());

    // Encode as JPEG (much smaller than PNG, good enough for LLM vision)
    let mut jpeg_bytes = Vec::new();
    let mut cursor = Cursor::new(&mut jpeg_bytes);

    // Try JPEG first, fall back to PNG if JPEG encoder not available
    let _format = if resized.write_to(&mut cursor, image::ImageFormat::Jpeg).is_ok() {
        "image/jpeg"
    } else {
        jpeg_bytes.clear();
        cursor = Cursor::new(&mut jpeg_bytes);
        resized.write_to(&mut cursor, image::ImageFormat::Png).unwrap();
        "image/png"
    };

    let size_kb = jpeg_bytes.len() / 1024;
    let b64 = base64::Engine::encode(
        &base64::engine::general_purpose::STANDARD,
        &jpeg_bytes,
    );

    (b64, w, h, size_kb)
}

impl Tool for ScreenshotTool {
    fn name(&self) -> &str { "screenshot" }

    fn description(&self) -> &str {
        "Capture the current screen. Returns a resized JPEG image (720p max) for efficient vision analysis."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {},
            "required": []
        })
    }

    fn execute(
        &self,
        _args: serde_json::Value,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<ToolResult>> + Send + '_>> {
        Box::pin(async move {
            // Wake the display + dismiss the keyguard before capturing,
            // otherwise a dozing phone produces a blank image.
            ensure_awake();

            // Prefer `screencap` (goes through SurfaceFlinger, returns the
            // composited display at native resolution) over direct
            // framebuffer reads. On sdm845 and similar, /dev/graphics/fb0
            // is a stale low-res AOD buffer and returns black, so the old
            // "fb first, screencap fallback" order silently produced a
            // useless image. The _device field is kept so devices that
            // really do need fb access (headless, no SurfaceFlinger) can
            // be special-cased later.
            Self::capture_via_screencap()
        })
    }
}
