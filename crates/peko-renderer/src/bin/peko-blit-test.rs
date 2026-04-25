//! Standalone binary that exercises the Lane A blit path:
//! reads framebuffer geometry, renders a centered "AGENT BOOTED" string,
//! blits to /dev/graphics/fb0. No agent loop, no config, no LLM.
//!
//! Usage:
//!   peko-blit-test                            # read fb0 dimensions, blit
//!   peko-blit-test --info                     # only print fb info, no blit
//!   peko-blit-test --text "HELLO"             # custom text
//!   peko-blit-test --target /dev/graphics/fb1 # custom framebuffer

use peko_renderer::{blit::read_fb_info, blit_to_framebuffer, Canvas, Rgba};
use std::path::PathBuf;
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    let info_only = args.iter().any(|a| a == "--info");

    let target = arg_value(&args, "--target")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/dev/graphics/fb0"));
    let text = arg_value(&args, "--text").unwrap_or_else(|| "AGENT BOOTED".to_string());

    let info = match read_fb_info(&target) {
        Ok(i) => i,
        Err(e) => {
            eprintln!("read_fb_info({}) failed: {e}", target.display());
            return ExitCode::from(1);
        }
    };
    println!(
        "framebuffer {} : {}x{} bpp={} stride={} format={:?}",
        target.display(),
        info.width,
        info.height,
        info.bytes_per_pixel * 8,
        info.line_length,
        info.format
    );
    if info_only {
        return ExitCode::SUCCESS;
    }

    // Canvas matches fb dimensions so blit_to_framebuffer's
    // verify_dimensions=true path is satisfied.
    let mut canvas = Canvas::new(info.width, info.height, Rgba(0, 0, 0, 255));

    // Big centered title. 5x7 glyphs at scale=8 → 40x56 each, ~16 chars span.
    let scale = 8u32;
    let glyph_w = (5 + 1) * scale; // glyph plus spacing
    let glyph_h = (7 + 1) * scale;
    let text_w = glyph_w as i32 * text.chars().count() as i32;
    let x = (info.width as i32 - text_w) / 2;
    let y = (info.height as i32 - glyph_h as i32) / 2;
    canvas.draw_text(x, y, &text, Rgba(0, 255, 0, 255), scale);

    // A frame so we can confirm the canvas covers the whole panel.
    let pad = 20;
    canvas.stroke_rect(
        pad,
        pad,
        info.width - 2 * pad as u32,
        info.height - 2 * pad as u32,
        Rgba(255, 255, 255, 255),
    );

    let buf = canvas.into_buffer();
    match blit_to_framebuffer(&buf, &target, true) {
        Ok(n) => {
            println!("blitted {} bytes to {}", n, target.display());
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("blit failed: {e}");
            ExitCode::from(2)
        }
    }
}

fn arg_value(args: &[String], key: &str) -> Option<String> {
    let i = args.iter().position(|a| a == key)?;
    args.get(i + 1).cloned()
}
