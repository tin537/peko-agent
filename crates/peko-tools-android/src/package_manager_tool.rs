use peko_core::tool::{Tool, ToolResult};
use peko_hal::PackageManager;
use serde_json::json;
use std::future::Future;
use std::path::Path;
use std::pin::Pin;

pub struct PackageManagerTool;

impl PackageManagerTool {
    pub fn new() -> Self { Self }
}

impl Tool for PackageManagerTool {
    fn name(&self) -> &str { "package_manager" }

    fn description(&self) -> &str {
        "Manage Android packages (APKs). Actions: \
         install (install APK from path), \
         uninstall (remove package), \
         list (list installed packages, optional filter: 'third_party', 'system', 'enabled', 'disabled'), \
         info (get package details), \
         enable/disable (toggle package), \
         clear (clear app data), \
         launch (start app), \
         stop (force stop app), \
         methods (show available install methods)."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["install", "uninstall", "list", "info", "enable", "disable",
                             "clear", "launch", "stop", "methods"],
                    "description": "Package management action"
                },
                "package": {
                    "type": "string",
                    "description": "Package name (e.g., com.example.app) or APK path (for install)"
                },
                "filter": {
                    "type": "string",
                    "description": "Filter for list action: 'third_party', 'system', 'enabled', 'disabled'"
                }
            },
            "required": ["action"]
        })
    }

    fn is_dangerous(&self) -> bool { true }

    fn is_available(&self) -> bool {
        !PackageManager::available_methods().is_empty()
    }

    fn execute(
        &self,
        args: serde_json::Value,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<ToolResult>> + Send + '_>> {
        Box::pin(async move {
            let action = args["action"].as_str()
                .ok_or_else(|| anyhow::anyhow!("missing 'action'"))?;

            match action {
                "install" => {
                    let package = args["package"].as_str()
                        .ok_or_else(|| anyhow::anyhow!("missing 'package' (APK path)"))?;
                    let apk_path = Path::new(package);

                    if !apk_path.exists() {
                        return Ok(ToolResult::error(format!("APK not found: {}", package)));
                    }

                    // Try pm first, then direct
                    if PackageManager::pm_available() {
                        match PackageManager::pm_install(apk_path) {
                            Ok(msg) => Ok(ToolResult::success(msg)),
                            Err(e) => {
                                // Try streaming install for large APKs
                                match PackageManager::pm_install_stream(apk_path) {
                                    Ok(msg) => Ok(ToolResult::success(msg)),
                                    Err(e2) => Ok(ToolResult::error(
                                        format!("Install failed: {} (streaming: {})", e, e2)))
                                }
                            }
                        }
                    } else {
                        match PackageManager::direct_install(apk_path) {
                            Ok(msg) => Ok(ToolResult::success(format!(
                                "{}\nNote: Direct install — app may need manual dex optimization", msg))),
                            Err(e) => Ok(ToolResult::error(format!("Direct install failed: {}", e)))
                        }
                    }
                }

                "uninstall" => {
                    let package = args["package"].as_str()
                        .ok_or_else(|| anyhow::anyhow!("missing 'package'"))?;

                    match PackageManager::pm_uninstall(package) {
                        Ok(msg) => Ok(ToolResult::success(msg)),
                        Err(e) => Ok(ToolResult::error(format!("{}", e)))
                    }
                }

                "list" => {
                    let filter = args["filter"].as_str();
                    match PackageManager::pm_list_packages(filter) {
                        Ok(packages) => {
                            if packages.is_empty() {
                                Ok(ToolResult::success("No packages found".to_string()))
                            } else {
                                let output = format!("{} packages:\n{}",
                                    packages.len(),
                                    packages.join("\n"));
                                Ok(ToolResult::success(output))
                            }
                        }
                        Err(e) => Ok(ToolResult::error(format!("List failed: {}", e)))
                    }
                }

                "info" => {
                    let package = args["package"].as_str()
                        .ok_or_else(|| anyhow::anyhow!("missing 'package'"))?;

                    match PackageManager::pm_package_info(package) {
                        Ok(info) => {
                            Ok(ToolResult::success(format!(
                                "Package: {}\nAPK: {}\nVersion: {} ({})\nEnabled: {}",
                                info.package_name,
                                info.apk_path,
                                info.version_name.unwrap_or_else(|| "?".to_string()),
                                info.version_code.map(|v| v.to_string()).unwrap_or_else(|| "?".to_string()),
                                info.enabled,
                            )))
                        }
                        Err(e) => Ok(ToolResult::error(format!("Info failed: {}", e)))
                    }
                }

                "enable" => {
                    let package = args["package"].as_str()
                        .ok_or_else(|| anyhow::anyhow!("missing 'package'"))?;
                    match PackageManager::pm_set_enabled(package, true) {
                        Ok(msg) => Ok(ToolResult::success(msg)),
                        Err(e) => Ok(ToolResult::error(format!("{}", e)))
                    }
                }

                "disable" => {
                    let package = args["package"].as_str()
                        .ok_or_else(|| anyhow::anyhow!("missing 'package'"))?;
                    match PackageManager::pm_set_enabled(package, false) {
                        Ok(msg) => Ok(ToolResult::success(msg)),
                        Err(e) => Ok(ToolResult::error(format!("{}", e)))
                    }
                }

                "clear" => {
                    let package = args["package"].as_str()
                        .ok_or_else(|| anyhow::anyhow!("missing 'package'"))?;
                    match PackageManager::pm_clear(package) {
                        Ok(msg) => Ok(ToolResult::success(msg)),
                        Err(e) => Ok(ToolResult::error(format!("{}", e)))
                    }
                }

                "launch" => {
                    let package = args["package"].as_str()
                        .ok_or_else(|| anyhow::anyhow!("missing 'package'"))?;
                    match PackageManager::am_start(package) {
                        Ok(msg) => Ok(ToolResult::success(msg)),
                        Err(e) => Ok(ToolResult::error(format!("{}", e)))
                    }
                }

                "stop" => {
                    let package = args["package"].as_str()
                        .ok_or_else(|| anyhow::anyhow!("missing 'package'"))?;
                    match PackageManager::am_force_stop(package) {
                        Ok(msg) => Ok(ToolResult::success(msg)),
                        Err(e) => Ok(ToolResult::error(format!("{}", e)))
                    }
                }

                "methods" => {
                    let methods = PackageManager::available_methods();
                    let names: Vec<&str> = methods.iter().map(|m| match m {
                        peko_hal::package_manager::InstallMethod::Pm => "pm (shell command)",
                        peko_hal::package_manager::InstallMethod::Direct => "direct (/data/app/ write)",
                        peko_hal::package_manager::InstallMethod::Installd => "installd (socket)",
                    }).collect();

                    Ok(ToolResult::success(format!(
                        "Available install methods:\n{}",
                        if names.is_empty() { "  none".to_string() } else { names.join("\n") }
                    )))
                }

                _ => Ok(ToolResult::error(format!("unknown action: {}", action)))
            }
        })
    }
}
