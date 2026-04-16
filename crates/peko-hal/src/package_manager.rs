use std::fs;
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

/// Package manager for installing/uninstalling/listing APKs.
/// Three methods available, in order of preference:
/// 1. Shell `pm` command (requires framework running)
/// 2. Direct APK write to /data/app/ (frameworkless, limited)
/// 3. installd socket communication (low-level)
pub struct PackageManager;

#[derive(Debug, Clone)]
pub struct PackageInfo {
    pub package_name: String,
    pub apk_path: String,
    pub version_name: Option<String>,
    pub version_code: Option<i64>,
    pub enabled: bool,
}

#[derive(Debug, Clone)]
pub enum InstallMethod {
    Pm,         // /system/bin/pm (framework running)
    Direct,     // direct file copy to /data/app/
    Installd,   // /dev/socket/installd
}

impl PackageManager {
    /// Detect which installation methods are available
    pub fn available_methods() -> Vec<InstallMethod> {
        let mut methods = Vec::new();

        if Self::pm_available() {
            methods.push(InstallMethod::Pm);
        }
        if Path::new("/data/app").exists() {
            methods.push(InstallMethod::Direct);
        }
        if Path::new("/dev/socket/installd").exists() {
            methods.push(InstallMethod::Installd);
        }

        methods
    }

    /// Check if `pm` command is available (framework running)
    pub fn pm_available() -> bool {
        Command::new("pm")
            .arg("list")
            .arg("packages")
            .arg("-3")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    // ─── pm shell commands ───────────────────────────────────────────

    /// Install APK via pm
    pub fn pm_install(apk_path: &Path) -> anyhow::Result<String> {
        let output = Command::new("pm")
            .arg("install")
            .arg("-r")  // replace existing
            .arg("-d")  // allow downgrade
            .arg(apk_path)
            .output()?;

        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();

        if output.status.success() && stdout.contains("Success") {
            Ok(format!("Installed: {}", stdout.trim()))
        } else {
            anyhow::bail!("pm install failed: {} {}", stdout.trim(), stderr.trim())
        }
    }

    /// Install APK via pm with streaming (for large APKs)
    pub fn pm_install_stream(apk_path: &Path) -> anyhow::Result<String> {
        let file_size = fs::metadata(apk_path)?.len();

        // Create install session
        let create_output = Command::new("pm")
            .arg("install-create")
            .arg("-S")
            .arg(file_size.to_string())
            .output()?;

        let create_str = String::from_utf8_lossy(&create_output.stdout);
        let session_id = extract_session_id(&create_str)
            .ok_or_else(|| anyhow::anyhow!("failed to get session ID: {}", create_str))?;

        // Write APK to session
        let write_output = Command::new("sh")
            .arg("-c")
            .arg(format!(
                "cat {} | pm install-write -S {} {} base.apk",
                apk_path.display(), file_size, session_id
            ))
            .output()?;

        if !write_output.status.success() {
            let _ = Command::new("pm").arg("install-abandon").arg(&session_id).output();
            anyhow::bail!("pm install-write failed: {}",
                String::from_utf8_lossy(&write_output.stderr));
        }

        // Commit session
        let commit_output = Command::new("pm")
            .arg("install-commit")
            .arg(&session_id)
            .output()?;

        let result = String::from_utf8_lossy(&commit_output.stdout).to_string();
        if commit_output.status.success() && result.contains("Success") {
            Ok(format!("Installed (streamed): {}", result.trim()))
        } else {
            anyhow::bail!("pm install-commit failed: {}", result.trim())
        }
    }

    /// Uninstall package via pm
    pub fn pm_uninstall(package_name: &str) -> anyhow::Result<String> {
        let output = Command::new("pm")
            .arg("uninstall")
            .arg(package_name)
            .output()?;

        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        if output.status.success() && stdout.contains("Success") {
            Ok(format!("Uninstalled: {}", package_name))
        } else {
            anyhow::bail!("pm uninstall failed: {}", stdout.trim())
        }
    }

    /// List installed packages via pm
    pub fn pm_list_packages(filter: Option<&str>) -> anyhow::Result<Vec<String>> {
        let mut cmd = Command::new("pm");
        cmd.arg("list").arg("packages");

        if let Some(f) = filter {
            match f {
                "third_party" | "3" => { cmd.arg("-3"); }
                "system" | "s" => { cmd.arg("-s"); }
                "enabled" | "e" => { cmd.arg("-e"); }
                "disabled" | "d" => { cmd.arg("-d"); }
                _ => { cmd.arg(f); } // use as name filter
            }
        }

        let output = cmd.output()?;
        let stdout = String::from_utf8_lossy(&output.stdout);

        Ok(stdout.lines()
            .filter_map(|line| line.strip_prefix("package:"))
            .map(|s| s.to_string())
            .collect())
    }

    /// Get detailed package info via dumpsys
    pub fn pm_package_info(package_name: &str) -> anyhow::Result<PackageInfo> {
        let output = Command::new("dumpsys")
            .arg("package")
            .arg(package_name)
            .output()?;

        let stdout = String::from_utf8_lossy(&output.stdout);

        let apk_path = stdout.lines()
            .find(|l| l.trim().starts_with("codePath="))
            .and_then(|l| l.trim().strip_prefix("codePath="))
            .unwrap_or("")
            .to_string();

        let version_name = stdout.lines()
            .find(|l| l.trim().starts_with("versionName="))
            .and_then(|l| l.trim().strip_prefix("versionName="))
            .map(|s| s.to_string());

        let version_code = stdout.lines()
            .find(|l| l.trim().starts_with("versionCode="))
            .and_then(|l| l.trim().strip_prefix("versionCode="))
            .and_then(|s| s.split_whitespace().next())
            .and_then(|s| s.parse().ok());

        let enabled = !stdout.contains("enabled=false");

        Ok(PackageInfo {
            package_name: package_name.to_string(),
            apk_path,
            version_name,
            version_code,
            enabled,
        })
    }

    /// Enable/disable a package
    pub fn pm_set_enabled(package_name: &str, enabled: bool) -> anyhow::Result<String> {
        let state = if enabled { "enable" } else { "disable" };
        let output = Command::new("pm")
            .arg(state)
            .arg(package_name)
            .output()?;

        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        if output.status.success() {
            Ok(format!("Package {} {}", package_name, state))
        } else {
            anyhow::bail!("pm {} failed: {}", state, stdout.trim())
        }
    }

    /// Clear package data
    pub fn pm_clear(package_name: &str) -> anyhow::Result<String> {
        let output = Command::new("pm")
            .arg("clear")
            .arg(package_name)
            .output()?;

        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        if output.status.success() && stdout.contains("Success") {
            Ok(format!("Cleared data for {}", package_name))
        } else {
            anyhow::bail!("pm clear failed: {}", stdout.trim())
        }
    }

    /// Force stop a package
    pub fn am_force_stop(package_name: &str) -> anyhow::Result<String> {
        let output = Command::new("am")
            .arg("force-stop")
            .arg(package_name)
            .output()?;

        if output.status.success() {
            Ok(format!("Force stopped {}", package_name))
        } else {
            anyhow::bail!("am force-stop failed")
        }
    }

    /// Launch a package's main activity
    pub fn am_start(package_name: &str) -> anyhow::Result<String> {
        let output = Command::new("monkey")
            .arg("-p")
            .arg(package_name)
            .arg("-c")
            .arg("android.intent.category.LAUNCHER")
            .arg("1")
            .output()?;

        if output.status.success() {
            Ok(format!("Launched {}", package_name))
        } else {
            // Fallback: try am start
            let output2 = Command::new("am")
                .arg("start")
                .arg("-n")
                .arg(format!("{}/.MainActivity", package_name))
                .output()?;

            let stdout = String::from_utf8_lossy(&output2.stdout).to_string();
            Ok(format!("Launch attempt: {}", stdout.trim()))
        }
    }

    // ─── Direct APK install ──────────────────────────────────────────

    /// Install APK by copying directly to /data/app/ (frameworkless mode)
    /// This is a simplified install — no dex optimization, no permission granting
    pub fn direct_install(apk_path: &Path) -> anyhow::Result<String> {
        let file_name = apk_path.file_name()
            .ok_or_else(|| anyhow::anyhow!("invalid APK path"))?
            .to_string_lossy();

        let dest_dir = PathBuf::from("/data/app").join(file_name.replace(".apk", ""));
        fs::create_dir_all(&dest_dir)?;

        let dest_apk = dest_dir.join("base.apk");
        fs::copy(apk_path, &dest_apk)?;

        // Set permissions
        Command::new("chmod").arg("644").arg(&dest_apk).output()?;
        Command::new("chown").arg("system:system").arg(&dest_apk).output()?;

        Ok(format!("Direct installed to {}", dest_dir.display()))
    }

    // ─── installd socket ─────────────────────────────────────────────

    /// Send a command to installd via its Unix socket
    pub fn installd_command(cmd: &str) -> anyhow::Result<String> {
        let socket_path = "/dev/socket/installd";
        if !Path::new(socket_path).exists() {
            anyhow::bail!("installd socket not found at {}", socket_path);
        }

        let mut stream = UnixStream::connect(socket_path)?;
        stream.set_read_timeout(Some(Duration::from_secs(10)))?;
        stream.set_write_timeout(Some(Duration::from_secs(5)))?;

        // installd protocol: length-prefixed commands
        let cmd_bytes = cmd.as_bytes();
        let len = cmd_bytes.len() as u16;
        stream.write_all(&len.to_be_bytes())?;
        stream.write_all(cmd_bytes)?;
        stream.flush()?;

        let mut len_buf = [0u8; 2];
        stream.read_exact(&mut len_buf)?;
        let resp_len = u16::from_be_bytes(len_buf) as usize;

        let mut resp_buf = vec![0u8; resp_len];
        stream.read_exact(&mut resp_buf)?;

        Ok(String::from_utf8_lossy(&resp_buf).to_string())
    }

    /// Request dex optimization via installd
    pub fn installd_dexopt(apk_path: &str, package_name: &str) -> anyhow::Result<String> {
        let cmd = format!("dexopt {} {} 0 * speed ! 0", apk_path, package_name);
        Self::installd_command(&cmd)
    }
}

fn extract_session_id(output: &str) -> Option<String> {
    // Output format: "Success: created install session [1234567]"
    let start = output.find('[')? + 1;
    let end = output.find(']')?;
    Some(output[start..end].to_string())
}
