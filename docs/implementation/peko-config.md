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

[security]
# Optional lockscreen auto-unlock. Plaintext; anyone with root
# already can read it, so no additional exposure. Omit or leave
# empty to disable.
# lock_pin = "1234"

[calls]
# Voice-call recording + transcription + summary pipeline.
# Opt-in; default false.
enabled           = false
recordings_dir    = "/data/data/com.peko.shim.sms/files/calls"
stt_base_url      = "https://api.openai.com/v1"
stt_model         = "whisper-1"
# stt_api_key     = "sk-..."        # blank -> reads OPENAI_API_KEY env
# stt_language    = "en"            # blank -> auto-detect
min_duration_ms   = 2000            # skip pocket dials
retain_audio_days = 7               # .m4a retention; transcripts live forever in DB
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
    // …openai / groq / deepseek / mistral / together / embedded +
    // a #[serde(flatten)] `extra: HashMap<String, Value>` catch-all
    // so user-named custom providers (e.g. `[provider.xiaomi]`) are
    // preserved through a get_config → set_config round-trip.
}

#[derive(Debug, Deserialize, Default)]
pub struct SecurityConfig {
    // Digits only. set_lock_pin() validates.
    pub lock_pin: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct CallsConfig {
    pub enabled: bool,
    pub recordings_dir: String,
    pub stt_base_url: String,
    pub stt_api_key: Option<String>,
    pub stt_model: String,
    pub stt_language: Option<String>,
    pub min_duration_ms: u64,
    pub retain_audio_days: i64,
}
```

`[calls]` is read by `crates/peko-core/src/call_pipeline.rs` every poll tick
from the shared `Arc<Mutex<Value>>` — so toggling `enabled` from the web UI
takes effect on the next 10 s cycle without restarting the daemon. The STT
key is masked (`first4...last4`) in `GET /api/config`; the UI sends `****`
back on save to signal "keep existing" without round-tripping the secret.

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
