//! Phase 23 — shared file-RPC client for the PekoOverlay bridge.
//!
//! Every bridge service (audio, gps, telephony, camera) follows the
//! same protocol: write request + sentinel into `<topic>/in/`, poll
//! for `.done` in `<topic>/out/`, read response + optional asset, clean
//! up. This module factors that out so each tool just calls one helper.
//!
//! All four bridges live under `/data/data/com.peko.overlay/files/`.
//! peko-agent runs as root and can read/write across UID boundaries.

use anyhow::Context;
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use tokio::time::sleep;
use uuid::Uuid;

pub const APP_FILES_DIR: &str = "/data/data/com.peko.overlay/files";
pub const POLL_INTERVAL_MS: u64 = 200;
pub const DEFAULT_TIMEOUT_SECS: u64 = 30;
pub const MAX_TIMEOUT_SECS: u64 = 130;

/// Path to the shared event SQLite database written by every streaming
/// service inside the priv-app. peko-agent reads via rusqlite directly.
pub fn events_db_path() -> PathBuf {
    PathBuf::from("/data/data/com.peko.overlay/databases/events.db")
}

pub struct BridgeResponse {
    pub json: Value,
    pub asset: Option<PathBuf>,
}

pub struct BridgeRequest<'a> {
    pub topic: &'a str,
    pub body: Value,
    pub input_asset: Option<&'a Path>,
    pub input_asset_ext: &'a str,
    pub timeout: Duration,
}

pub async fn send(req: BridgeRequest<'_>) -> anyhow::Result<BridgeResponse> {
    let topic_root = PathBuf::from(APP_FILES_DIR).join(req.topic);
    let in_dir = topic_root.join("in");
    let out_dir = topic_root.join("out");
    tokio::fs::create_dir_all(&in_dir).await
        .with_context(|| format!("create {}", in_dir.display()))?;
    tokio::fs::create_dir_all(&out_dir).await
        .with_context(|| format!("create {}", out_dir.display()))?;

    let id = Uuid::new_v4().to_string();
    let req_path = in_dir.join(format!("{id}.json"));
    let in_asset = in_dir.join(format!("{id}.{}", req.input_asset_ext));
    let start_path = in_dir.join(format!("{id}.start"));
    let out_json = out_dir.join(format!("{id}.json"));
    let done = out_dir.join(format!("{id}.done"));

    // Cleanup guard intentionally excludes any output asset
    // (jpg/wav/png/bin). Caller's persist_asset() copies the file out
    // AFTER this function returns; deleting it in the guard would race
    // the copy and produce ENOENT. The priv-app's out/ directory leaks
    // these until overwritten — bounded by usage, cheap to live with.
    let _guard = CleanupGuard {
        paths: vec![
            req_path.clone(), in_asset.clone(), start_path.clone(),
            out_json.clone(), done.clone(),
        ],
    };

    tokio::fs::write(&req_path, serde_json::to_vec(&req.body)?).await?;
    if let Some(src) = req.input_asset {
        tokio::fs::copy(src, &in_asset).await?;
    }
    let _ = chmod_world_readable(&req_path).await;
    if req.input_asset.is_some() { let _ = chmod_world_readable(&in_asset).await; }

    tokio::fs::write(&start_path, b"").await?;
    let _ = chmod_world_readable(&start_path).await;

    let deadline = Instant::now() + req.timeout;
    while Instant::now() < deadline {
        if done.exists() {
            let body = tokio::fs::read(&out_json).await
                .unwrap_or_else(|_| b"{\"ok\":false,\"error\":\"out json missing\"}".to_vec());
            let json: Value = serde_json::from_slice(&body)?;
            // Asset extension is whatever the bridge wrote alongside;
            // we look for common extensions in priority order.
            let asset = ["jpg", "wav", "png", "bin"].iter()
                .map(|ext| out_dir.join(format!("{id}.{ext}")))
                .find(|p| p.exists() && std::fs::metadata(p).map(|m| m.len() > 0).unwrap_or(false));
            return Ok(BridgeResponse { json, asset });
        }
        sleep(Duration::from_millis(POLL_INTERVAL_MS)).await;
    }
    anyhow::bail!(
        "bridge '{}' timed out after {:?}. Is PekoOverlay running? \
         dumpsys activity services com.peko.overlay",
        req.topic, req.timeout
    )
}

pub fn pick_timeout(args: &Value, default_secs: u64) -> Duration {
    let secs = args["timeout_secs"]
        .as_u64()
        .unwrap_or(default_secs.max(DEFAULT_TIMEOUT_SECS))
        .min(MAX_TIMEOUT_SECS);
    Duration::from_secs(secs)
}

/// Best-effort chmod 0644 so the priv-app's own UID can read files
/// peko-agent (root) wrote. Failure is non-fatal — the bridge dispatcher
/// retries by reading anyway, since priv-app can read its own files
/// dir regardless of mode.
async fn chmod_world_readable(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let mut perms = tokio::fs::metadata(path).await?.permissions();
    perms.set_mode(0o644);
    tokio::fs::set_permissions(path, perms).await
}

struct CleanupGuard {
    paths: Vec<PathBuf>,
}
impl Drop for CleanupGuard {
    fn drop(&mut self) {
        for p in &self.paths { let _ = std::fs::remove_file(p); }
    }
}

/// Move a bridge-output asset to a permanent location under /data/peko/.
/// Use AFTER reading the bridge response — the cleanup guard will not
/// touch it once it's at the new path.
pub async fn persist_asset(src: &Path, sub: &str) -> anyhow::Result<PathBuf> {
    let dest_dir = PathBuf::from("/data/peko").join(sub);
    tokio::fs::create_dir_all(&dest_dir).await.ok();
    let name = src.file_name().unwrap_or_default().to_string_lossy().to_string();
    let dest = dest_dir.join(name);
    tokio::fs::copy(src, &dest).await?;
    Ok(dest)
}
