//! Wi-Fi control without going through Java framework APIs.
//!
//! Two backends, picked at runtime:
//!
//!   1. `CmdWifiBackend` — shell-out to `cmd wifi`. Goes through Android
//!      WifiManagerService, but it's a stable userspace command, doesn't
//!      need binder client code, and works on every modern AOSP/LineageOS.
//!      Lane B preferred path.
//!   2. `WpaSupplicantBackend` — speaks the wpa_supplicant control protocol
//!      directly over a UNIX socket at /data/vendor/wifi/wpa/sockets/wlan0.
//!      Lane A path (frameworkless), and the Lane B fallback when
//!      `cmd wifi` is missing. Documents the protocol enough that a
//!      future Lane A binary can drive Wi-Fi without any framework
//!      services running.
//!
//! Both backends return the same typed structs so callers don't care
//! which fired.

use std::os::unix::net::UnixDatagram;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

#[derive(Debug, thiserror::Error)]
pub enum WifiError {
    #[error("no wifi backend available (cmd wifi missing and wpa_supplicant socket not reachable)")]
    NoBackend,
    #[error("cmd wifi failed: {0}")]
    CmdFailed(String),
    #[error("wpa_supplicant ctrl error on {path}: {err}")]
    WpaCtrl {
        path: PathBuf,
        #[source]
        err: std::io::Error,
    },
    #[error("wpa_supplicant returned: {0}")]
    WpaResponse(String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("parse error: {0}")]
    Parse(String),
}

#[derive(Debug, Clone, Default)]
pub struct WifiStatus {
    pub enabled: bool,
    pub connected: bool,
    pub ssid: Option<String>,
    pub bssid: Option<String>,
    pub ip_address: Option<String>,
    pub rssi_dbm: Option<i32>,
    pub link_speed_mbps: Option<u32>,
    pub frequency_mhz: Option<u32>,
    pub security: Option<String>,
    pub net_id: Option<i32>,
}

#[derive(Debug, Clone)]
pub struct ScanResult {
    pub bssid: String,
    pub ssid: String,
    pub frequency_mhz: u32,
    pub rssi_dbm: i32,
    pub flags: String,
}

#[derive(Debug, Clone)]
pub struct SavedNetwork {
    pub id: i32,
    pub ssid: String,
    pub security: String,
}

pub trait WifiBackend: Send {
    fn name(&self) -> &'static str;
    fn status(&self) -> Result<WifiStatus, WifiError>;
    fn scan(&self) -> Result<Vec<ScanResult>, WifiError>;
    fn list_networks(&self) -> Result<Vec<SavedNetwork>, WifiError>;
    fn connect(&self, ssid: &str, password: Option<&str>) -> Result<(), WifiError>;
    fn disconnect(&self) -> Result<(), WifiError>;
    fn set_enabled(&self, enabled: bool) -> Result<(), WifiError>;
}

/// Hints for `auto_backend` — typically populated from `DeviceProfile`
/// at the tool layer so this crate stays independent of peko-config.
#[derive(Debug, Default, Clone)]
pub struct WifiHints {
    /// Direct path to the wpa_supplicant ctrl socket (e.g. `<dir>/wlan0`).
    pub ctrl_socket_path: Option<PathBuf>,
    /// Directory holding wpa_supplicant per-iface sockets — used when
    /// no global path is configured. We try `<dir>/wlan0` first.
    pub ctrl_socket_dir: Option<PathBuf>,
    /// If true, prefer the wpa_supplicant backend even when `cmd wifi`
    /// is available. Lane A simulators set this for testing.
    pub prefer_wpa_supplicant: bool,
}

/// Pick the best available backend honouring hints. Falls through
/// `cmd wifi` → wpa_supplicant.
pub fn auto_backend(hints: &WifiHints) -> Result<Box<dyn WifiBackend>, WifiError> {
    if !hints.prefer_wpa_supplicant && CmdWifiBackend::is_available() {
        return Ok(Box::new(CmdWifiBackend));
    }

    let socket = hints
        .ctrl_socket_path
        .clone()
        .or_else(|| hints.ctrl_socket_dir.as_ref().map(|d| d.join("wlan0")))
        .or_else(|| {
            for p in [
                "/data/vendor/wifi/wpa/sockets/wlan0",
                "/data/misc/wifi/sockets/wpa_ctrl",
                "/data/vendor/wifi/wpa/sockets/wpa_ctrl_global",
            ] {
                if Path::new(p).exists() {
                    return Some(PathBuf::from(p));
                }
            }
            None
        });

    if let Some(path) = socket {
        return Ok(Box::new(WpaSupplicantBackend::open(&path)?));
    }
    if CmdWifiBackend::is_available() {
        return Ok(Box::new(CmdWifiBackend));
    }
    Err(WifiError::NoBackend)
}

// -----------------------------------------------------------------------------
// CmdWifiBackend
// -----------------------------------------------------------------------------

pub struct CmdWifiBackend;

impl CmdWifiBackend {
    pub fn is_available() -> bool {
        Command::new("which")
            .arg("cmd")
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    fn run(args: &[&str]) -> Result<String, WifiError> {
        let out = Command::new("cmd")
            .arg("wifi")
            .args(args)
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output()
            .map_err(|e| WifiError::CmdFailed(e.to_string()))?;
        if !out.status.success() {
            return Err(WifiError::CmdFailed(
                String::from_utf8_lossy(&out.stderr).to_string(),
            ));
        }
        Ok(String::from_utf8_lossy(&out.stdout).to_string())
    }
}

impl WifiBackend for CmdWifiBackend {
    fn name(&self) -> &'static str { "cmd-wifi" }

    fn status(&self) -> Result<WifiStatus, WifiError> {
        let out = Self::run(&["status"])?;
        Ok(parse_cmd_wifi_status(&out))
    }

    fn scan(&self) -> Result<Vec<ScanResult>, WifiError> {
        let _ = Self::run(&["start-scan"]); // best-effort kick
        let out = Self::run(&["list-scan-results"])?;
        Ok(parse_cmd_wifi_scan(&out))
    }

    fn list_networks(&self) -> Result<Vec<SavedNetwork>, WifiError> {
        let out = Self::run(&["list-networks"])?;
        Ok(parse_cmd_wifi_networks(&out))
    }

    fn connect(&self, ssid: &str, password: Option<&str>) -> Result<(), WifiError> {
        // `cmd wifi connect-network <ssid> <auth-type> [password]`
        // auth-types: open, wpa2, wpa3, owe, wep
        let auth = if password.is_some() { "wpa2" } else { "open" };
        let mut args: Vec<String> = vec![
            "connect-network".into(),
            ssid.into(),
            auth.into(),
        ];
        if let Some(pw) = password {
            args.push(pw.into());
        }
        let argv: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
        let out = Self::run(&argv)?;
        if out.to_lowercase().contains("fail") {
            return Err(WifiError::CmdFailed(out));
        }
        Ok(())
    }

    fn disconnect(&self) -> Result<(), WifiError> {
        let _ = Self::run(&["disconnect"])?;
        Ok(())
    }

    fn set_enabled(&self, enabled: bool) -> Result<(), WifiError> {
        let v = if enabled { "enabled" } else { "disabled" };
        let _ = Self::run(&["set-wifi-enabled", v])?;
        Ok(())
    }
}

// -----------------------------------------------------------------------------
// WpaSupplicantBackend
// -----------------------------------------------------------------------------

pub struct WpaSupplicantBackend {
    server: PathBuf,
    client: UnixDatagram,
    client_path: PathBuf,
}

impl WpaSupplicantBackend {
    pub fn open(server_path: &Path) -> Result<Self, WifiError> {
        let pid = std::process::id();
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.subsec_nanos())
            .unwrap_or(0);
        let client_path = std::env::temp_dir().join(format!("peko-wpa-{pid}-{nanos}"));
        let _ = std::fs::remove_file(&client_path);

        let client = UnixDatagram::bind(&client_path).map_err(|err| WifiError::WpaCtrl {
            path: client_path.clone(),
            err,
        })?;
        client
            .set_read_timeout(Some(Duration::from_millis(2000)))
            .map_err(|e| WifiError::Io(e))?;
        client.connect(server_path).map_err(|err| WifiError::WpaCtrl {
            path: server_path.to_path_buf(),
            err,
        })?;

        Ok(Self {
            server: server_path.to_path_buf(),
            client,
            client_path,
        })
    }

    fn request(&self, cmd: &str) -> Result<String, WifiError> {
        self.client.send(cmd.as_bytes()).map_err(|e| WifiError::Io(e))?;
        let mut buf = [0u8; 8192];
        loop {
            let n = self.client.recv(&mut buf).map_err(|e| WifiError::Io(e))?;
            if n == 0 {
                return Err(WifiError::WpaResponse(
                    "empty response from wpa_supplicant".into(),
                ));
            }
            // Skip unsolicited events (lines beginning with '<' or 'CTRL-EVENT-').
            let text = String::from_utf8_lossy(&buf[..n]).to_string();
            if text.starts_with('<') || text.starts_with("CTRL-EVENT-") {
                continue;
            }
            return Ok(text);
        }
    }
}

impl Drop for WpaSupplicantBackend {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.client_path);
    }
}

impl WifiBackend for WpaSupplicantBackend {
    fn name(&self) -> &'static str { "wpa_supplicant" }

    fn status(&self) -> Result<WifiStatus, WifiError> {
        let r = self.request("STATUS")?;
        Ok(parse_wpa_status(&r))
    }

    fn scan(&self) -> Result<Vec<ScanResult>, WifiError> {
        let _ = self.request("SCAN");
        std::thread::sleep(Duration::from_millis(2000));
        let r = self.request("SCAN_RESULTS")?;
        Ok(parse_wpa_scan(&r))
    }

    fn list_networks(&self) -> Result<Vec<SavedNetwork>, WifiError> {
        let r = self.request("LIST_NETWORKS")?;
        Ok(parse_wpa_networks(&r))
    }

    fn connect(&self, ssid: &str, password: Option<&str>) -> Result<(), WifiError> {
        let id_resp = self.request("ADD_NETWORK")?;
        let id: i32 = id_resp.trim().parse().map_err(|_| {
            WifiError::WpaResponse(format!("ADD_NETWORK returned non-integer: {id_resp}"))
        })?;
        // Quote string fields per wpa_cli convention.
        self.request(&format!("SET_NETWORK {} ssid \"{}\"", id, ssid))?;
        match password {
            Some(pw) => {
                self.request(&format!("SET_NETWORK {} psk \"{}\"", id, pw))?;
            }
            None => {
                self.request(&format!("SET_NETWORK {} key_mgmt NONE", id))?;
            }
        }
        self.request(&format!("ENABLE_NETWORK {}", id))?;
        self.request(&format!("SELECT_NETWORK {}", id))?;
        let _ = self.request("SAVE_CONFIG");
        Ok(())
    }

    fn disconnect(&self) -> Result<(), WifiError> {
        self.request("DISCONNECT")?;
        Ok(())
    }

    fn set_enabled(&self, enabled: bool) -> Result<(), WifiError> {
        // wpa_supplicant doesn't have a power switch — that's a kernel
        // / framework concern. We treat enabled=false as DISCONNECT.
        if !enabled {
            return self.disconnect();
        }
        self.request("RECONNECT")?;
        Ok(())
    }
}

// -----------------------------------------------------------------------------
// Parsers — pure functions, fully unit-tested.
// -----------------------------------------------------------------------------

fn parse_cmd_wifi_status(text: &str) -> WifiStatus {
    let mut s = WifiStatus::default();
    s.enabled = text.contains("Wifi is enabled");
    s.connected = text.contains("Wifi is connected to");

    if let Some(line) = text
        .lines()
        .find(|l| l.contains("Supplicant state: COMPLETED") || l.contains("WifiInfo:"))
    {
        // Strip the leading "WifiInfo: " label so the first chunk
        // becomes "SSID: \"tin\"" instead of "WifiInfo: SSID: \"tin\"".
        let body = line.trim_start().strip_prefix("WifiInfo: ").unwrap_or(line);
        // Pull every "key: value" pair on the line; values may contain
        // spaces and end at the next ", "
        for chunk in body.split(", ") {
            let Some((k, v)) = chunk.split_once(": ") else { continue };
            let k = k.trim();
            let v = v.trim();
            match k {
                "SSID" => {
                    let cleaned = v.trim_matches('"').to_string();
                    if !cleaned.is_empty() && cleaned != "<unknown ssid>" {
                        s.ssid = Some(cleaned);
                    }
                }
                "BSSID" => {
                    if v != "02:00:00:00:00:00" {
                        s.bssid = Some(v.to_string());
                    }
                }
                "IP" => s.ip_address = Some(v.trim_start_matches('/').to_string()),
                "RSSI" => s.rssi_dbm = v.parse().ok(),
                "Link speed" => s.link_speed_mbps = v.trim_end_matches("Mbps").parse().ok(),
                "Frequency" => s.frequency_mhz = v.trim_end_matches("MHz").parse().ok(),
                "Security type" => s.security = Some(v.to_string()),
                "Net ID" => s.net_id = v.parse().ok(),
                _ => {}
            }
        }
    }
    s
}

fn parse_cmd_wifi_scan(text: &str) -> Vec<ScanResult> {
    let mut results = Vec::new();
    for line in text.lines() {
        let line = line.trim_start();
        // Header line: "BSSID Frequency RSSI Age(sec) SSID Flags" — skip.
        if line.starts_with("BSSID") || line.is_empty() {
            continue;
        }
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() < 4 {
            continue;
        }
        let bssid = parts[0].to_string();
        let Ok(freq) = parts[1].parse::<u32>() else { continue };
        // RSSI col is e.g. "-87(0:-94/1:-100)" — take the leading number.
        let rssi: i32 = parts[2]
            .split_once('(')
            .map(|(p, _)| p)
            .unwrap_or(parts[2])
            .parse()
            .unwrap_or(0);
        // The Age column may contain ">1000.0" or "12.3"; we don't use it
        // but skip past it.
        // Remaining: "<SSID...> <flags>" — flags always start with `[`.
        // Find the first token starting with `[` — everything before is SSID.
        let rest = &parts[4..];
        let flags_idx = rest.iter().position(|t| t.starts_with('['));
        let (ssid_parts, flag_parts) = match flags_idx {
            Some(i) => (&rest[..i], &rest[i..]),
            None => (rest, &[][..]),
        };
        let ssid = ssid_parts.join(" ");
        let flags = flag_parts.join("");
        results.push(ScanResult {
            bssid,
            ssid,
            frequency_mhz: freq,
            rssi_dbm: rssi,
            flags,
        });
    }
    results
}

fn parse_cmd_wifi_networks(text: &str) -> Vec<SavedNetwork> {
    let mut out: Vec<SavedNetwork> = Vec::new();
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with("Network Id") {
            continue;
        }
        // First whitespace-separated token = network id; remainder is
        // "<SSID...>  <Security type>". Security is the last 1–2 tokens
        // matching {open, wpa2-psk, wpa3-sae, owe, wep, ...}. SSIDs may
        // contain spaces/unicode, so split from the right.
        let (id_str, rest) = match trimmed.split_once(char::is_whitespace) {
            Some(t) => t,
            None => continue,
        };
        let Ok(id) = id_str.parse::<i32>() else { continue };
        let rest = rest.trim_start();
        // Heuristic: security column ends at end of line; the last
        // token (no spaces) is the security type. Strip trailing `^`.
        let (ssid_part, sec_part) = match rest.rsplit_once(char::is_whitespace) {
            Some(t) => t,
            None => (rest, ""),
        };
        let ssid = ssid_part.trim().to_string();
        let security = sec_part.trim_end_matches('^').to_string();

        // Dedupe by id — `cmd wifi list-networks` lists each saved net
        // twice (wpa2-psk + wpa3-sae transition mode).
        if !out.iter().any(|n| n.id == id) {
            out.push(SavedNetwork { id, ssid, security });
        }
    }
    out
}

fn parse_wpa_status(text: &str) -> WifiStatus {
    let mut s = WifiStatus::default();
    for line in text.lines() {
        let Some((k, v)) = line.split_once('=') else { continue };
        match k {
            "wpa_state" => s.connected = v == "COMPLETED",
            "ssid" => s.ssid = Some(v.to_string()),
            "bssid" => s.bssid = Some(v.to_string()),
            "ip_address" => s.ip_address = Some(v.to_string()),
            "freq" => s.frequency_mhz = v.parse().ok(),
            "key_mgmt" => s.security = Some(v.to_string()),
            "id" => s.net_id = v.parse().ok(),
            _ => {}
        }
    }
    s.enabled = s.connected || s.ssid.is_some();
    s
}

fn parse_wpa_scan(text: &str) -> Vec<ScanResult> {
    let mut out = Vec::new();
    for line in text.lines() {
        if line.starts_with("bssid") || line.is_empty() {
            continue;
        }
        // tab-separated: bssid \t freq \t signal \t flags \t ssid
        let cols: Vec<&str> = line.split('\t').collect();
        if cols.len() < 5 {
            continue;
        }
        let Ok(freq) = cols[1].parse::<u32>() else { continue };
        let rssi: i32 = cols[2].parse().unwrap_or(0);
        out.push(ScanResult {
            bssid: cols[0].to_string(),
            frequency_mhz: freq,
            rssi_dbm: rssi,
            flags: cols[3].to_string(),
            ssid: cols[4].to_string(),
        });
    }
    out
}

fn parse_wpa_networks(text: &str) -> Vec<SavedNetwork> {
    let mut out = Vec::new();
    for line in text.lines() {
        if line.starts_with("network id") || line.is_empty() {
            continue;
        }
        let cols: Vec<&str> = line.split('\t').collect();
        if cols.len() < 3 {
            continue;
        }
        let Ok(id) = cols[0].parse::<i32>() else { continue };
        out.push(SavedNetwork {
            id,
            ssid: cols[1].to_string(),
            security: cols.get(3).map(|s| s.to_string()).unwrap_or_default(),
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_STATUS: &str = "Wifi is enabled
Wifi scanning is always available
==== ClientModeManager instance: ConcreteClientModeManager{id=27353 iface=wlan0 role=ROLE_CLIENT_PRIMARY} ====
Wifi is connected to \"tin\"
WifiInfo: SSID: \"tin\", BSSID: 64:64:4a:bd:34:0c, MAC: 64:a2:f9:eb:f6:d4, IP: /192.168.31.5, Security type: 2, Supplicant state: COMPLETED, Wi-Fi standard: 5, RSSI: -50, Link speed: 866Mbps, Tx Link speed: 866Mbps, Max Supported Tx Link speed: 1733Mbps, Rx Link speed: 433Mbps, Max Supported Rx Link speed: 1733Mbps, Frequency: 5220MHz, Net ID: 8";

    #[test]
    fn parses_real_cmd_wifi_status() {
        let s = parse_cmd_wifi_status(SAMPLE_STATUS);
        assert!(s.enabled);
        assert!(s.connected);
        assert_eq!(s.ssid.as_deref(), Some("tin"));
        assert_eq!(s.bssid.as_deref(), Some("64:64:4a:bd:34:0c"));
        assert_eq!(s.ip_address.as_deref(), Some("192.168.31.5"));
        assert_eq!(s.rssi_dbm, Some(-50));
        assert_eq!(s.link_speed_mbps, Some(866));
        assert_eq!(s.frequency_mhz, Some(5220));
        assert_eq!(s.security.as_deref(), Some("2"));
        assert_eq!(s.net_id, Some(8));
    }

    #[test]
    fn parses_disconnected_cmd_wifi_status() {
        let text = "Wifi is enabled\nWifi is disconnected";
        let s = parse_cmd_wifi_status(text);
        assert!(s.enabled);
        assert!(!s.connected);
        assert!(s.ssid.is_none());
    }

    #[test]
    fn parses_real_cmd_wifi_scan() {
        let text = "    BSSID              Frequency      RSSI           Age(sec)     SSID                                 Flags
  64:64:4a:bd:34:0c       5220    -38(0:-41/1:-47)   >1000.0    tin                               [WPA2-PSK-CCMP][RSN-PSK-CCMP][ESS][WPS]
  c8:c4:65:f5:d4:f4       2412    -74(0:-80)          >1000.0    SONSON-2.4G                       [WPA2-PSK-CCMP][RSN-PSK-CCMP][ESS]
  6c:44:2a:52:4c:7f       2437    -87(0:-94/1:-100)   >1000.0                                      [WPA2-PSK-CCMP][ESS]";
        let scans = parse_cmd_wifi_scan(text);
        assert!(scans.len() >= 3);
        let tin = scans.iter().find(|s| s.ssid == "tin").unwrap();
        assert_eq!(tin.bssid, "64:64:4a:bd:34:0c");
        assert_eq!(tin.frequency_mhz, 5220);
        assert_eq!(tin.rssi_dbm, -38);
        assert!(tin.flags.contains("WPA2"));
    }

    #[test]
    fn parses_cmd_wifi_networks_with_unicode_ssids() {
        let text = "Network Id      SSID                         Security type
0               .@ TRUEWIFI                   open
1            (*´◒`*) (HI)                     wpa2-psk
1            (*´◒`*) (HI)                     wpa3-sae^
8            tin                              wpa2-psk
8            tin                              wpa3-sae^";
        let nets = parse_cmd_wifi_networks(text);
        assert_eq!(nets.len(), 3);
        assert_eq!(nets.iter().find(|n| n.id == 8).unwrap().ssid, "tin");
        assert_eq!(
            nets.iter().find(|n| n.id == 1).unwrap().ssid,
            "(*´◒`*) (HI)"
        );
        // Dedupe: two entries collapsed to one.
        assert_eq!(nets.iter().filter(|n| n.id == 1).count(), 1);
    }

    #[test]
    fn parses_wpa_status_kv_format() {
        let text = "bssid=64:64:4a:bd:34:0c
freq=5220
ssid=tin
id=8
mode=station
wifi_generation=5
pairwise_cipher=CCMP
group_cipher=CCMP
key_mgmt=WPA2-PSK
wpa_state=COMPLETED
ip_address=192.168.31.5";
        let s = parse_wpa_status(text);
        assert!(s.connected);
        assert!(s.enabled);
        assert_eq!(s.ssid.as_deref(), Some("tin"));
        assert_eq!(s.bssid.as_deref(), Some("64:64:4a:bd:34:0c"));
        assert_eq!(s.ip_address.as_deref(), Some("192.168.31.5"));
        assert_eq!(s.security.as_deref(), Some("WPA2-PSK"));
        assert_eq!(s.frequency_mhz, Some(5220));
        assert_eq!(s.net_id, Some(8));
    }

    #[test]
    fn parses_wpa_scan_tab_format() {
        let text = "bssid / frequency / signal level / flags / ssid
64:64:4a:bd:34:0c\t5220\t-38\t[WPA2-PSK-CCMP][ESS]\ttin
c8:c4:65:f5:d4:f4\t2412\t-74\t[WPA2-PSK-CCMP][ESS]\tSONSON-2.4G";
        let s = parse_wpa_scan(text);
        assert_eq!(s.len(), 2);
        assert_eq!(s[0].ssid, "tin");
        assert_eq!(s[0].rssi_dbm, -38);
        assert_eq!(s[1].frequency_mhz, 2412);
    }
}
