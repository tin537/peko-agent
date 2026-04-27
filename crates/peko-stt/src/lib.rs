//! Phase 25 — offline speech-to-text.
//!
//! Architecture: `peko-stt` shells out to `whisper-cli` (the standard
//! whisper.cpp CLI binary) installed on the device. The CLI's `-oj` flag
//! produces a JSON document we parse into [`Transcript`].
//!
//! Why shell-out instead of in-process FFI: the `whisper-rs`/`cmake-rs`
//! NDK cross-compile path tripped over CMake's `ANDROID_ABI` discovery
//! and produced 32-bit ARM objects that failed to link against our
//! aarch64 binary. The CLI route uses the SAME proven CMake recipe
//! that already builds `peko-llm-daemon` (both are ggml-based), runs
//! exactly once per call (~2s model-load overhead is acceptable for
//! voice-loop UX), and is bulletproof against build-system drift.
//!
//! For sub-second latency in the future we can swap in a long-running
//! daemon (mirror peko-llm-daemon's UDS pattern) without changing the
//! agent-facing API.

use anyhow::{anyhow, Context};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use tokio::process::Command;

/// Default model + binary paths. Both shipped via the Magisk module's
/// /system/bin and /data/peko/models/. Overridable via `Engine::open`.
pub const DEFAULT_MODEL_PATH: &str = "/data/peko/models/whisper.bin";
pub const DEFAULT_BIN_PATH: &str = "/system/bin/whisper-cli";

/// Search order for whisper-cli: the Magisk-bound /system/bin path
/// (works after reboot once the module is installed), the live Magisk
/// module dir (works pre-reboot since root can read it), and a
/// dev-friendly /data/local/tmp staging copy. First-existing wins.
pub const BIN_SEARCH_PATHS: &[&str] = &[
    "/system/bin/whisper-cli",
    "/data/adb/modules/peko_agent/system/bin/whisper-cli",
    "/data/local/tmp/whisper-cli",
];

/// Resolve whisper-cli on the device by walking [`BIN_SEARCH_PATHS`].
/// Returns the first existing path. Used by [`Engine::open`] when the
/// caller doesn't pin a specific binary location.
pub fn discover_bin() -> Option<PathBuf> {
    for p in BIN_SEARCH_PATHS {
        let pb = PathBuf::from(p);
        if pb.exists() {
            return Some(pb);
        }
    }
    None
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Segment {
    /// Inclusive start time, milliseconds.
    pub t0_ms: i64,
    /// Inclusive end time, milliseconds.
    pub t1_ms: i64,
    pub text: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Transcript {
    pub text: String,
    pub language: String,
    pub duration_ms: u64,
    pub segments: Vec<Segment>,
    pub model_path: String,
}

#[derive(Debug, Clone)]
pub struct TranscribeOpts {
    /// "auto" → whisper detects (default); or "th", "en", "ja", etc.
    pub language: String,
    /// Force translation TO English. Useful when the user speaks Thai
    /// and wants the model to think in English.
    pub translate: bool,
    /// CPU threads. OnePlus 6T A75 cluster has 4 cores; default to 4.
    pub threads: usize,
    /// Bias whisper's vocabulary; helpful for proper nouns / jargon.
    pub initial_prompt: Option<String>,
    /// Hard wall-clock cap in seconds. The CLI will be killed if it
    /// runs over. Default 60s — generous for clips up to ~2 minutes.
    pub timeout_secs: u64,
}

impl Default for TranscribeOpts {
    fn default() -> Self {
        Self {
            language: "auto".to_string(),
            translate: false,
            threads: 4,
            initial_prompt: None,
            timeout_secs: 60,
        }
    }
}

#[derive(Clone)]
pub struct Engine {
    model_path: PathBuf,
    bin_path: PathBuf,
}

impl Engine {
    /// Open with custom paths (or defaults when None). When `bin_path`
    /// is None, walks [`BIN_SEARCH_PATHS`] to find a working whisper-cli
    /// — convenient for dev (binary in /data/local/tmp) and prod
    /// (Magisk-bound /system/bin) without code changes.
    pub fn open(model_path: Option<&Path>, bin_path: Option<&Path>) -> anyhow::Result<Self> {
        let model = model_path.map(Path::to_path_buf).unwrap_or_else(|| PathBuf::from(DEFAULT_MODEL_PATH));
        let bin = match bin_path {
            Some(b) => b.to_path_buf(),
            None => discover_bin().ok_or_else(|| anyhow!(
                "whisper-cli not found in any of {:?}. Build it via \
                 `cd crates/peko-stt-bin && ./build-android.sh`, then \
                 push to /data/local/tmp/whisper-cli (immediate use) or \
                 the Magisk module's system/bin/ (persists across boots).",
                BIN_SEARCH_PATHS,
            ))?,
        };
        if !bin.exists() {
            return Err(anyhow!(
                "whisper-cli binary not found at {}. Build it via \
                 `cd crates/peko-stt-bin && ./build-android.sh`, then \
                 push to /system/bin/whisper-cli (Magisk module copies \
                 it on next reboot).",
                bin.display(),
            ));
        }
        if !model.exists() {
            return Err(anyhow!(
                "whisper model not found at {}. Push one via \
                 `scripts/download-whisper-model.sh` (default: \
                 ggml-base.bin, ~150 MB, multilingual).",
                model.display(),
            ));
        }
        Ok(Self { model_path: model, bin_path: bin })
    }

    pub fn model_path(&self) -> &Path { &self.model_path }
    pub fn bin_path(&self) -> &Path { &self.bin_path }

    /// Transcribe a 16-bit PCM WAV file. whisper-cli accepts mono /
    /// stereo / any common sample rate and resamples internally.
    pub async fn transcribe(&self, wav_path: &Path, opts: &TranscribeOpts) -> anyhow::Result<Transcript> {
        if !wav_path.exists() {
            return Err(anyhow!("WAV file missing: {}", wav_path.display()));
        }
        // -oj writes a JSON file alongside the input WAV — note the
        // upstream behaviour APPENDS ".json" rather than replacing the
        // extension, so foo.wav → foo.wav.json. We harvest from there
        // rather than pipe stdout (which is its progress log + coloured
        // text mixed with model warnings).
        let mut json_out_os = wav_path.as_os_str().to_os_string();
        json_out_os.push(".json");
        let json_out = PathBuf::from(json_out_os);
        let _ = tokio::fs::remove_file(&json_out).await;

        let mut cmd = Command::new(&self.bin_path);
        cmd.arg("-m").arg(&self.model_path)
            .arg("-f").arg(wav_path)
            .arg("-l").arg(&opts.language)
            .arg("-t").arg(opts.threads.to_string())
            .arg("-oj")          // emit <stem>.json with segments
            .arg("-np")          // no progress bar (would clutter stderr)
            .arg("-nt")          // no per-segment timestamps in stdout
            .arg("-pp")          // print summary
            ;
        if opts.translate { cmd.arg("-tr"); }
        if let Some(ref ip) = opts.initial_prompt {
            cmd.arg("--prompt").arg(ip);
        }
        cmd.stdout(Stdio::piped()).stderr(Stdio::piped()).kill_on_drop(true);

        let started = std::time::Instant::now();
        let run = cmd.output();
        let output = tokio::time::timeout(std::time::Duration::from_secs(opts.timeout_secs), run)
            .await
            .map_err(|_| anyhow!(
                "whisper-cli timed out after {}s. Try a smaller model \
                 (push ggml-tiny.bin) or shorter clip.", opts.timeout_secs))?
            .with_context(|| format!("spawning {}", self.bin_path.display()))?;
        let elapsed_ms = started.elapsed().as_millis() as u64;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(anyhow!(
                "whisper-cli failed (exit {:?}): {}",
                output.status.code(),
                stderr.lines().rev().take(5).collect::<Vec<_>>().join(" / ")
            ));
        }

        // Parse the -oj JSON file. Schema documented at
        // https://github.com/ggml-org/whisper.cpp `examples/cli/cli.cpp`
        // — top-level has { transcription: [{timestamps:{from,to}, text}] }
        // and the language-detect output goes to stderr.
        let raw = tokio::fs::read_to_string(&json_out).await
            .with_context(|| format!("reading whisper output {}", json_out.display()))?;
        let _ = tokio::fs::remove_file(&json_out).await;
        let parsed: WhisperOutput = serde_json::from_str(&raw)
            .with_context(|| "parsing whisper-cli json")?;

        let detected_lang = parsed.result.as_ref()
            .and_then(|r| if r.language.is_empty() { None } else { Some(r.language.clone()) });
        let segments: Vec<Segment> = parsed.transcription.into_iter().map(|s| Segment {
            t0_ms: parse_ts(&s.timestamps.from),
            t1_ms: parse_ts(&s.timestamps.to),
            text: s.text.trim().to_string(),
        }).collect();
        let text: String = segments.iter().map(|s| s.text.as_str()).collect::<Vec<_>>().join(" ");

        // Prefer the language reported in the JSON `result.language`
        // field (set by whisper-cli regardless of -l auto vs explicit).
        // Falls back to stderr scrape, then to whatever the caller asked for.
        let language = detected_lang
            .or_else(|| {
                let stderr = String::from_utf8_lossy(&output.stderr);
                detect_language_from_stderr(&stderr)
            })
            .unwrap_or_else(|| {
                if opts.language == "auto" || opts.language.is_empty() { "auto".into() }
                else { opts.language.clone() }
            });

        Ok(Transcript {
            text: text.trim().to_string(),
            language,
            duration_ms: elapsed_ms,
            segments,
            model_path: self.model_path.display().to_string(),
        })
    }
}

#[derive(Deserialize)]
struct WhisperOutput {
    transcription: Vec<WhisperSegment>,
    #[serde(default)]
    result: Option<WhisperResult>,
}

#[derive(Deserialize)]
struct WhisperResult {
    #[serde(default)]
    language: String,
}

#[derive(Deserialize)]
struct WhisperSegment {
    timestamps: WhisperTimestamps,
    text: String,
}

#[derive(Deserialize)]
struct WhisperTimestamps {
    from: String,
    to: String,
}

/// "00:00:01,250" → 1250 ms.
fn parse_ts(s: &str) -> i64 {
    // Accept HH:MM:SS,mmm or HH:MM:SS.mmm
    let s = s.replace(',', ".");
    let parts: Vec<&str> = s.split(':').collect();
    if parts.len() != 3 { return 0; }
    let h: i64 = parts[0].parse().unwrap_or(0);
    let m: i64 = parts[1].parse().unwrap_or(0);
    let sec_ms: f64 = parts[2].parse().unwrap_or(0.0);
    h * 3_600_000 + m * 60_000 + (sec_ms * 1000.0) as i64
}

fn detect_language_from_stderr(stderr: &str) -> Option<String> {
    for line in stderr.lines() {
        if let Some(rest) = line.strip_prefix("auto-detected language:") {
            return Some(rest.split_whitespace().next()?.to_string());
        }
        if let Some(rest) = line.find("language: ").map(|i| &line[i + 10..]) {
            // Some whisper-cli versions print "language: th" — pick code only.
            let code = rest.split_whitespace().next()?;
            if code.len() <= 6 { return Some(code.to_string()); }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_ts_handles_both_formats() {
        assert_eq!(parse_ts("00:00:01,250"), 1250);
        assert_eq!(parse_ts("00:01:30.000"), 90_000);
        assert_eq!(parse_ts("01:00:00,000"), 3_600_000);
    }

    #[test]
    fn detect_language_picks_code() {
        let s = "whisper_init_from_file_with_params_no_state: loading model\nauto-detected language: th (probability 0.984512)\n";
        assert_eq!(detect_language_from_stderr(s).as_deref(), Some("th"));
    }

    #[test]
    fn engine_open_errors_when_bin_missing() {
        let result = Engine::open(
            Some(Path::new("/nonexistent/model.bin")),
            Some(Path::new("/nonexistent/whisper-cli")),
        );
        assert!(result.is_err());
        let msg = result.err().unwrap().to_string();
        assert!(msg.contains("whisper-cli binary not found"));
    }
}
