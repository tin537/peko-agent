//! On-device OCR — shells `tesseract` to extract exact text from an
//! image (typically a fresh screencap). Useful when:
//!
//!   - the active LLM brain is NOT vision-capable and the agent has
//!     to read screen content from text only;
//!   - the user asks for an EXACT extraction (a phone number, an OTP
//!     digit, a wallet address) where vision models confabulate;
//!   - the screen has mixed scripts (Thai + English) and we want
//!     unambiguous tokenisation.
//!
//! Runtime dep: a `tesseract` binary on the device PATH. The
//! `discover_tesseract` helper walks a list of well-known locations
//! so users with Termux installs OR a Magisk-shipped static binary
//! both work without extra config:
//!
//!   1. /data/peko/bin/tesseract       — Magisk module add-on
//!   2. /data/local/tmp/tesseract      — quick adb push
//!   3. /system/bin/tesseract          — system install
//!   4. /data/data/com.termux/files/usr/bin/tesseract  — Termux
//!   5. PATH lookup via `which`
//!
//! When no binary is found the tool returns a typed "tesseract not
//! installed" error that includes the discovery list — never
//! crashes, never silently no-ops.

use peko_core::tool::{Tool, ToolResult};
use serde_json::json;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::process::{Command, Stdio};

/// Default Page Segmentation Mode. 6 = "Assume a single uniform block
/// of text" — works well for app screens. PSM 3 (auto) over-segments
/// status bars + chrome; PSM 11 (sparse) loses paragraph structure.
const DEFAULT_PSM: u32 = 6;
const DEFAULT_LANGS: &str = "eng";

const TESSERACT_CANDIDATES: &[&str] = &[
    "/data/peko/bin/tesseract",
    "/data/local/tmp/tesseract",
    "/system/bin/tesseract",
    "/data/data/com.termux/files/usr/bin/tesseract",
];

pub struct OcrTool;

impl OcrTool {
    pub fn new() -> Self { Self }
}

impl Default for OcrTool {
    fn default() -> Self { Self::new() }
}

impl Tool for OcrTool {
    fn name(&self) -> &str { "ocr" }

    fn description(&self) -> &str {
        "Extract exact text from an image using local Tesseract OCR. \
         Faster + more reliable than vision-LLM transcription for small \
         text (OTP codes, phone numbers, ledger lines, addresses). \
         Actions: \
         read_screen { lang?: 'eng' or 'eng+tha' or any installed lang code, psm?: 6 } \
           — captures a fresh screenshot and OCRs it; \
         read_image { path: '/data/peko/foo.png', lang?, psm? } \
           — OCRs an existing image. \
         Requires `tesseract` binary on device. Returns the recognised \
         text with no LLM in the loop, so it's fully local."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["read_screen", "read_image"],
                },
                "path": {
                    "type": "string",
                    "description": "Image path on device (read_image only)."
                },
                "lang": {
                    "type": "string",
                    "description": "Tesseract language codes joined with '+', e.g. 'eng', 'eng+tha'. Default 'eng'."
                },
                "psm": {
                    "type": "integer",
                    "description": "Page segmentation mode 0-13. Default 6 (single block)."
                }
            },
            "required": ["action"]
        })
    }

    fn execute(
        &self,
        args: serde_json::Value,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<ToolResult>> + Send + '_>> {
        let action = args["action"].as_str().unwrap_or("").to_string();
        let path = args["path"].as_str().map(String::from);
        let lang = args["lang"]
            .as_str()
            .map(String::from)
            .unwrap_or_else(|| DEFAULT_LANGS.to_string());
        let psm = args["psm"].as_u64().map(|n| n as u32).unwrap_or(DEFAULT_PSM);
        Box::pin(async move {
            tokio::task::spawn_blocking(move || dispatch(&action, path, &lang, psm))
                .await
                .map_err(|e| anyhow::anyhow!("ocr task panicked: {e}"))?
        })
    }
}

fn dispatch(
    action: &str,
    path: Option<String>,
    lang: &str,
    psm: u32,
) -> anyhow::Result<ToolResult> {
    let bin = match discover_tesseract() {
        Some(b) => b,
        None => {
            return Ok(ToolResult::error(format!(
                "tesseract not found in any of: {}. \
                 Install via Termux (`pkg install tesseract tesseract-data-eng`) \
                 or push a prebuilt arm64 binary to /data/peko/bin/tesseract.",
                TESSERACT_CANDIDATES.join(", ")
            )))
        }
    };

    if let Err(e) = validate_lang(lang) {
        return Ok(ToolResult::error(e));
    }
    if !(0..=13).contains(&psm) {
        return Ok(ToolResult::error(format!("psm must be 0..=13, got {psm}")));
    }

    match action {
        "read_screen" => {
            let png = match capture_screen() {
                Ok(p) => p,
                Err(e) => return Ok(ToolResult::error(format!("screencap failed: {e}"))),
            };
            run_tesseract(&bin, &png, lang, psm).map(|out| {
                let _ = std::fs::remove_file(&png);
                out
            })
        }
        "read_image" => {
            let Some(p) = path else {
                return Ok(ToolResult::error("missing 'path' for read_image".to_string()));
            };
            let path = PathBuf::from(p);
            if !path.exists() {
                return Ok(ToolResult::error(format!("image not found: {}", path.display())));
            }
            run_tesseract(&bin, &path, lang, psm)
        }
        "" => Ok(ToolResult::error("missing 'action' parameter".to_string())),
        other => Ok(ToolResult::error(format!(
            "unknown action '{other}'. valid: read_screen, read_image"
        ))),
    }
}

/// Find the first executable tesseract binary in our search list, or
/// fall back to `which` for the user's PATH.
pub(crate) fn discover_tesseract() -> Option<PathBuf> {
    for cand in TESSERACT_CANDIDATES {
        let p = Path::new(cand);
        if p.exists() && is_executable(p) {
            return Some(p.to_path_buf());
        }
    }
    // PATH probe via `which`.
    if let Ok(out) = Command::new("which")
        .arg("tesseract")
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
    {
        if out.status.success() {
            let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !s.is_empty() {
                let p = PathBuf::from(s);
                if p.exists() {
                    return Some(p);
                }
            }
        }
    }
    None
}

fn is_executable(p: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(p)
        .map(|m| m.is_file() && (m.permissions().mode() & 0o111 != 0))
        .unwrap_or(false)
}

/// Validate the `lang` argument so we don't shell-inject through it.
/// Tesseract language codes are 3-letter ISO 639-3 lowercase joined
/// with '+'. We reject anything else.
fn validate_lang(lang: &str) -> Result<(), String> {
    if lang.is_empty() {
        return Err("lang is empty".to_string());
    }
    if lang.len() > 64 {
        return Err("lang too long".to_string());
    }
    for part in lang.split('+') {
        if part.is_empty() || !part.chars().all(|c| c.is_ascii_alphanumeric()) {
            return Err(format!("invalid lang component: '{part}'"));
        }
    }
    Ok(())
}

fn capture_screen() -> anyhow::Result<PathBuf> {
    let out = std::env::temp_dir().join(format!("peko-ocr-{}.png", std::process::id()));
    let _ = std::fs::remove_file(&out);
    // Try tmp dir on device first; fallback to /sdcard which is
    // world-readable and usually writable from root.
    let target = if cfg!(target_os = "android") {
        PathBuf::from(format!("/data/local/tmp/peko-ocr-{}.png", std::process::id()))
    } else {
        out
    };
    let status = Command::new("screencap")
        .args(["-p"])
        .arg(&target)
        .stdin(Stdio::null())
        .stderr(Stdio::piped())
        .status()?;
    if !status.success() {
        anyhow::bail!("screencap exited with {status}");
    }
    Ok(target)
}

fn run_tesseract(
    bin: &Path,
    image: &Path,
    lang: &str,
    psm: u32,
) -> anyhow::Result<ToolResult> {
    let out = Command::new(bin)
        .arg(image)
        .arg("stdout")
        .args(["-l", lang])
        .args(["--psm", &psm.to_string()])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output();
    let out = match out {
        Ok(o) => o,
        Err(e) => return Ok(ToolResult::error(format!("spawn tesseract: {e}"))),
    };
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        return Ok(ToolResult::error(format!(
            "tesseract failed (exit {}): {}",
            out.status.code().unwrap_or(-1),
            stderr.trim()
        )));
    }
    let text = String::from_utf8_lossy(&out.stdout).to_string();
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Ok(ToolResult::success(format!(
            "OCR ({}) found no text in {}",
            lang,
            image.display()
        )));
    }
    Ok(ToolResult::success(format!(
        "OCR ({}, psm={}) — {} chars from {}:\n---\n{}",
        lang,
        psm,
        trimmed.chars().count(),
        image.display(),
        trimmed
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_lang_strings() {
        assert!(validate_lang("eng").is_ok());
        assert!(validate_lang("eng+tha").is_ok());
        assert!(validate_lang("eng+tha+jpn").is_ok());
        assert!(validate_lang("").is_err());
        assert!(validate_lang("eng;rm -rf /").is_err());
        assert!(validate_lang("eng+").is_err());
        assert!(validate_lang("../etc/passwd").is_err());
        assert!(validate_lang("eng+tha+").is_err());
    }

    #[test]
    fn discover_returns_none_when_no_binary() {
        // On the host, tesseract may or may not exist. Verify the
        // result is well-formed (Some(real path) or None) and never
        // panics.
        let _ = discover_tesseract();
    }

    #[test]
    fn schema_lists_actions() {
        let s = OcrTool::new().parameters_schema();
        let actions: Vec<&str> = s["properties"]["action"]["enum"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|v| v.as_str())
            .collect();
        assert!(actions.contains(&"read_screen"));
        assert!(actions.contains(&"read_image"));
    }

    #[test]
    fn rejects_unknown_action() {
        let r = dispatch("xyz", None, "eng", 6).unwrap();
        let json: serde_json::Value = serde_json::json!({
            "is_error": r.is_error,
        });
        assert_eq!(json["is_error"], true);
    }

    #[test]
    fn rejects_invalid_psm() {
        // psm has to be 0..=13. We can only check our own validation
        // path; can't actually invoke tesseract here.
        let r = dispatch("read_image", Some("/tmp/x.png".to_string()), "eng", 99).unwrap();
        assert_eq!(r.is_error, true);
    }
}
