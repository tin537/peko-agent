use std::fs;
use std::time::Instant;
use tracing::{info, warn};

const RSS_LIMIT_MB: u64 = 50;

#[derive(Debug, Clone)]
pub struct MemStats {
    pub rss_kb: u64,
    pub vms_kb: u64,
    pub rss_mb: f64,
    pub vms_mb: f64,
}

impl MemStats {
    pub fn current() -> Option<Self> {
        // Linux /proc/self/statm: size resident shared text lib data dt (in pages)
        let statm = fs::read_to_string("/proc/self/statm").ok()?;
        let parts: Vec<u64> = statm.split_whitespace()
            .filter_map(|s| s.parse().ok())
            .collect();

        if parts.len() < 2 { return None; }

        let page_size = unsafe { libc::sysconf(libc::_SC_PAGESIZE) } as u64;
        let vms_kb = parts[0] * page_size / 1024;
        let rss_kb = parts[1] * page_size / 1024;

        Some(Self {
            rss_kb,
            vms_kb,
            rss_mb: rss_kb as f64 / 1024.0,
            vms_mb: vms_kb as f64 / 1024.0,
        })
    }

    /// Try macOS alternative (no /proc)
    pub fn current_portable() -> Option<Self> {
        Self::current().or_else(Self::current_rusage)
    }

    fn current_rusage() -> Option<Self> {
        unsafe {
            let mut usage: libc::rusage = std::mem::zeroed();
            if libc::getrusage(libc::RUSAGE_SELF, &mut usage) == 0 {
                // maxrss is in KB on Linux, bytes on macOS
                #[cfg(target_os = "macos")]
                let rss_kb = usage.ru_maxrss as u64 / 1024;
                #[cfg(not(target_os = "macos"))]
                let rss_kb = usage.ru_maxrss as u64;

                Some(Self {
                    rss_kb,
                    vms_kb: 0,
                    rss_mb: rss_kb as f64 / 1024.0,
                    vms_mb: 0.0,
                })
            } else {
                None
            }
        }
    }

    pub fn is_over_limit(&self) -> bool {
        self.rss_kb > RSS_LIMIT_MB * 1024
    }
}

pub struct MemMonitor {
    start: Instant,
    check_interval_secs: u64,
    last_check: Instant,
}

impl MemMonitor {
    pub fn new(check_interval_secs: u64) -> Self {
        let now = Instant::now();
        Self {
            start: now,
            check_interval_secs,
            last_check: now,
        }
    }

    /// Check memory and log if it's time. Returns true if over RSS limit.
    pub fn check(&mut self) -> bool {
        let now = Instant::now();
        if now.duration_since(self.last_check).as_secs() < self.check_interval_secs {
            return false;
        }
        self.last_check = now;

        let uptime = now.duration_since(self.start).as_secs();

        if let Some(stats) = MemStats::current_portable() {
            info!(
                rss_mb = format!("{:.1}", stats.rss_mb),
                vms_mb = format!("{:.1}", stats.vms_mb),
                uptime_s = uptime,
                "memory check"
            );

            if stats.is_over_limit() {
                warn!(
                    rss_mb = format!("{:.1}", stats.rss_mb),
                    limit_mb = RSS_LIMIT_MB,
                    "RSS exceeds target! Consider reducing context window or screenshot resolution"
                );
                return true;
            }
        }
        false
    }

    /// Get current stats for API/status endpoint
    pub fn snapshot() -> serde_json::Value {
        match MemStats::current_portable() {
            Some(stats) => serde_json::json!({
                "rss_mb": format!("{:.1}", stats.rss_mb),
                "vms_mb": format!("{:.1}", stats.vms_mb),
                "rss_kb": stats.rss_kb,
                "over_limit": stats.is_over_limit(),
                "limit_mb": RSS_LIMIT_MB,
            }),
            None => serde_json::json!({
                "error": "unable to read memory stats"
            }),
        }
    }
}
