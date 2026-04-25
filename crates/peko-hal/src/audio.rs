//! Audio inspection + mixer control without the Java framework.
//!
//! Three things this module exposes:
//!
//!   1. ALSA topology — list `/proc/asound/cards` + `/proc/asound/devices`
//!      so the agent can see what sound hardware exists.
//!   2. tinymix — `/system/bin/tinymix` is present on every Android with
//!      audio. We shell to it for getting/setting kernel mixer controls
//!      (volume, mute, routing). Works in Lane A; in Lane B a stripped
//!      sysroot we fall back to alsactl-style `/proc/asound/card*/pcm*`
//!      reads for info only.
//!   3. Media volume — `cmd audio` / `media volume` for the
//!      framework-mediated stream volumes (music/ring/notification).
//!      Lane B only.
//!
//! PCM capture and playback are intentionally NOT here — Phase 5 ships
//! an AudioRecord/AudioTrack-backed shim service in the overlay APK
//! that's both simpler and more reliable than re-implementing ALSA's
//! ioctl set in Rust. Rolling our own would duplicate ~500 LOC of
//! tinyalsa with the same caveats.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

#[derive(Debug, thiserror::Error)]
pub enum AudioError {
    #[error("/proc/asound not present — kernel without ALSA support")]
    NoAlsa,
    #[error("tinymix not on PATH; mixer control unavailable")]
    NoTinymix,
    #[error("io error reading {path}: {err}")]
    Io {
        path: PathBuf,
        #[source]
        err: std::io::Error,
    },
    #[error("tinymix exited non-zero: {0}")]
    TinymixFailed(String),
    #[error("control '{name}' not found")]
    ControlNotFound { name: String },
    #[error("media command failed: {0}")]
    MediaFailed(String),
}

#[derive(Debug, Clone, Default)]
pub struct AlsaTopology {
    pub cards: Vec<AlsaCard>,
    pub pcm_capture: Vec<AlsaPcmDevice>,
    pub pcm_playback: Vec<AlsaPcmDevice>,
}

#[derive(Debug, Clone, Default)]
pub struct AlsaCard {
    pub index: u32,
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, Default)]
pub struct AlsaPcmDevice {
    pub card: u32,
    pub device: u32,
    pub direction: PcmDirection,
    pub node: PathBuf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PcmDirection {
    #[default]
    Playback,
    Capture,
}

pub fn topology() -> Result<AlsaTopology, AudioError> {
    let asound = Path::new("/proc/asound");
    if !asound.exists() {
        return Err(AudioError::NoAlsa);
    }
    let cards_text = fs::read_to_string(asound.join("cards")).map_err(|err| AudioError::Io {
        path: asound.join("cards"),
        err,
    })?;
    let cards = parse_alsa_cards(&cards_text);

    let mut top = AlsaTopology { cards, ..Default::default() };

    // Walk /dev/snd for pcmC*D*{c,p} nodes — same as what we saw on the
    // OnePlus 6T probe. Keep a `c` (capture) / `p` (playback) suffix
    // distinction.
    let dev_snd = Path::new("/dev/snd");
    if let Ok(entries) = fs::read_dir(dev_snd) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if let Some(dev) = parse_pcm_device_name(&name, dev_snd) {
                match dev.direction {
                    PcmDirection::Playback => top.pcm_playback.push(dev),
                    PcmDirection::Capture => top.pcm_capture.push(dev),
                }
            }
        }
    }
    top.pcm_playback.sort_by_key(|d| (d.card, d.device));
    top.pcm_capture.sort_by_key(|d| (d.card, d.device));

    Ok(top)
}

fn parse_alsa_cards(text: &str) -> Vec<AlsaCard> {
    // Format:
    //   0 [Card0          ]: Hardware - Card0 Description
    //                        Long-form description on next line
    let mut out = Vec::new();
    for line in text.lines() {
        let trimmed = line.trim_start();
        // Skip continuation lines (no number prefix).
        if !trimmed.starts_with(|c: char| c.is_ascii_digit()) {
            continue;
        }
        let Some((idx_str, rest)) = trimmed.split_once(' ') else { continue };
        let Ok(index) = idx_str.parse::<u32>() else { continue };
        let Some(open) = rest.find('[') else { continue };
        let Some(close) = rest[open + 1..].find(']') else { continue };
        let id = rest[open + 1..open + 1 + close].trim().to_string();
        let after = rest[open + 1 + close + 1..].trim_start_matches(':').trim().to_string();
        out.push(AlsaCard { index, id, name: after });
    }
    out
}

fn parse_pcm_device_name(name: &str, base: &Path) -> Option<AlsaPcmDevice> {
    // pcmC<card>D<device><direction> — direction in {p, c}
    let body = name.strip_prefix("pcmC")?;
    let (card_str, rest) = body.split_once('D')?;
    let card = card_str.parse().ok()?;
    let direction = match rest.chars().last()? {
        'p' => PcmDirection::Playback,
        'c' => PcmDirection::Capture,
        _ => return None,
    };
    let device_str = &rest[..rest.len() - 1];
    let device = device_str.parse().ok()?;
    Some(AlsaPcmDevice {
        card,
        device,
        direction,
        node: base.join(name),
    })
}

// -----------------------------------------------------------------------------
// Mixer controls (tinymix)
// -----------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct MixerControl {
    pub id: u32,
    pub name: String,
    pub kind: String,
    pub value: String,
}

pub fn tinymix_available() -> bool {
    Command::new("which")
        .arg("tinymix")
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// List every mixer control. tinymix output is space-aligned:
///   "Number of controls: N"
///   "ctl     type    num  name                            value"
///   "0       INT     1    Master Playback Volume          -2000 0"
pub fn mixer_list() -> Result<Vec<MixerControl>, AudioError> {
    if !tinymix_available() {
        return Err(AudioError::NoTinymix);
    }
    let out = Command::new("tinymix")
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .map_err(|e| AudioError::TinymixFailed(e.to_string()))?;
    if !out.status.success() {
        return Err(AudioError::TinymixFailed(
            String::from_utf8_lossy(&out.stderr).to_string(),
        ));
    }
    let text = String::from_utf8_lossy(&out.stdout).to_string();
    Ok(parse_tinymix_list(&text))
}

pub fn mixer_get(name: &str) -> Result<String, AudioError> {
    if !tinymix_available() {
        return Err(AudioError::NoTinymix);
    }
    let out = Command::new("tinymix")
        .args(["get", name])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .map_err(|e| AudioError::TinymixFailed(e.to_string()))?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr).to_string();
        if stderr.to_lowercase().contains("not found") {
            return Err(AudioError::ControlNotFound { name: name.into() });
        }
        return Err(AudioError::TinymixFailed(stderr));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

pub fn mixer_set(name: &str, value: &str) -> Result<(), AudioError> {
    if !tinymix_available() {
        return Err(AudioError::NoTinymix);
    }
    let out = Command::new("tinymix")
        .args(["set", name, value])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .map_err(|e| AudioError::TinymixFailed(e.to_string()))?;
    if !out.status.success() {
        return Err(AudioError::TinymixFailed(
            String::from_utf8_lossy(&out.stderr).to_string(),
        ));
    }
    Ok(())
}

fn parse_tinymix_list(text: &str) -> Vec<MixerControl> {
    let mut out = Vec::new();
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty()
            || trimmed.starts_with("Number of controls")
            || trimmed.starts_with("ctl")
        {
            continue;
        }
        // tinymix uses runs of spaces, not tabs. Collapse runs by
        // tokenising up to the first 3 fields manually.
        let toks: Vec<&str> = trimmed.split_whitespace().collect();
        if toks.len() < 4 {
            continue;
        }
        let Ok(id) = toks[0].parse::<u32>() else { continue };
        let kind = toks[1].to_string();
        // toks[2] is num — ignored, we don't expose it.
        // The remainder is "<name with spaces> <value with spaces>".
        // Reconstruct by re-finding where we are in the original line.
        let after_first_three = trimmed
            .split_once(char::is_whitespace)
            .and_then(|(_, r)| r.trim_start().split_once(char::is_whitespace))
            .and_then(|(_, r)| r.trim_start().split_once(char::is_whitespace))
            .map(|(_, r)| r.trim_start().to_string())
            .unwrap_or_default();
        let rest = after_first_three.as_str();
        // Heuristic: split off the value as the last whitespace-delimited
        // run that starts with a digit, sign, or "On"/"Off".
        let (name, value) = split_tinymix_name_value(rest);
        out.push(MixerControl {
            id,
            name,
            kind,
            value,
        });
    }
    out
}

fn split_tinymix_name_value(rest: &str) -> (String, String) {
    // We look from the right: take tokens while they look like a value
    // (numeric or one of On/Off). When we hit a non-value token we stop —
    // everything to its right is the value.
    let tokens: Vec<&str> = rest.split_whitespace().collect();
    let mut split_at = tokens.len();
    for (i, t) in tokens.iter().enumerate().rev() {
        if looks_like_value_token(t) {
            split_at = i;
        } else {
            break;
        }
    }
    let name = tokens[..split_at].join(" ");
    let value = tokens[split_at..].join(" ");
    (name, value)
}

fn looks_like_value_token(t: &str) -> bool {
    if t == "On" || t == "Off" || t == ">" || t == "%" {
        return true;
    }
    let stripped = t.trim_end_matches('%').trim_end_matches(',');
    stripped.parse::<i64>().is_ok() || stripped.parse::<f64>().is_ok()
}

// -----------------------------------------------------------------------------
// Media volume (Lane B convenience)
// -----------------------------------------------------------------------------

pub fn media_volume_get(stream: &str) -> Result<i32, AudioError> {
    let out = Command::new("cmd")
        .args(["audio", "get-volume", stream])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .map_err(|e| AudioError::MediaFailed(e.to_string()))?;
    if !out.status.success() {
        // Fallback to `media volume --get` on older builds.
        let alt = Command::new("media")
            .args(["volume", "--stream", stream, "--get"])
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output()
            .map_err(|e| AudioError::MediaFailed(e.to_string()))?;
        if !alt.status.success() {
            return Err(AudioError::MediaFailed(
                String::from_utf8_lossy(&alt.stderr).to_string(),
            ));
        }
        let txt = String::from_utf8_lossy(&alt.stdout).to_string();
        return parse_volume_int(&txt);
    }
    parse_volume_int(&String::from_utf8_lossy(&out.stdout))
}

fn parse_volume_int(text: &str) -> Result<i32, AudioError> {
    let trimmed = text.trim();
    // Accepts plain "5" or "volume is 5" or "[5]" — extract first int.
    for tok in trimmed.split(|c: char| !c.is_ascii_digit() && c != '-') {
        if !tok.is_empty() {
            if let Ok(n) = tok.parse::<i32>() {
                return Ok(n);
            }
        }
    }
    Err(AudioError::MediaFailed(format!(
        "no integer in volume output: {trimmed}"
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_proc_asound_cards() {
        let text = " 0 [SDM845X         ]: SDM845X - SDM845X\n                      SDM845X-snd-card\n";
        let cards = parse_alsa_cards(text);
        assert_eq!(cards.len(), 1);
        assert_eq!(cards[0].index, 0);
        assert_eq!(cards[0].id, "SDM845X");
        assert!(cards[0].name.contains("SDM845X"));
    }

    #[test]
    fn parses_pcm_device_names_correctly() {
        let base = Path::new("/dev/snd");
        let p = parse_pcm_device_name("pcmC0D0p", base).unwrap();
        assert_eq!(p.card, 0);
        assert_eq!(p.device, 0);
        assert_eq!(p.direction, PcmDirection::Playback);
        assert_eq!(p.node, base.join("pcmC0D0p"));

        let c = parse_pcm_device_name("pcmC0D33c", base).unwrap();
        assert_eq!(c.card, 0);
        assert_eq!(c.device, 33);
        assert_eq!(c.direction, PcmDirection::Capture);

        assert!(parse_pcm_device_name("controlC0", base).is_none());
        assert!(parse_pcm_device_name("hwC0D10", base).is_none());
    }

    #[test]
    fn parses_tinymix_list_basic() {
        let text = "Number of controls: 3
ctl     type    num     name                                            value
0       INT     1       Master Playback Volume                          -2000
1       BOOL    1       Master Playback Switch                          On
2       INT     2       Headphone Playback Volume                       -1500 -1500";
        let controls = parse_tinymix_list(text);
        assert_eq!(controls.len(), 3);
        assert_eq!(controls[0].id, 0);
        assert_eq!(controls[0].kind, "INT");
        assert_eq!(controls[0].name, "Master Playback Volume");
        assert_eq!(controls[0].value, "-2000");
        assert_eq!(controls[1].name, "Master Playback Switch");
        assert_eq!(controls[1].value, "On");
        assert_eq!(controls[2].name, "Headphone Playback Volume");
        assert_eq!(controls[2].value, "-1500 -1500");
    }

    #[test]
    fn split_tinymix_handles_value_with_units() {
        let (name, value) = split_tinymix_name_value("DEC1 Volume -84");
        assert_eq!(name, "DEC1 Volume");
        assert_eq!(value, "-84");

        let (name, value) = split_tinymix_name_value("Master Switch On");
        assert_eq!(name, "Master Switch");
        assert_eq!(value, "On");

        let (name, value) = split_tinymix_name_value("Stereo Volume -1500 -1500");
        assert_eq!(name, "Stereo Volume");
        assert_eq!(value, "-1500 -1500");
    }

    #[test]
    fn parse_volume_int_extracts_number_from_various_formats() {
        assert_eq!(parse_volume_int("5\n").unwrap(), 5);
        assert_eq!(parse_volume_int("volume is 7").unwrap(), 7);
        assert_eq!(parse_volume_int("Stream music: [3]").unwrap(), 3);
    }

    #[test]
    fn topology_returns_no_alsa_on_host() {
        // /proc/asound is Linux-only; this should fail gracefully on macOS.
        let r = topology();
        match r {
            Err(AudioError::NoAlsa) => {}
            Ok(_) => {} // CI Linux runners may have ALSA; that's also fine.
            Err(other) => panic!("unexpected error: {other:?}"),
        }
    }
}
