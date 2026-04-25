use peko_config::DeviceProfile;
use peko_core::tool::{Tool, ToolResult};
use peko_hal::{wifi_auto_backend, WifiHints};
use serde_json::json;
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;

pub struct WifiTool;

impl WifiTool {
    pub fn new() -> Self { Self }
}

impl Default for WifiTool {
    fn default() -> Self { Self::new() }
}

fn hints_from_profile() -> WifiHints {
    // Best-effort: load the device profile if it exists. If anything
    // goes wrong (missing file, parse error) we fall back to defaults
    // and let `auto_backend` discover the socket itself.
    let profile = DeviceProfile::load().unwrap_or_default();
    WifiHints {
        ctrl_socket_path: profile.wifi.ctrl_socket_path.map(PathBuf::from),
        ctrl_socket_dir: profile.wifi.ctrl_socket_dir.map(PathBuf::from),
        prefer_wpa_supplicant: false,
    }
}

impl Tool for WifiTool {
    fn name(&self) -> &str { "wifi" }

    fn description(&self) -> &str {
        "Inspect and control Wi-Fi without going through framework Java APIs. \
         Actions: status (current connection), scan (list nearby APs), \
         list_networks (saved profiles), connect (join an SSID), \
         disconnect, enable, disable. \
         Backend is auto-selected: cmd wifi (Lane B) → wpa_supplicant ctrl socket (Lane A)."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["status", "scan", "list_networks", "connect",
                             "disconnect", "enable", "disable"],
                    "description": "Operation to perform."
                },
                "ssid": {
                    "type": "string",
                    "description": "SSID for connect."
                },
                "password": {
                    "type": "string",
                    "description": "Password for connect (omit for open networks)."
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
        let ssid = args["ssid"].as_str().map(String::from);
        let password = args["password"].as_str().map(String::from);
        Box::pin(async move {
            tokio::task::spawn_blocking(move || dispatch(&action, ssid, password))
                .await
                .map_err(|e| anyhow::anyhow!("wifi task panicked: {e}"))?
        })
    }
}

fn dispatch(
    action: &str,
    ssid: Option<String>,
    password: Option<String>,
) -> anyhow::Result<ToolResult> {
    let hints = hints_from_profile();
    let backend = match wifi_auto_backend(&hints) {
        Ok(b) => b,
        Err(e) => return Ok(ToolResult::error(format!("no wifi backend available: {e}"))),
    };
    let label = backend.name();

    match action {
        "status" => match backend.status() {
            Ok(s) => Ok(ToolResult::success(format_status(&s, label))),
            Err(e) => Ok(ToolResult::error(format!("status failed via {label}: {e}"))),
        },
        "scan" => match backend.scan() {
            Ok(list) => Ok(ToolResult::success(format_scan(&list, label))),
            Err(e) => Ok(ToolResult::error(format!("scan failed via {label}: {e}"))),
        },
        "list_networks" => match backend.list_networks() {
            Ok(list) => Ok(ToolResult::success(format_networks(&list, label))),
            Err(e) => Ok(ToolResult::error(format!("list_networks failed via {label}: {e}"))),
        },
        "connect" => {
            let Some(ssid) = ssid else {
                return Ok(ToolResult::error("missing 'ssid' for connect".to_string()));
            };
            match backend.connect(&ssid, password.as_deref()) {
                Ok(_) => Ok(ToolResult::success(format!(
                    "connect to '{ssid}' submitted via {label}; check `status` to verify"
                ))),
                Err(e) => Ok(ToolResult::error(format!("connect failed via {label}: {e}"))),
            }
        }
        "disconnect" => match backend.disconnect() {
            Ok(_) => Ok(ToolResult::success(format!("disconnected via {label}"))),
            Err(e) => Ok(ToolResult::error(format!("disconnect failed via {label}: {e}"))),
        },
        "enable" => match backend.set_enabled(true) {
            Ok(_) => Ok(ToolResult::success(format!("Wi-Fi enabled via {label}"))),
            Err(e) => Ok(ToolResult::error(format!("enable failed via {label}: {e}"))),
        },
        "disable" => match backend.set_enabled(false) {
            Ok(_) => Ok(ToolResult::success(format!("Wi-Fi disabled via {label}"))),
            Err(e) => Ok(ToolResult::error(format!("disable failed via {label}: {e}"))),
        },
        "" => Ok(ToolResult::error("missing 'action' parameter".to_string())),
        other => Ok(ToolResult::error(format!(
            "unknown action '{other}'. valid: status, scan, list_networks, connect, \
             disconnect, enable, disable"
        ))),
    }
}

fn format_status(s: &peko_hal::WifiStatus, label: &str) -> String {
    let mut out = format!("Wi-Fi status (via {label}):");
    out.push_str(&format!("\n  enabled: {}", s.enabled));
    out.push_str(&format!("\n  connected: {}", s.connected));
    if let Some(ssid) = &s.ssid {
        out.push_str(&format!("\n  ssid: {ssid}"));
    }
    if let Some(bssid) = &s.bssid {
        out.push_str(&format!("\n  bssid: {bssid}"));
    }
    if let Some(ip) = &s.ip_address {
        out.push_str(&format!("\n  ip: {ip}"));
    }
    if let Some(rssi) = s.rssi_dbm {
        out.push_str(&format!("\n  rssi: {rssi} dBm"));
    }
    if let Some(freq) = s.frequency_mhz {
        out.push_str(&format!("\n  frequency: {freq} MHz"));
    }
    if let Some(speed) = s.link_speed_mbps {
        out.push_str(&format!("\n  link_speed: {speed} Mbps"));
    }
    if let Some(sec) = &s.security {
        out.push_str(&format!("\n  security: {sec}"));
    }
    if let Some(id) = s.net_id {
        out.push_str(&format!("\n  net_id: {id}"));
    }
    out
}

fn format_scan(list: &[peko_hal::ScanResult], label: &str) -> String {
    if list.is_empty() {
        return format!("No scan results from {label} yet — try again in a moment.");
    }
    let mut out = format!("{} access points (via {label}):\n", list.len());
    let mut sorted: Vec<&peko_hal::ScanResult> = list.iter().collect();
    sorted.sort_by_key(|s| -s.rssi_dbm); // strongest first
    for r in sorted.iter().take(30) {
        out.push_str(&format!(
            "  {:>4} dBm  {} MHz  {:<32}  {}\n",
            r.rssi_dbm,
            r.frequency_mhz,
            if r.ssid.is_empty() { "<hidden>" } else { &r.ssid },
            r.flags
        ));
    }
    if list.len() > 30 {
        out.push_str(&format!("  … {} more (truncated)\n", list.len() - 30));
    }
    out
}

fn format_networks(list: &[peko_hal::SavedNetwork], label: &str) -> String {
    if list.is_empty() {
        return format!("No saved networks (via {label}).");
    }
    let mut out = format!("{} saved network(s) (via {label}):\n", list.len());
    for n in list {
        out.push_str(&format!("  {:>4}  {:<40}  {}\n", n.id, n.ssid, n.security));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use peko_hal::{SavedNetwork, ScanResult, WifiStatus};

    #[test]
    fn status_format_includes_all_fields() {
        let s = WifiStatus {
            enabled: true,
            connected: true,
            ssid: Some("tin".into()),
            bssid: Some("aa:bb:cc:dd:ee:ff".into()),
            ip_address: Some("192.168.1.5".into()),
            rssi_dbm: Some(-50),
            link_speed_mbps: Some(866),
            frequency_mhz: Some(5220),
            security: Some("WPA2-PSK".into()),
            net_id: Some(8),
        };
        let f = format_status(&s, "cmd-wifi");
        assert!(f.contains("ssid: tin"));
        assert!(f.contains("rssi: -50 dBm"));
        assert!(f.contains("frequency: 5220 MHz"));
    }

    #[test]
    fn scan_sorts_by_signal_strength() {
        let l = vec![
            ScanResult {
                bssid: "a".into(),
                ssid: "weak".into(),
                frequency_mhz: 2412,
                rssi_dbm: -90,
                flags: "[ESS]".into(),
            },
            ScanResult {
                bssid: "b".into(),
                ssid: "strong".into(),
                frequency_mhz: 5180,
                rssi_dbm: -40,
                flags: "[ESS]".into(),
            },
        ];
        let out = format_scan(&l, "cmd-wifi");
        // strong should come before weak in output
        let strong_idx = out.find("strong").unwrap();
        let weak_idx = out.find("weak").unwrap();
        assert!(strong_idx < weak_idx);
    }

    #[test]
    fn networks_format_lists_each_id() {
        let l = vec![SavedNetwork {
            id: 8,
            ssid: "tin".into(),
            security: "wpa2-psk".into(),
        }];
        let out = format_networks(&l, "cmd-wifi");
        assert!(out.contains("tin"));
        assert!(out.contains("wpa2-psk"));
    }

    #[test]
    fn schema_lists_all_actions() {
        let t = WifiTool::new();
        let schema = t.parameters_schema();
        let actions: Vec<&str> = schema["properties"]["action"]["enum"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|v| v.as_str())
            .collect();
        for a in [
            "status", "scan", "list_networks", "connect", "disconnect", "enable", "disable",
        ] {
            assert!(actions.contains(&a), "missing action {a}");
        }
    }
}
