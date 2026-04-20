use axum::{extract::State, extract::Query, response::{IntoResponse, Response}, Json};
use serde::{Serialize, Deserialize};
use std::process::Command;
use std::collections::HashMap;

use super::api::AppState;

// ═══════════════════════════════════════════════════════════
// Device Profile — identity, hardware, screen, tools
// ═══════════════════════════════════════════════════════════

#[derive(Serialize)]
pub struct DeviceProfile {
    identity: DeviceIdentity,
    screen: ScreenInfo,
    hardware: HardwareInfo,
    tools: Vec<ToolStatus>,
    android: AndroidInfo,
}

#[derive(Serialize)]
struct DeviceIdentity {
    model: String,
    manufacturer: String,
    brand: String,
    device: String,
    serial: String,
    fingerprint: String,
}

#[derive(Serialize)]
struct ScreenInfo {
    width: i32,
    height: i32,
    density: i32,
    density_name: String,
}

#[derive(Serialize)]
struct HardwareInfo {
    cpu_abi: String,
    cpu_cores: usize,
    ram_total_mb: u64,
    soc: String,
    has_touchscreen: bool,
    has_framebuffer: bool,
    has_modem: bool,
    has_wifi: bool,
    has_camera: bool,
    input_devices: Vec<String>,
}

#[derive(Serialize)]
struct ToolStatus {
    name: String,
    available: bool,
    method: String,
}

#[derive(Serialize)]
struct AndroidInfo {
    version: String,
    api_level: String,
    build_type: String,
    security_patch: String,
    selinux: String,
    rooted: bool,
}

pub async fn device_profile(State(state): State<AppState>) -> Json<DeviceProfile> {
    let tools = state.tools.available_tools().iter()
        .map(|name| ToolStatus {
            name: name.to_string(),
            available: true,
            method: match *name {
                "screenshot" => "framebuffer/screencap",
                "touch" => "/dev/input evdev",
                "key_event" => "/dev/input evdev",
                "text_input" => "/dev/uinput",
                "sms" => "AT commands /dev/ttyACM*",
                "call" => "AT commands /dev/ttyACM*",
                "shell" => "sh -c",
                "filesystem" => "POSIX I/O",
                "ui_inspect" => "uiautomator/screencap",
                "package_manager" => "pm/am/installd",
                _ => "native",
            }.to_string(),
        })
        .collect();

    Json(DeviceProfile {
        identity: get_identity(),
        screen: get_screen(),
        hardware: get_hardware(),
        tools,
        android: get_android_info(),
    })
}

fn get_identity() -> DeviceIdentity {
    DeviceIdentity {
        model: prop("ro.product.model"),
        manufacturer: prop("ro.product.manufacturer"),
        brand: prop("ro.product.brand"),
        device: prop("ro.product.device"),
        serial: prop("ro.serialno"),
        fingerprint: prop("ro.build.fingerprint"),
    }
}

fn get_screen() -> ScreenInfo {
    let wm = shell("wm size 2>/dev/null");
    let (w, h) = if let Some(size) = wm.split(':').last() {
        let parts: Vec<i32> = size.trim().split('x').filter_map(|s| s.parse().ok()).collect();
        if parts.len() == 2 { (parts[0], parts[1]) } else { (0, 0) }
    } else { (0, 0) };

    let density: i32 = shell("wm density 2>/dev/null").split(':').last()
        .and_then(|s| s.trim().parse().ok()).unwrap_or(0);

    let density_name = match density {
        0..=120 => "ldpi",
        121..=160 => "mdpi",
        161..=240 => "hdpi",
        241..=320 => "xhdpi",
        321..=480 => "xxhdpi",
        _ => "xxxhdpi",
    }.to_string();

    ScreenInfo { width: w, height: h, density, density_name }
}

fn get_hardware() -> HardwareInfo {
    let cores = shell("nproc").parse().unwrap_or(1);
    let meminfo = shell("cat /proc/meminfo | head -1");
    let ram_kb: u64 = meminfo.split_whitespace().nth(1).and_then(|s| s.parse().ok()).unwrap_or(0);

    let input_devs: Vec<String> = shell("cat /proc/bus/input/devices 2>/dev/null")
        .split("I:")
        .filter_map(|block| {
            let name = block.lines().find(|l| l.starts_with("N: Name="))
                .map(|l| l.trim_start_matches("N: Name=").trim_matches('"').to_string());
            name
        })
        .filter(|n| !n.is_empty())
        .collect();

    let has_touch = input_devs.iter().any(|n| {
        let l = n.to_lowercase();
        l.contains("touch") || l.contains("ts") || l.contains("virtio") || l.contains("goldfish")
    });

    // Framebuffer: legacy fb0 OR modern DRM/KMS OR screencap available
    let has_fb = std::path::Path::new("/dev/graphics/fb0").exists()
        || std::path::Path::new("/dev/fb0").exists()
        || std::path::Path::new("/dev/dri/card0").exists()
        || !shell("which screencap 2>/dev/null").is_empty();

    // Modem: serial devices OR RIL (Radio Interface Layer) running
    let has_modem = [
        "/dev/ttyACM0", "/dev/ttyACM1",
        "/dev/ttyUSB0", "/dev/ttyUSB2",
        "/dev/ttyGF0", "/dev/ttyGF1",
        "/dev/ttyMSM0",
        "/dev/ttyS0", "/dev/ttyS1",
    ].iter().any(|p| std::path::Path::new(p).exists())
        || shell("getprop gsm.version.ril-impl 2>/dev/null").len() > 0;

    HardwareInfo {
        cpu_abi: prop("ro.product.cpu.abi"),
        cpu_cores: cores,
        ram_total_mb: ram_kb / 1024,
        soc: prop("ro.hardware"),
        has_touchscreen: has_touch,
        has_framebuffer: has_fb,
        has_modem,
        has_wifi: shell("ip link show wlan0 2>/dev/null").contains("wlan0")
            || shell("ip link show eth0 2>/dev/null").contains("eth0"),
        has_camera: !shell("ls /dev/video* 2>/dev/null").is_empty() || prop("ro.camera.notify_nfc") != "",
        input_devices: input_devs,
    }
}

fn get_android_info() -> AndroidInfo {
    AndroidInfo {
        version: prop("ro.build.version.release"),
        api_level: prop("ro.build.version.sdk"),
        build_type: prop("ro.build.type"),
        security_patch: prop("ro.build.version.security_patch"),
        selinux: shell("getenforce 2>/dev/null"),
        rooted: shell("which su 2>/dev/null").contains("su") || shell("id").contains("uid=0"),
    }
}

fn prop(name: &str) -> String {
    shell(&format!("getprop {} 2>/dev/null", name))
}

// ═══════════════════════════════════════════════════════════
// Device Monitor — CPU, memory, battery, disk, network
// ═══════════════════════════════════════════════════════════

#[derive(Serialize)]
pub struct DeviceStats {
    cpu: CpuInfo,
    memory: MemInfo,
    battery: BatteryInfo,
    disk: DiskInfo,
    network: NetworkInfo,
    uptime: String,
    processes: Vec<ProcessInfo>,
}

#[derive(Serialize)]
struct CpuInfo {
    usage_percent: f32,
    cores: usize,
    load_avg: String,
}

#[derive(Serialize)]
struct MemInfo {
    total_mb: u64,
    available_mb: u64,
    used_mb: u64,
    used_percent: f32,
    peko_rss_mb: f32,
}

#[derive(Serialize)]
struct BatteryInfo {
    level: i32,
    status: String,
    temperature: f32,
    voltage: f32,
}

#[derive(Serialize)]
struct DiskInfo {
    data_total_mb: u64,
    data_free_mb: u64,
    data_used_percent: f32,
}

#[derive(Serialize)]
struct NetworkInfo {
    wifi_connected: bool,
    ip_address: String,
    wifi_ssid: String,
}

#[derive(Serialize)]
struct ProcessInfo {
    pid: String,
    name: String,
    rss_kb: String,
    cpu: String,
}

pub async fn device_stats(State(_state): State<AppState>) -> Json<DeviceStats> {
    Json(DeviceStats {
        cpu: get_cpu_info(),
        memory: get_mem_info(),
        battery: get_battery_info(),
        disk: get_disk_info(),
        network: get_network_info(),
        uptime: get_uptime(),
        processes: get_top_processes(),
    })
}

fn shell(cmd: &str) -> String {
    Command::new("sh").arg("-c").arg(cmd)
        .output().map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default()
}

fn get_cpu_info() -> CpuInfo {
    let cores = shell("nproc").parse().unwrap_or(1);
    let load_avg = shell("cat /proc/loadavg").split_whitespace()
        .take(3).collect::<Vec<_>>().join(" ");

    // CPU usage from /proc/stat snapshot
    let stat1 = shell("cat /proc/stat | head -1");
    std::thread::sleep(std::time::Duration::from_millis(200));
    let stat2 = shell("cat /proc/stat | head -1");

    let usage = calc_cpu_usage(&stat1, &stat2);

    CpuInfo { usage_percent: usage, cores, load_avg }
}

fn calc_cpu_usage(stat1: &str, stat2: &str) -> f32 {
    let parse = |s: &str| -> Vec<u64> {
        s.split_whitespace().skip(1).take(7)
            .filter_map(|v| v.parse().ok()).collect()
    };
    let v1 = parse(stat1);
    let v2 = parse(stat2);
    if v1.len() < 4 || v2.len() < 4 { return 0.0; }

    let total1: u64 = v1.iter().sum();
    let total2: u64 = v2.iter().sum();
    let idle1 = v1[3];
    let idle2 = v2[3];

    let total_diff = total2.saturating_sub(total1) as f32;
    let idle_diff = idle2.saturating_sub(idle1) as f32;

    if total_diff > 0.0 { ((total_diff - idle_diff) / total_diff * 100.0).min(100.0) } else { 0.0 }
}

fn get_mem_info() -> MemInfo {
    let meminfo = shell("cat /proc/meminfo");
    let parse_kb = |key: &str| -> u64 {
        meminfo.lines().find(|l| l.starts_with(key))
            .and_then(|l| l.split_whitespace().nth(1))
            .and_then(|v| v.parse().ok())
            .unwrap_or(0)
    };

    let total_kb = parse_kb("MemTotal:");
    let available_kb = parse_kb("MemAvailable:");
    let used_kb = total_kb.saturating_sub(available_kb);

    let peko_rss = peko_core::MemStats::current_portable()
        .map(|s| s.rss_mb as f32).unwrap_or(0.0);

    MemInfo {
        total_mb: total_kb / 1024,
        available_mb: available_kb / 1024,
        used_mb: used_kb / 1024,
        used_percent: if total_kb > 0 { used_kb as f32 / total_kb as f32 * 100.0 } else { 0.0 },
        peko_rss_mb: peko_rss,
    }
}

fn get_battery_info() -> BatteryInfo {
    let level = shell("cat /sys/class/power_supply/battery/capacity")
        .parse().unwrap_or(-1);
    let status = shell("cat /sys/class/power_supply/battery/status");
    let temp = shell("cat /sys/class/power_supply/battery/temp")
        .parse::<f32>().unwrap_or(0.0) / 10.0;
    let voltage = shell("cat /sys/class/power_supply/battery/voltage_now")
        .parse::<f32>().unwrap_or(0.0) / 1_000_000.0;

    // Fallback for emulator
    let status = if status.is_empty() {
        shell("dumpsys battery 2>/dev/null | grep status").split(':').last()
            .unwrap_or("Unknown").trim().to_string()
    } else { status };

    let level = if level < 0 {
        shell("dumpsys battery 2>/dev/null | grep level").split(':').last()
            .and_then(|s| s.trim().parse().ok()).unwrap_or(-1)
    } else { level };

    BatteryInfo { level, status, temperature: temp, voltage }
}

fn get_disk_info() -> DiskInfo {
    let df = shell("df /data 2>/dev/null | tail -1");
    let parts: Vec<&str> = df.split_whitespace().collect();

    if parts.len() >= 4 {
        let total = parts[1].parse::<u64>().unwrap_or(0) / 1024;
        let used = parts[2].parse::<u64>().unwrap_or(0) / 1024;
        let free = parts[3].parse::<u64>().unwrap_or(0) / 1024;
        let pct = if total > 0 { used as f32 / total as f32 * 100.0 } else { 0.0 };
        DiskInfo { data_total_mb: total, data_free_mb: free, data_used_percent: pct }
    } else {
        DiskInfo { data_total_mb: 0, data_free_mb: 0, data_used_percent: 0.0 }
    }
}

fn get_network_info() -> NetworkInfo {
    let ip = shell("ip addr show wlan0 2>/dev/null | grep 'inet ' | awk '{print $2}' | cut -d/ -f1");
    let wifi = !ip.is_empty();
    let ssid = shell("dumpsys wifi 2>/dev/null | grep 'mWifiInfo' | grep -oP 'SSID: [^,]+' | cut -d' ' -f2");

    let ip = if ip.is_empty() {
        shell("ip addr show eth0 2>/dev/null | grep 'inet ' | awk '{print $2}' | cut -d/ -f1")
    } else { ip };

    NetworkInfo {
        wifi_connected: wifi,
        ip_address: if ip.is_empty() { "no network".to_string() } else { ip },
        wifi_ssid: if ssid.is_empty() { "N/A".to_string() } else { ssid },
    }
}

fn get_uptime() -> String {
    let uptime = shell("cat /proc/uptime");
    let secs: f64 = uptime.split_whitespace().next()
        .and_then(|s| s.parse().ok()).unwrap_or(0.0);

    let hours = secs as u64 / 3600;
    let mins = (secs as u64 % 3600) / 60;
    format!("{}h {}m", hours, mins)
}

fn get_top_processes() -> Vec<ProcessInfo> {
    let ps = shell("ps -eo pid,rss,pcpu,comm --sort=-rss 2>/dev/null | head -11 || ps -A -o pid,rss,pcpu,comm 2>/dev/null | head -11 || ps 2>/dev/null | head -11");
    ps.lines().skip(1).filter_map(|line| {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() >= 4 {
            Some(ProcessInfo {
                pid: parts[0].to_string(),
                rss_kb: parts[1].to_string(),
                cpu: parts[2].to_string(),
                name: parts[3..].join(" "),
            })
        } else { None }
    }).take(10).collect()
}

// ═══════════════════════════════════════════════════════════
// Log streaming — logcat via SSE
// ═══════════════════════════════════════════════════════════

pub async fn log_stream(State(_state): State<AppState>) -> Response {
    let stream = async_stream::stream! {
        // Get recent logcat lines then stream new ones
        let mut child = match tokio::process::Command::new("logcat")
            .arg("-v").arg("time")
            .arg("-T").arg("50")
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .spawn() {
                Ok(c) => c,
                Err(e) => {
                    yield Ok::<_, std::convert::Infallible>(format!("data: {}\n\n",
                        serde_json::json!({"type":"error","message":format!("logcat failed: {}",e)})));
                    return;
                }
            };

        let stdout = child.stdout.take().unwrap();
        let mut reader = tokio::io::BufReader::new(stdout);
        let mut line = String::new();

        loop {
            line.clear();
            match tokio::io::AsyncBufReadExt::read_line(&mut reader, &mut line).await {
                Ok(0) => break,
                Ok(_) => {
                    let trimmed = line.trim();
                    if !trimmed.is_empty() {
                        yield Ok(format!("data: {}\n\n",
                            serde_json::json!({"type":"log","line":trimmed})));
                    }
                }
                Err(_) => break,
            }
        }

        let _ = child.kill().await;
    };

    let body = axum::body::Body::from_stream(stream);
    axum::response::Response::builder()
        .header("content-type", "text/event-stream")
        .header("cache-control", "no-cache")
        .body(body).unwrap()
}

// ═══════════════════════════════════════════════════════════
// Installed apps list — with icons and filter
// ═══════════════════════════════════════════════════════════

#[derive(Serialize, Clone)]
pub struct AppInfo {
    package: String,
    label: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    version: Option<String>,
    app_type: String,
    enabled: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    icon: Option<String>,
    apk_path: String,
}

#[derive(Deserialize)]
pub struct AppQuery {
    #[serde(default)]
    filter: Option<String>, // "user", "system", or empty for all
    /// When true, extract + base64 the launcher icon out of each APK.
    /// Off by default because it's the slowest part of the whole path
    /// (aapt + unzip per user app), and the UI has a graceful letter-
    /// initial fallback when icon is None.
    #[serde(default)]
    icons: bool,
}

/// In-process cache for /api/apps. List-packages output is stable for
/// tens of seconds at a time; a 30-second TTL keeps the Apps tab
/// instant on reload while still picking up installs/uninstalls
/// within a reasonable window. Key includes the filter AND the icons
/// flag because the two result sizes differ by ~10x.
///
/// Cleared implicitly by TTL; on app install/uninstall the user will
/// see stale data for up to 30s, which is fine — the next poll flushes.
static APPS_CACHE: tokio::sync::Mutex<Option<AppsCacheEntry>> =
    tokio::sync::Mutex::const_new(None);

struct AppsCacheEntry {
    filter: String,
    icons: bool,
    at: std::time::Instant,
    apps: Vec<AppInfo>,
}

/// How many enrichment shell-outs may run in parallel. Each `dumpsys
/// package <pkg>` / `aapt dump badging` fork hits a binder +
/// filesystem burst. 16 is empirically a sweet spot on sdm845 — below
/// that we're CPU-idle; above it the system_server binder queue
/// starts contending with itself and total wall time flattens out.
const APP_ENRICH_CONCURRENCY: usize = 16;
const APPS_CACHE_TTL: std::time::Duration = std::time::Duration::from_secs(30);

pub async fn list_apps(
    State(_state): State<AppState>,
    Query(query): Query<AppQuery>,
) -> Json<Vec<AppInfo>> {
    let filter = query.filter.as_deref().unwrap_or("all").to_string();
    let want_icons = query.icons;

    // Cache fast path — return the last result if the query matches
    // and it's still fresh.
    {
        let cache = APPS_CACHE.lock().await;
        if let Some(ref entry) = *cache {
            if entry.filter == filter
                && entry.icons == want_icons
                && entry.at.elapsed() < APPS_CACHE_TTL
            {
                return Json(entry.apps.clone());
            }
        }
    }

    let mut apps = Vec::new();

    // The three `pm list packages` calls are cheap by comparison (a
    // few hundred ms each). We still run them on a blocking thread
    // to keep the tokio worker free; sh() itself is sync.
    let (user_lines, sys_lines, disabled_lines) = tokio::task::spawn_blocking(|| {
        (
            shell("pm list packages -3 -f 2>/dev/null"),
            shell("pm list packages -s -f 2>/dev/null"),
            shell("pm list packages -d 2>/dev/null"),
        )
    }).await.unwrap_or_default();

    if filter == "all" || filter == "user" {
        for line in user_lines.lines() {
            if let Some(app) = parse_package_line(line, "user") {
                apps.push(app);
            }
        }
    }
    if filter == "all" || filter == "system" {
        for line in sys_lines.lines() {
            if let Some(app) = parse_package_line(line, "system") {
                apps.push(app);
            }
        }
    }
    for line in disabled_lines.lines() {
        if let Some(pkg) = line.strip_prefix("package:") {
            if let Some(app) = apps.iter_mut().find(|a| a.package == pkg) {
                app.enabled = false;
            }
        }
    }

    // User-app enrichment (label + version, optionally icon). Each
    // app's work is a tight chain of shell()+grep over dumpsys /
    // aapt; we fan out to spawn_blocking so those don't stall the
    // tokio worker, and chunk the fan-out so we never have more
    // than APP_ENRICH_CONCURRENCY processes live at once. Wall-time
    // on sdm845 drops from ~50s sequential to under 2s with 16-way.
    let user_targets: Vec<(usize, String, String)> = apps.iter()
        .enumerate()
        .filter(|(_, a)| a.app_type == "user")
        .map(|(i, a)| (i, a.package.clone(), a.apk_path.clone()))
        .collect();

    for chunk in user_targets.chunks(APP_ENRICH_CONCURRENCY) {
        let handles: Vec<_> = chunk.iter().cloned().map(|(idx, pkg, apk)| {
            tokio::task::spawn_blocking(move || {
                let label = get_app_label(&pkg);
                let version = get_app_version(&pkg);
                let icon = if want_icons { get_app_icon_b64(&apk) } else { None };
                (idx, label, version, icon)
            })
        }).collect();
        for h in handles {
            if let Ok((idx, label, version, icon)) = h.await {
                if let Some(app) = apps.get_mut(idx) {
                    if let Some(l) = label { app.label = l; }
                    if version.is_some() { app.version = version; }
                    if icon.is_some()    { app.icon    = icon; }
                }
            }
        }
    }

    apps.sort_by(|a, b| {
        a.app_type.cmp(&b.app_type)
            .then(a.label.to_lowercase().cmp(&b.label.to_lowercase()))
    });

    // Update cache.
    {
        let mut cache = APPS_CACHE.lock().await;
        *cache = Some(AppsCacheEntry {
            filter,
            icons: want_icons,
            at: std::time::Instant::now(),
            apps: apps.clone(),
        });
    }

    Json(apps)
}

fn parse_package_line(line: &str, app_type: &str) -> Option<AppInfo> {
    // Format: "package:/path/to/base.apk=com.example.app"
    let stripped = line.strip_prefix("package:")?;
    let eq_pos = stripped.rfind('=')?;
    let apk_path = stripped[..eq_pos].to_string();
    let package = stripped[eq_pos+1..].to_string();

    Some(AppInfo {
        label: package.rsplit('.').next().unwrap_or(&package).to_string(),
        package,
        version: None,
        app_type: app_type.to_string(),
        enabled: true,
        icon: None,
        apk_path,
    })
}

fn get_app_label(pkg: &str) -> Option<String> {
    let out = shell(&format!(
        "dumpsys package {} 2>/dev/null | grep -A1 'applicationInfo' | grep 'labelRes\\|nonLocalizedLabel' | head -1", pkg
    ));
    // Try pm dump
    if out.is_empty() {
        let label = shell(&format!(
            "cmd package dump {} 2>/dev/null | grep -m1 'label=' | sed 's/.*label=//'", pkg
        ));
        if !label.is_empty() { return Some(label); }
    }
    None
}

fn get_app_version(pkg: &str) -> Option<String> {
    let info = shell(&format!("dumpsys package {} 2>/dev/null | grep versionName | head -1", pkg));
    info.split('=').last().map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

fn get_app_icon_b64(apk_path: &str) -> Option<String> {
    if apk_path.is_empty() { return None; }
    // Extract icon from APK using aapt
    let icon_path = shell(&format!(
        "aapt dump badging '{}' 2>/dev/null | grep 'application-icon-160\\|application-icon-240\\|application-icon-320' | head -1 | sed \"s/.*'\\(.*\\)'/\\1/\"",
        apk_path
    ));
    if icon_path.is_empty() { return None; }

    // Extract the icon file from APK
    let b64 = shell(&format!(
        "unzip -p '{}' '{}' 2>/dev/null | base64 2>/dev/null | tr -d '\\n'",
        apk_path, icon_path
    ));
    if b64.len() > 100 { // sanity check
        Some(format!("data:image/png;base64,{}", b64))
    } else {
        None
    }
}

// ═══════════════════════════════════════════════════════════
// App actions
// ═══════════════════════════════════════════════════════════

#[derive(serde::Deserialize)]
pub struct AppAction {
    package: String,
    action: String, // launch, stop, uninstall, enable, disable, clear
}

pub async fn app_action(
    State(_state): State<AppState>,
    Json(req): Json<AppAction>,
) -> Json<serde_json::Value> {
    let result = match req.action.as_str() {
        "launch" => shell(&format!("monkey -p {} -c android.intent.category.LAUNCHER 1 2>&1", req.package)),
        "stop" => shell(&format!("am force-stop {} 2>&1", req.package)),
        "uninstall" => shell(&format!("pm uninstall {} 2>&1", req.package)),
        "enable" => shell(&format!("pm enable {} 2>&1", req.package)),
        "disable" => shell(&format!("pm disable {} 2>&1", req.package)),
        "clear" => shell(&format!("pm clear {} 2>&1", req.package)),
        _ => "unknown action".to_string(),
    };

    Json(serde_json::json!({"result": result}))
}

// ═══════════════════════════════════════════════════════════
// SMS / Notification stream
// ═══════════════════════════════════════════════════════════

pub async fn messages_stream(State(_state): State<AppState>) -> Response {
    let stream = async_stream::stream! {
        // Stream SMS and notifications via logcat filtering
        let mut child = match tokio::process::Command::new("logcat")
            .arg("-v").arg("time")
            .arg("-s")
            .arg("Telephony:*")
            .arg("SmsReceiver:*")
            .arg("NotificationService:*")
            .arg("StatusBarNotification:*")
            .arg("NotificationListenerService:*")
            .arg("Peko:*")
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .spawn() {
                Ok(c) => c,
                Err(e) => {
                    yield Ok::<_, std::convert::Infallible>(format!("data: {}\n\n",
                        serde_json::json!({"type":"error","message":format!("logcat failed: {}",e)})));
                    return;
                }
            };

        // Also poll SMS database periodically
        let mut last_sms_check = std::time::Instant::now();

        let stdout = child.stdout.take().unwrap();
        let mut reader = tokio::io::BufReader::new(stdout);
        let mut line = String::new();

        // Send initial SMS list
        let sms_list = get_recent_sms();
        if !sms_list.is_empty() {
            yield Ok(format!("data: {}\n\n",
                serde_json::json!({"type":"sms_history","messages":sms_list})));
        }

        // Send current notifications
        let notifs = get_current_notifications();
        if !notifs.is_empty() {
            yield Ok(format!("data: {}\n\n",
                serde_json::json!({"type":"notifications","items":notifs})));
        }

        loop {
            line.clear();
            match tokio::io::AsyncBufReadExt::read_line(&mut reader, &mut line).await {
                Ok(0) => break,
                Ok(_) => {
                    let trimmed = line.trim();
                    if !trimmed.is_empty() {
                        // Classify the log line
                        let lower = trimmed.to_lowercase();
                        if lower.contains("sms") || lower.contains("mms") || lower.contains("telephony") {
                            yield Ok(format!("data: {}\n\n",
                                serde_json::json!({"type":"sms_event","line":trimmed})));
                        } else if lower.contains("notification") {
                            yield Ok(format!("data: {}\n\n",
                                serde_json::json!({"type":"notification_event","line":trimmed})));
                        }
                    }

                    // Periodic SMS re-check
                    if last_sms_check.elapsed() > std::time::Duration::from_secs(10) {
                        last_sms_check = std::time::Instant::now();
                        let sms = get_recent_sms();
                        if !sms.is_empty() {
                            yield Ok(format!("data: {}\n\n",
                                serde_json::json!({"type":"sms_update","messages":sms})));
                        }
                    }
                }
                Err(_) => break,
            }
        }

        let _ = child.kill().await;
    };

    let body = axum::body::Body::from_stream(stream);
    axum::response::Response::builder()
        .header("content-type", "text/event-stream")
        .header("cache-control", "no-cache")
        .body(body).unwrap()
}

fn get_recent_sms() -> Vec<serde_json::Value> {
    // Query the telephony provider database directly. peko-agent runs as
    // root on Magisk-installed devices, so we can bypass the `content`
    // CLI (which has a shell-quoting landmine around `date DESC LIMIT 20`
    // that silently turned the command into a usage error).
    //
    // Android 11+ moved the DB to /data/user_de/0/; the legacy path is
    // kept for older ROMs. Use whichever exists.
    const DBS: &[&str] = &[
        "/data/user_de/0/com.android.providers.telephony/databases/mmssms.db",
        "/data/data/com.android.providers.telephony/databases/mmssms.db",
    ];
    let Some(db) = DBS.iter().find(|p| std::path::Path::new(*p).exists()) else {
        return vec![];
    };
    // \x1f is the ASCII Unit Separator — guaranteed not to appear in an
    // SMS address or body, so we can split fields without worrying about
    // commas/tabs/quotes in the content.
    let cmd = format!(
        "sqlite3 -separator $'\\x1f' {} \"SELECT address, body, date FROM sms WHERE type=1 ORDER BY date DESC LIMIT 20;\"",
        db
    );
    let out = shell(&cmd);
    if out.trim().is_empty() { return vec![]; }

    out.lines().filter_map(|line| {
        let parts: Vec<&str> = line.splitn(3, '\u{1f}').collect();
        if parts.len() < 3 { return None; }
        // `date` is ms since epoch — convert to an ISO8601 string for the UI.
        let ts_ms = parts[2].parse::<i64>().ok()?;
        let ts = chrono::DateTime::<chrono::Utc>::from_timestamp_millis(ts_ms)
            .map(|d| d.to_rfc3339())
            .unwrap_or_else(|| parts[2].to_string());
        Some(serde_json::json!({
            "from": parts[0],
            "body": parts[1],
            "date": ts,
        }))
    }).collect()
}

fn get_current_notifications() -> Vec<serde_json::Value> {
    // Full dumpsys output — no `grep -A5` prefilter, which was cutting off
    // `tickerText=` / `android.text=` lines that live 10-20 rows below the
    // NotificationRecord header. Scope is bounded by the total dump size
    // (a few hundred KB on a busy device; fine to parse in-process).
    let raw = shell("dumpsys notification --noredact 2>/dev/null");
    if raw.is_empty() { return vec![]; }

    let mut items: Vec<serde_json::Value> = Vec::new();
    let mut cur_pkg:   Option<String> = None;
    let mut cur_title: Option<String> = None;
    let mut cur_text:  Option<String> = None;
    let mut cur_ticker: Option<String> = None;

    // Extract text after `key=` on a dumpsys line. Two modes:
    //   compact=true  — value ends at the next whitespace; used for pkg,
    //                   uid, and other single-token fields so we don't
    //                   slurp "user=... id=..." into the value.
    //   compact=false — value ends at EOL; used for title/text/tickerText
    //                   which contain spaces and the whole line belongs.
    //
    // Also unwraps the `String (…)` / `SpannableString (…)` / `CharSequence (…)`
    // wrappers that Android's toString prefixes on notification fields.
    fn after(line: &str, key: &str, compact: bool) -> Option<String> {
        let start = line.find(key)? + key.len();
        let rest = line[start..].trim_start_matches('"');
        let end = if compact {
            rest.find(|c: char| c.is_whitespace()).unwrap_or(rest.len())
        } else {
            rest.find(['\n', '\r']).unwrap_or(rest.len())
        };
        let mut v = rest[..end].trim().trim_end_matches('"').to_string();
        // Android's toString on text fields wraps the content in a type
        // prefix — `String (…)`, `SpannableString (…)`, `CharSequence (…)`.
        // A multi-line SpannableString gets split across dumpsys rows, so
        // we only see the opening half on the one line we matched.
        // Strip the prefix regardless; trim the trailing `)` only if it's
        // present on this line.
        for prefix in ["String (", "SpannableString (", "CharSequence ("] {
            if let Some(stripped) = v.strip_prefix(prefix) {
                v = stripped.to_string();
                if v.ends_with(')') {
                    v.truncate(v.len() - 1);
                }
                break;
            }
        }
        if v == "null" { return None; }
        if v.is_empty() { return None; }
        Some(v.trim().to_string())
    }

    let flush = |items: &mut Vec<serde_json::Value>,
                 pkg: &mut Option<String>,
                 title: &mut Option<String>,
                 text: &mut Option<String>,
                 ticker: &mut Option<String>| {
        if let Some(p) = pkg.take() {
            // Prefer the richest text we found: title+text, then text alone,
            // then tickerText (which often has the fullest human-readable
            // summary on TikTok / social apps).
            let body = match (title.take(), text.take(), ticker.take()) {
                (Some(t), Some(b), _) if !b.is_empty() => format!("{} — {}", t, b),
                (Some(t), _,       _) if !t.is_empty() => t,
                (_,       Some(b), _) if !b.is_empty() => b,
                (_,       _,       Some(tk)) if !tk.is_empty() => tk,
                _ => String::new(),
            };
            items.push(serde_json::json!({"package": p, "text": body}));
        } else {
            *title = None; *text = None; *ticker = None;
        }
    };

    for raw_line in raw.lines() {
        let line = raw_line.trim_start();
        // Each NotificationRecord starts a fresh entry.
        if line.starts_with("NotificationRecord(") {
            flush(&mut items, &mut cur_pkg, &mut cur_title, &mut cur_text, &mut cur_ticker);
            cur_pkg = after(line, "pkg=", true);
            continue;
        }
        if cur_pkg.is_none() { continue; }

        if let Some(t) = after(line, "android.title=",   false) { if cur_title.is_none()  { cur_title  = Some(t); } }
        if let Some(t) = after(line, "android.text=",    false) { if cur_text.is_none()   { cur_text   = Some(t); } }
        if let Some(t) = after(line, "android.bigText=", false) { cur_text = Some(t); }
        if let Some(t) = after(line, "tickerText=",      false) { if cur_ticker.is_none() { cur_ticker = Some(t); } }
    }
    flush(&mut items, &mut cur_pkg, &mut cur_title, &mut cur_text, &mut cur_ticker);

    // Drop the invisible system channel spam (android OS internal notifs
    // with no text) and cap to 20 so the panel stays scannable.
    items.retain(|it| {
        it.get("text").and_then(|v| v.as_str()).map(|s| !s.is_empty()).unwrap_or(false)
    });
    items.truncate(20);
    items
}

fn extract_field(line: &str, prefix: &str) -> Option<String> {
    let start = line.find(prefix)? + prefix.len();
    let rest = &line[start..];
    let end = rest.find(", ").unwrap_or(rest.len());
    Some(rest[..end].to_string())
}
