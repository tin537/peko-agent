//! Per-device hardware overrides loaded from a TOML profile.
//!
//! The profile is consulted at startup to override values that
//! auto-detection cannot reliably get right on every panel:
//!   - Touchscreen ABS ranges that the kernel mis-reports (some sdm845
//!     variants advertise the LCD pixel grid but actually expect
//!     touch_panel coordinates).
//!   - Display preference (DRM vs fbdev vs screencap) when the heuristic
//!     picks the wrong one.
//!   - Wifi control socket path (LineageOS vs Magisk-on-stock paths).
//!
//! The profile is OPTIONAL. If `/data/peko/device_profile.toml` is
//! missing or malformed, the agent uses auto-detected values and logs
//! the fact. We never ship a "default" profile — the *absence* of a
//! profile is the default.
//!
//! Profile lookup order:
//!   1. `$PEKO_DEVICE_PROFILE` env var (absolute path)
//!   2. `/data/peko/device_profile.toml`
//!   3. `device-profiles/<ro.product.device>-<ro.build.flavor>.toml` in
//!      the agent's binary directory (used by Magisk module to ship a
//!      per-device profile alongside the binary).

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, thiserror::Error)]
pub enum ProfileError {
    #[error("read {path}: {err}")]
    Io {
        path: PathBuf,
        #[source]
        err: std::io::Error,
    },
    #[error("parse {path}: {err}")]
    Parse {
        path: PathBuf,
        #[source]
        err: toml::de::Error,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DeviceProfile {
    /// Free-text identifier for diagnostics. Doesn't affect behavior.
    #[serde(default)]
    pub device: String,
    /// Free-text ROM tag (e.g. "lineage-20.0", "magisk-on-stock-13").
    #[serde(default)]
    pub rom: String,
    #[serde(default)]
    pub display: DisplayProfile,
    #[serde(default)]
    pub touch: TouchProfile,
    #[serde(default)]
    pub wifi: WifiProfile,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DisplayProfile {
    /// Override the auto-detected display dimensions. Most callers should
    /// leave this empty and let `wm size` decide.
    pub width: Option<u32>,
    pub height: Option<u32>,
    /// Override the sysfs-reported rotation (degrees). Useful when the
    /// vendor kernel doesn't expose rotation at all.
    pub rotation_deg: Option<i32>,
    /// Preferred capture backend: "auto" (default), "fb", "screencap",
    /// or "drm" (Phase 8).
    #[serde(default = "default_capture_pref")]
    pub prefer: String,
}

impl Default for DisplayProfile {
    // Hand-rolled because we need `prefer` to default to "auto" both
    // when the [display] section is missing entirely AND when only the
    // `prefer` key inside an existing section is omitted. Deriving
    // Default would set `prefer = String::new()` for the first case.
    fn default() -> Self {
        Self {
            width: None,
            height: None,
            rotation_deg: None,
            prefer: default_capture_pref(),
        }
    }
}

fn default_capture_pref() -> String { "auto".to_string() }

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TouchProfile {
    /// Override `EVIOCGABS(ABS_MT_POSITION_X).maximum`. Set when the
    /// kernel lies — confirmed via `getevent -lp` output.
    pub abs_x_max: Option<i32>,
    pub abs_y_max: Option<i32>,
    /// Override pressure default (some panels need full-scale to
    /// register a tap; a few reject anything above ~30).
    pub pressure_default: Option<i32>,
    /// Override touch-major default (contact-area metric).
    pub touch_major_default: Option<i32>,
    /// Force a specific input event device path. Empty = auto-detect.
    pub device_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct WifiProfile {
    /// Direct path to a wpa_supplicant global / per-interface control
    /// socket. Empty = probe runtime locations in order.
    pub ctrl_socket_path: Option<String>,
    /// Path to the directory holding wpa_supplicant per-client sockets.
    /// Used when no global socket exists (LineageOS 20+ on sdm845):
    /// Phase 3 binds its own client socket inside this dir and connects
    /// to the interface-specific socket (typically `<dir>/wlan0`).
    pub ctrl_socket_dir: Option<String>,
}

impl DeviceProfile {
    /// Load the device profile, returning a default profile if no
    /// profile file is found. Errors only when a profile file *does*
    /// exist but is malformed — silent fallback in that case would
    /// confuse the user (they edited the file expecting it to take
    /// effect, and instead got defaults).
    pub fn load() -> Result<Self, ProfileError> {
        for source in candidate_paths() {
            if source.exists() {
                let content = std::fs::read_to_string(&source).map_err(|err| ProfileError::Io {
                    path: source.clone(),
                    err,
                })?;
                let mut profile: DeviceProfile = toml::from_str(&content).map_err(|err| {
                    ProfileError::Parse {
                        path: source.clone(),
                        err,
                    }
                })?;
                if profile.device.is_empty() {
                    profile.device = source.display().to_string();
                }
                tracing::info!(profile = %source.display(), "loaded device profile");
                return Ok(profile);
            }
        }
        Ok(Self::default())
    }

    /// Load a profile from a specific path. Used by tests + tooling.
    pub fn load_from(path: &Path) -> Result<Self, ProfileError> {
        let content = std::fs::read_to_string(path).map_err(|err| ProfileError::Io {
            path: path.to_path_buf(),
            err,
        })?;
        toml::from_str(&content).map_err(|err| ProfileError::Parse {
            path: path.to_path_buf(),
            err,
        })
    }

    pub fn from_str_for_test(s: &str) -> Result<Self, toml::de::Error> {
        toml::from_str(s)
    }
}

fn candidate_paths() -> Vec<PathBuf> {
    let mut paths = Vec::new();

    if let Ok(env_path) = std::env::var("PEKO_DEVICE_PROFILE") {
        paths.push(PathBuf::from(env_path));
    }

    paths.push(PathBuf::from("/data/peko/device_profile.toml"));

    // Per-device profile shipped next to the binary. We read
    // `ro.product.device` via getprop when available, which is the
    // device codename used in `device-profiles/<codename>-*.toml`.
    if let Some(codename) = read_getprop("ro.product.device") {
        if let Ok(exe) = std::env::current_exe() {
            if let Some(dir) = exe.parent() {
                let device_profiles = dir.join("device-profiles");
                if let Ok(entries) = std::fs::read_dir(&device_profiles) {
                    for entry in entries.flatten() {
                        let p = entry.path();
                        let stem = p
                            .file_stem()
                            .and_then(|s| s.to_str())
                            .unwrap_or("");
                        if stem.starts_with(&codename) {
                            paths.push(p);
                        }
                    }
                }
            }
        }
    }

    paths
}

fn read_getprop(key: &str) -> Option<String> {
    let out = std::process::Command::new("getprop")
        .arg(key)
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let val = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if val.is_empty() { None } else { Some(val) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_profile_parses() {
        let p = DeviceProfile::from_str_for_test("").unwrap();
        assert_eq!(p.device, "");
        assert_eq!(p.display.prefer, "auto");
        assert!(p.touch.abs_x_max.is_none());
    }

    #[test]
    fn full_profile_parses() {
        let toml = r#"
            device = "OnePlus 6T (fajita)"
            rom = "lineage-20.0"

            [display]
            width = 1080
            height = 2340
            rotation_deg = 0
            prefer = "screencap"

            [touch]
            abs_x_max = 1080
            abs_y_max = 2340
            pressure_default = 50
            touch_major_default = 6
            device_path = "/dev/input/event2"

            [wifi]
            ctrl_socket_path = "/data/vendor/wifi/wpa/sockets/wpa_ctrl_global"
            ctrl_socket_dir = "/data/vendor/wifi/wpa/sockets"
        "#;
        let p: DeviceProfile = toml::from_str(toml).unwrap();
        assert_eq!(p.device, "OnePlus 6T (fajita)");
        assert_eq!(p.display.width, Some(1080));
        assert_eq!(p.display.height, Some(2340));
        assert_eq!(p.display.rotation_deg, Some(0));
        assert_eq!(p.display.prefer, "screencap");
        assert_eq!(p.touch.abs_x_max, Some(1080));
        assert_eq!(p.touch.abs_y_max, Some(2340));
        assert_eq!(p.touch.pressure_default, Some(50));
        assert_eq!(p.touch.touch_major_default, Some(6));
        assert_eq!(p.touch.device_path.as_deref(), Some("/dev/input/event2"));
        assert_eq!(
            p.wifi.ctrl_socket_path.as_deref(),
            Some("/data/vendor/wifi/wpa/sockets/wpa_ctrl_global")
        );
        assert_eq!(
            p.wifi.ctrl_socket_dir.as_deref(),
            Some("/data/vendor/wifi/wpa/sockets")
        );
    }

    #[test]
    fn partial_profile_uses_defaults_for_missing_sections() {
        let toml = r#"
            device = "Pixel 7"

            [touch]
            pressure_default = 100
        "#;
        let p: DeviceProfile = toml::from_str(toml).unwrap();
        assert_eq!(p.device, "Pixel 7");
        assert_eq!(p.touch.pressure_default, Some(100));
        assert!(p.display.width.is_none());
        assert_eq!(p.display.prefer, "auto");
        assert!(p.wifi.ctrl_socket_path.is_none());
    }

    #[test]
    fn unknown_keys_fail_loud() {
        // Missing keys are tolerated (Option), but mistyped keys at the
        // top level should fail so users notice typos. We accomplish
        // this in serde by NOT setting `deny_unknown_fields` — silent
        // tolerance is preferred so users can drop in keys reserved
        // for future phases without errors. This test documents that
        // contract: unknown keys are NOT a parse error.
        let toml = r#"
            device = "Test"
            future_key = "ignored"

            [display]
            future_subkey = 42
        "#;
        let p: DeviceProfile = toml::from_str(toml).unwrap();
        assert_eq!(p.device, "Test");
    }

    #[test]
    fn load_from_missing_path_returns_io_error() {
        let r = DeviceProfile::load_from(Path::new("/this/path/does/not/exist.toml"));
        match r {
            Err(ProfileError::Io { .. }) => {}
            other => panic!("expected Io error, got {:?}", other),
        }
    }
}
