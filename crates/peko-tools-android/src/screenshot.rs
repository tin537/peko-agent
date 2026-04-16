use peko_core::tool::{Tool, ToolResult};
use peko_hal::FramebufferDevice;
use serde_json::json;
use std::future::Future;
use std::io::Cursor;
use std::pin::Pin;
use std::sync::Arc;
use tokio::sync::Mutex;

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

        let b64 = base64::Engine::encode(
            &base64::engine::general_purpose::STANDARD,
            &output.stdout,
        );
        Ok(ToolResult::with_image(
            "Screenshot captured via screencap".to_string(),
            b64,
            "image/png".to_string(),
        ))
    }
}

impl Tool for ScreenshotTool {
    fn name(&self) -> &str { "screenshot" }

    fn description(&self) -> &str {
        "Capture the current screen as a PNG image. Returns a base64-encoded screenshot."
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
            if let Some(ref device) = self.device {
                let dev = device.lock().await;
                let buffer = dev.capture()?;

                let img = image::ImageBuffer::<image::Rgba<u8>, _>::from_raw(
                    buffer.width,
                    buffer.height,
                    buffer.data,
                ).ok_or_else(|| anyhow::anyhow!("failed to create image buffer"))?;

                let mut png_bytes = Vec::new();
                img.write_to(&mut Cursor::new(&mut png_bytes), image::ImageFormat::Png)?;

                let b64 = base64::Engine::encode(
                    &base64::engine::general_purpose::STANDARD,
                    &png_bytes,
                );
                Ok(ToolResult::with_image(
                    format!("Screenshot captured ({}x{})", buffer.width, buffer.height),
                    b64,
                    "image/png".to_string(),
                ))
            } else {
                Self::capture_via_screencap()
            }
        })
    }
}
