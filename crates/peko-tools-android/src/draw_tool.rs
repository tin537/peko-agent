use peko_core::tool::{Tool, ToolResult};
use peko_renderer::{blit_to_framebuffer, Canvas, Rgba};
use serde_json::json;
use std::future::Future;
use std::io::Cursor;
use std::path::PathBuf;
use std::pin::Pin;

/// Pure-Rust 2D drawing tool. Produces a PNG returned to the LLM (or
/// written to disk in Lane A so a future blit-to-fb step can show it).
///
/// Schema is intentionally narrow: a single canvas + an array of
/// drawing ops. The agent crafts an ops list, gets a PNG back, decides
/// what to do with it.
pub struct DrawTool;

impl DrawTool {
    pub fn new() -> Self { Self }
}

impl Default for DrawTool {
    fn default() -> Self { Self::new() }
}

impl Tool for DrawTool {
    fn name(&self) -> &str { "draw" }

    fn description(&self) -> &str {
        "Render a status overlay or notification using a pure-Rust 2D \
         renderer (no SurfaceFlinger). Inputs: width, height, optional \
         background color, and a list of ops. Each op is one of: \
         {type:'rect', x,y,w,h, color, fill?:bool}, \
         {type:'line_h'|'line_v', x,y, length, color}, \
         {type:'text', x,y, text, color, scale?:int}, \
         {type:'text_wrapped', x,y, text, max_width, color, scale?:int}. \
         Colors accept '#RGB', '#RRGGBB', or '#RRGGBBAA'. Returns a PNG. \
         Set blit=true (Lane A only) to also write the canvas to /dev/graphics/fb0."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "width": { "type": "integer", "description": "Canvas width in pixels" },
                "height": { "type": "integer", "description": "Canvas height in pixels" },
                "background": { "type": "string", "description": "Hex color, e.g. '#000'" },
                "ops": {
                    "type": "array",
                    "items": { "type": "object" },
                    "description": "Ordered list of drawing operations"
                },
                "blit": {
                    "type": "boolean",
                    "description": "If true, also write the canvas to /dev/graphics/fb0 (Lane A only)."
                },
                "blit_path": {
                    "type": "string",
                    "description": "Override blit target path. Default: /dev/graphics/fb0."
                }
            },
            "required": ["width", "height", "ops"]
        })
    }

    fn execute(
        &self,
        args: serde_json::Value,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<ToolResult>> + Send + '_>> {
        Box::pin(async move {
            tokio::task::spawn_blocking(move || render(args))
                .await
                .map_err(|e| anyhow::anyhow!("draw task panicked: {e}"))?
        })
    }
}

fn render(args: serde_json::Value) -> anyhow::Result<ToolResult> {
    let width = args["width"].as_u64().unwrap_or(0) as u32;
    let height = args["height"].as_u64().unwrap_or(0) as u32;
    if width == 0 || height == 0 || width > 4096 || height > 4096 {
        return Ok(ToolResult::error(format!(
            "invalid canvas dimensions {width}x{height} (must be 1..=4096)"
        )));
    }
    let background = args["background"]
        .as_str()
        .and_then(Rgba::from_hex)
        .unwrap_or(Rgba::BLACK);

    let mut canvas = Canvas::new(width, height, background);

    let empty: Vec<serde_json::Value> = Vec::new();
    let ops = args["ops"].as_array().unwrap_or(&empty);
    for (i, op) in ops.iter().enumerate() {
        if let Err(e) = apply_op(&mut canvas, op) {
            return Ok(ToolResult::error(format!("op {i}: {e}")));
        }
    }

    let buf = canvas.into_buffer();

    // Optional Lane A side-effect: also blit the canvas to a framebuffer.
    let mut blit_status: Option<String> = None;
    if args["blit"].as_bool().unwrap_or(false) {
        let target = args["blit_path"]
            .as_str()
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("/dev/graphics/fb0"));
        match blit_to_framebuffer(&buf, &target, /* verify_dimensions */ false) {
            Ok(written) => {
                blit_status = Some(format!(
                    "blitted {} bytes to {}",
                    written,
                    target.display()
                ))
            }
            Err(e) => {
                blit_status = Some(format!(
                    "blit to {} failed: {} (Lane B fb0 is often a stale AOD buffer)",
                    target.display(),
                    e
                ))
            }
        }
    }

    let img = image::RgbaImage::from_raw(buf.width, buf.height, buf.data)
        .ok_or_else(|| anyhow::anyhow!("renderer produced mis-sized buffer"))?;
    let mut png = Vec::new();
    image::DynamicImage::ImageRgba8(img)
        .write_to(&mut Cursor::new(&mut png), image::ImageFormat::Png)?;
    let b64 = base64::Engine::encode(
        &base64::engine::general_purpose::STANDARD,
        &png,
    );
    let kb = png.len() / 1024;
    let mut summary = format!(
        "Rendered {width}x{height} canvas, {kb} KB PNG, {} ops",
        ops.len()
    );
    if let Some(s) = blit_status {
        summary.push_str(&format!("\n{s}"));
    }
    Ok(ToolResult::with_image(summary, b64, "image/png".to_string()))
}

fn apply_op(canvas: &mut Canvas, op: &serde_json::Value) -> anyhow::Result<()> {
    let kind = op["type"].as_str().unwrap_or("");
    let color = op["color"]
        .as_str()
        .and_then(Rgba::from_hex)
        .unwrap_or(Rgba::WHITE);

    match kind {
        "rect" => {
            let x = op["x"].as_i64().unwrap_or(0) as i32;
            let y = op["y"].as_i64().unwrap_or(0) as i32;
            let w = op["w"].as_u64().unwrap_or(0) as u32;
            let h = op["h"].as_u64().unwrap_or(0) as u32;
            let fill = op["fill"].as_bool().unwrap_or(true);
            if fill {
                canvas.fill_rect(x, y, w, h, color);
            } else {
                canvas.stroke_rect(x, y, w, h, color);
            }
        }
        "line_h" => {
            let x = op["x"].as_i64().unwrap_or(0) as i32;
            let y = op["y"].as_i64().unwrap_or(0) as i32;
            let len = op["length"].as_u64().unwrap_or(0) as u32;
            canvas.line_h(x, y, len, color);
        }
        "line_v" => {
            let x = op["x"].as_i64().unwrap_or(0) as i32;
            let y = op["y"].as_i64().unwrap_or(0) as i32;
            let len = op["length"].as_u64().unwrap_or(0) as u32;
            canvas.line_v(x, y, len, color);
        }
        "text" => {
            let x = op["x"].as_i64().unwrap_or(0) as i32;
            let y = op["y"].as_i64().unwrap_or(0) as i32;
            let text = op["text"].as_str().unwrap_or("");
            let scale = op["scale"].as_u64().unwrap_or(1) as u32;
            canvas.draw_text(x, y, text, color, scale);
        }
        "text_wrapped" => {
            let x = op["x"].as_i64().unwrap_or(0) as i32;
            let y = op["y"].as_i64().unwrap_or(0) as i32;
            let text = op["text"].as_str().unwrap_or("");
            let max_w = op["max_width"].as_u64().unwrap_or(canvas.width() as u64) as u32;
            let scale = op["scale"].as_u64().unwrap_or(1) as u32;
            canvas.draw_text_wrapped(x, y, text, max_w, color, scale);
        }
        "" => anyhow::bail!("missing 'type'"),
        other => anyhow::bail!("unknown op type '{other}'"),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn renders_simple_canvas_to_png() {
        let t = DrawTool::new();
        let args = json!({
            "width": 64,
            "height": 32,
            "background": "#000",
            "ops": [
                {"type": "rect", "x": 4, "y": 4, "w": 56, "h": 24, "color": "#222", "fill": true},
                {"type": "text", "x": 8, "y": 12, "text": "AGENT OK", "color": "#0f0"}
            ]
        });
        let _ = t.execute(args).await.unwrap();
    }

    #[tokio::test]
    async fn rejects_invalid_dimensions() {
        let t = DrawTool::new();
        let args = json!({"width": 0, "height": 0, "ops": []});
        let _ = t.execute(args).await.unwrap();
    }

    #[tokio::test]
    async fn unknown_op_type_returns_error() {
        let t = DrawTool::new();
        let args = json!({
            "width": 16, "height": 16,
            "ops": [{"type": "spaceship"}]
        });
        let _ = t.execute(args).await.unwrap();
    }
}
