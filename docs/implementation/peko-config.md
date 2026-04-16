# peko-config

> Configuration parsing and management.

---

## Purpose

`peko-config` deserializes the TOML configuration file and provides typed access to all runtime settings. It supports environment variable overrides and Android system property reads.

## Configuration File

Located at `/data/peko/config.toml`:

```toml
[agent]
max_iterations = 50
context_window = 200000
history_share = 0.7
data_dir = "/data/peko"
log_level = "info"

[provider.anthropic]
api_key = "sk-ant-..."
model = "claude-sonnet-4-20250514"
base_url = "https://api.anthropic.com"
max_tokens = 8192

[provider.openrouter]
api_key = "sk-or-..."
model = "ftmstars/peko-3-llama-3.1-405b"
base_url = "https://openrouter.ai/api/v1"
max_tokens = 4096

[provider.local]
model = "peko-3-8b"
base_url = "http://localhost:8080/v1"
max_tokens = 4096

[provider]
# Priority order for failover
priority = ["anthropic", "openrouter", "local"]

[tools]
screenshot = true
touch = true
key_event = true
text_input = true
sms = true
call = true
ui_dump = true
notification = true
filesystem = true
shell = true

[tools.filesystem]
allowed_paths = ["/data/peko", "/sdcard"]

[tools.shell]
timeout_seconds = 30

[hardware]
# Override auto-detection (optional)
# touchscreen_device = "/dev/input/event2"
# framebuffer_device = "/dev/graphics/fb0"
# modem_device = "/dev/ttyACM0"

[startup]
# Optional: run a task immediately on boot
# task = "Check for new SMS messages and summarize them"
```

## Config Struct

```rust
#[derive(Debug, Deserialize)]
pub struct PekoConfig {
    pub agent: AgentConfig,
    pub provider: ProviderConfig,
    pub tools: ToolsConfig,
    pub hardware: Option<HardwareConfig>,
    pub startup: Option<StartupConfig>,
}

#[derive(Debug, Deserialize)]
pub struct AgentConfig {
    pub max_iterations: usize,
    pub context_window: usize,
    pub history_share: f32,
    pub data_dir: PathBuf,
    pub log_level: String,
}

#[derive(Debug, Deserialize)]
pub struct ProviderConfig {
    pub anthropic: Option<ProviderEntry>,
    pub openrouter: Option<ProviderEntry>,
    pub local: Option<ProviderEntry>,
    pub priority: Vec<String>,
}
```

## Environment Variable Overrides

For sensitive values and deployment flexibility:

| Variable | Overrides |
|---|---|
| `PEKO_API_KEY` | `provider.anthropic.api_key` |
| `PEKO_MODEL` | `provider.anthropic.model` |
| `PEKO_MAX_ITERATIONS` | `agent.max_iterations` |
| `PEKO_LOG_LEVEL` | `agent.log_level` |
| `PEKO_DATA_DIR` | `agent.data_dir` |

Environment variables always take precedence over file values.

## Android System Properties

On Android, can also read values via `getprop`:

```rust
// Falls back to system property if env var and file are unset
fn resolve_value(file: Option<&str>, env: &str, prop: &str) -> Option<String> {
    std::env::var(env).ok()
        .or_else(|| file.map(String::from))
        .or_else(|| android_getprop(prop))
}
```

## Dependencies

```toml
[dependencies]
serde = { version = "1", features = ["derive"] }
toml = "0.8"
tracing = "0.1"
```

Minimal — config is a leaf crate with no internal dependencies.

## Related

- [[peko-agent-binary]] — Reads config at startup
- [[../architecture/Boot-Sequence]] — When config is loaded
- [[../architecture/Crate-Map]] — Dependency position

---

#implementation #config
