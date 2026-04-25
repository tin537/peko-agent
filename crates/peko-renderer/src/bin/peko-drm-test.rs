//! Standalone DRM blit test for Lane A on Qualcomm SoCs.
//!
//! Modes:
//!   --enumerate (default)  : list connectors / modes / CRTC; safe.
//!                             Does NOT acquire DRM master.
//!   --paint                : full master + dumb buffer + addfb +
//!                             setcrtc cycle; UNSAFE on a device where
//!                             SurfaceFlinger holds master.
//!
//! The --paint flow requires `--i-know-what-im-doing` so we don't
//! brick anyone's daily driver by accident.

use peko_renderer::{drm_paint_to_panel, drm_pick_target, Canvas, DrmBlitError, Rgba};
use std::path::PathBuf;
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    let target_path = arg_value(&args, "--target")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/dev/dri/card0"));
    let text = arg_value(&args, "--text").unwrap_or_else(|| "AGENT BOOTED".to_string());
    let hold_ms: u64 = arg_value(&args, "--hold-ms")
        .and_then(|s| s.parse().ok())
        .unwrap_or(5000);

    let target = match drm_pick_target(&target_path) {
        Ok(t) => t,
        Err(DrmBlitError::NotPresent { .. }) => {
            eprintln!("DRM device {} not present", target_path.display());
            return ExitCode::from(1);
        }
        Err(e) => {
            eprintln!("pick_target failed: {e}");
            return ExitCode::from(1);
        }
    };
    println!(
        "DRM target: {} connector#{} encoder#{} crtc#{} {}x{}@{} ({})",
        target.device_path.display(),
        target.connector_id,
        target.encoder_id,
        target.crtc_id,
        target.width,
        target.height,
        target.refresh_hz,
        target.mode_name
    );

    if !args.iter().any(|a| a == "--paint") {
        println!("(--paint not given; enumeration only)");
        return ExitCode::SUCCESS;
    }
    if !args.iter().any(|a| a == "--i-know-what-im-doing") {
        eprintln!(
            "--paint requires --i-know-what-im-doing.\n\
             SurfaceFlinger probably holds DRM master right now;\n\
             if it does, SET_MASTER returns EBUSY and we just bail.\n\
             If it doesn't (Lane A boot, SF stopped, etc.) we'll\n\
             paint and the panel may flicker. Caller's call."
        );
        return ExitCode::from(2);
    }

    // Build the canvas at the target's mode dimensions.
    let mut canvas = Canvas::new(target.width, target.height, Rgba(0, 0, 0, 255));
    let scale = 8u32;
    let glyph_w = (5 + 1) * scale; // glyph + spacing
    let glyph_h = (7 + 1) * scale;
    let text_w = glyph_w as i32 * text.chars().count() as i32;
    let x = (target.width as i32 - text_w) / 2;
    let y = (target.height as i32 - glyph_h as i32) / 2;
    canvas.draw_text(x, y, &text, Rgba(0, 255, 0, 255), scale);

    let pad = 30;
    canvas.stroke_rect(
        pad,
        pad,
        target.width - 2 * pad as u32,
        target.height - 2 * pad as u32,
        Rgba(255, 255, 255, 255),
    );

    let buf = canvas.into_buffer();
    println!("painting for {hold_ms} ms...");
    match drm_paint_to_panel(&target, &buf, hold_ms) {
        Ok(written) => {
            println!("wrote {written} bytes; teardown complete.");
            ExitCode::SUCCESS
        }
        Err(DrmBlitError::Ioctl { ioctl: "SET_MASTER", err }) => {
            eprintln!(
                "SET_MASTER failed: {err}\n\
                 SurfaceFlinger almost certainly holds master.\n\
                 This is expected in Lane B and is not a bug — paint\n\
                 only works in Lane A or with SF stopped."
            );
            ExitCode::from(3)
        }
        Err(e) => {
            eprintln!("paint_to_panel failed: {e}");
            ExitCode::from(4)
        }
    }
}

fn arg_value(args: &[String], key: &str) -> Option<String> {
    let i = args.iter().position(|a| a == key)?;
    args.get(i + 1).cloned()
}
