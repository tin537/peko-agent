use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("failed to read config file: {0}")]
    Io(#[from] std::io::Error),
    #[error("failed to parse config: {0}")]
    Parse(#[from] toml::de::Error),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PekoConfig {
    pub agent: AgentConfig,
    pub provider: ProviderConfig,
    #[serde(default)]
    pub tools: ToolsConfig,
    pub hardware: Option<HardwareConfig>,
    pub startup: Option<StartupConfig>,
    pub telegram: Option<TelegramConfig>,
    #[serde(default)]
    pub schedule: Vec<ScheduleEntry>,
    #[serde(default)]
    pub mcp: Vec<McpConfig>,
    /// Autonomous behavior: reflections, heartbeat, curiosity, goal generation.
    /// Default-disabled. See docs/implementation/Safety-Model.md.
    #[serde(default)]
    pub autonomy: AutonomyConfig,
}

/// Controls the "digital life" autonomous behavior stack.
/// See docs/architecture/Full-Life-Roadmap.md + Safety-Model.md.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutonomyConfig {
    /// Master switch. DEFAULT FALSE — no autonomy ships enabled.
    #[serde(default)]
    pub enabled: bool,

    /// Heartbeat interval in seconds.
    #[serde(default = "default_tick_secs")]
    pub tick_interval_secs: u64,

    /// Cap on internally-generated tasks per rolling hour.
    #[serde(default = "default_max_per_hour")]
    pub max_internal_tasks_per_hour: u32,

    /// Cap on internally-generated tasks per UTC day.
    #[serde(default = "default_max_per_day")]
    pub max_internal_tasks_per_day: u32,

    /// Cap on LLM tokens spent on autonomy per UTC day (best-effort estimate).
    #[serde(default = "default_max_tokens_per_day")]
    pub max_tokens_per_day: u64,

    /// If true, internal tasks go into a proposal queue instead of executing.
    /// DEFAULT TRUE — user approves each proposal via the Life UI.
    #[serde(default = "bool_true")]
    pub propose_only: bool,

    /// Tool allowlist for internal tasks. Defaults are read-only.
    /// Users opt-in to more via config.
    #[serde(default = "default_allowed_tools")]
    pub allowed_tools: Vec<String>,

    /// Auto-run memory gardener (pruning + summarization) daily.
    #[serde(default = "bool_true")]
    pub memory_gardener: bool,

    /// Cron expression for the gardener. Default 06:00 local daily.
    #[serde(default = "default_gardener_cron")]
    pub memory_gardener_cron: String,

    /// Enable post-task reflections (Phase A).
    #[serde(default = "bool_true")]
    pub reflection: bool,

    /// Curiosity probability per tick (0-1). Higher = more exploration.
    #[serde(default = "default_curiosity")]
    pub curiosity: f32,

    /// Enable pattern-driven goal generation (Phase F).
    #[serde(default = "bool_true")]
    pub goal_generation: bool,
}

impl Default for AutonomyConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            tick_interval_secs: default_tick_secs(),
            max_internal_tasks_per_hour: default_max_per_hour(),
            max_internal_tasks_per_day: default_max_per_day(),
            max_tokens_per_day: default_max_tokens_per_day(),
            propose_only: true,
            allowed_tools: default_allowed_tools(),
            memory_gardener: true,
            memory_gardener_cron: default_gardener_cron(),
            reflection: true,
            curiosity: default_curiosity(),
            goal_generation: true,
        }
    }
}

fn default_tick_secs() -> u64 { 300 }           // 5 min
fn default_max_per_hour() -> u32 { 4 }
fn default_max_per_day() -> u32 { 20 }
fn default_max_tokens_per_day() -> u64 { 50_000 }
fn default_gardener_cron() -> String { "0 6 * * *".to_string() }
fn default_curiosity() -> f32 { 0.10 }
fn default_allowed_tools() -> Vec<String> {
    vec![
        "screenshot".to_string(),
        "ui_inspect".to_string(),
        "memory".to_string(),
        "skills".to_string(),
        "filesystem".to_string(),
    ]
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpConfig {
    pub name: String,
    #[serde(default)]
    pub command: Option<String>,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub env: std::collections::HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScheduleEntry {
    pub name: String,
    pub cron: String,
    pub task: String,
    #[serde(default = "default_schedule_notify")]
    pub notify: String,
    #[serde(default = "bool_true")]
    pub enabled: bool,
}

fn default_schedule_notify() -> String { "log".to_string() }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TelegramConfig {
    pub bot_token: String,
    #[serde(default)]
    pub allowed_users: Vec<i64>,
    #[serde(default = "bool_true")]
    pub send_screenshots: bool,
    #[serde(default = "default_telegram_max_message")]
    pub max_message_length: usize,
}

fn default_telegram_max_message() -> usize { 4000 }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentConfig {
    #[serde(default = "default_max_iterations")]
    pub max_iterations: usize,
    #[serde(default = "default_context_window")]
    pub context_window: usize,
    #[serde(default = "default_history_share")]
    pub history_share: f32,
    #[serde(default = "default_data_dir")]
    pub data_dir: PathBuf,
    #[serde(default = "default_log_level")]
    pub log_level: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderConfig {
    // Cloud providers — each optional. Add more by adding a field here and a
    // matching case in runtime::build_provider_by_name.
    pub anthropic:  Option<ProviderEntry>,
    pub openrouter: Option<ProviderEntry>,
    pub openai:     Option<ProviderEntry>,
    pub groq:       Option<ProviderEntry>,
    pub deepseek:   Option<ProviderEntry>,
    pub mistral:    Option<ProviderEntry>,
    pub together:   Option<ProviderEntry>,
    pub local: Option<ProviderEntry>,
    /// Embedded local brain — GGUF model loaded in-process via candle
    pub embedded: Option<EmbeddedProviderEntry>,
    #[serde(default = "default_priority")]
    pub priority: Vec<String>,
    /// Dual-brain config. Accepts:
    ///   `"local:anthropic"`              single cloud
    ///   `"local:anthropic,openrouter"`   cloud chain — first available wins,
    ///                                    later entries are fallback on error
    ///   `"local"`                        local-only
    ///   `"anthropic"` or `"a,b,c"`       cloud-only, with or without fallback
    #[serde(default)]
    pub brain: Option<String>,
    /// Catch-all for user-named custom providers (e.g. `[provider.xiaomi]`,
    /// `[provider.custom]`, `[provider.ollama]`). Without `#[serde(flatten)]`
    /// here the strongly-typed parse would silently drop these sections,
    /// and runtime::build_provider_by_name would then see an empty entry
    /// and refuse to build the brain. Deserialised as arbitrary JSON so
    /// new provider shapes don't require recompiling.
    #[serde(flatten, default)]
    pub extra: std::collections::HashMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderEntry {
    pub api_key: Option<String>,
    pub model: String,
    #[serde(default)]
    pub base_url: Option<String>,
    #[serde(default = "default_max_tokens")]
    pub max_tokens: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmbeddedProviderEntry {
    /// Path to GGUF model file
    pub model: String,
    /// Path to tokenizer.json (optional — will look next to model file)
    #[serde(default)]
    pub tokenizer: Option<String>,
    /// HuggingFace model ID for auto-downloading tokenizer
    #[serde(default)]
    pub hf_model_id: Option<String>,
    #[serde(default = "default_embedded_ctx")]
    pub context_window: u32,
    #[serde(default = "default_embedded_temp")]
    pub temperature: f32,
    #[serde(default = "default_embedded_top_p")]
    pub top_p: f32,
    #[serde(default = "default_embedded_repeat_penalty")]
    pub repeat_penalty: f32,
    #[serde(default = "default_embedded_max_tokens")]
    pub max_tokens: u32,
    #[serde(default = "default_embedded_threads")]
    pub threads: u32,
}

fn default_embedded_ctx() -> u32 { 2048 }
fn default_embedded_temp() -> f32 { 0.7 }
fn default_embedded_top_p() -> f32 { 0.9 }
fn default_embedded_repeat_penalty() -> f32 { 1.1 }
fn default_embedded_max_tokens() -> u32 { 512 }
fn default_embedded_threads() -> u32 { 4 }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolsConfig {
    #[serde(default = "bool_true")]
    pub screenshot: bool,
    #[serde(default = "bool_true")]
    pub touch: bool,
    #[serde(default = "bool_true")]
    pub key_event: bool,
    #[serde(default = "bool_true")]
    pub text_input: bool,
    #[serde(default = "bool_true")]
    pub sms: bool,
    #[serde(default = "bool_true")]
    pub call: bool,
    #[serde(default = "bool_true")]
    pub ui_dump: bool,
    #[serde(default = "bool_true")]
    pub notification: bool,
    #[serde(default = "bool_true")]
    pub filesystem: bool,
    #[serde(default = "bool_true")]
    pub shell: bool,
    #[serde(default)]
    pub filesystem_config: FilesystemConfig,
    #[serde(default)]
    pub shell_config: ShellConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FilesystemConfig {
    #[serde(default = "default_allowed_paths")]
    pub allowed_paths: Vec<PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShellConfig {
    #[serde(default = "default_shell_timeout")]
    pub timeout_seconds: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HardwareConfig {
    pub touchscreen_device: Option<String>,
    pub framebuffer_device: Option<String>,
    pub modem_device: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StartupConfig {
    pub task: Option<String>,
}

impl PekoConfig {
    pub fn load(path: &Path) -> Result<Self, ConfigError> {
        let content = std::fs::read_to_string(path)?;
        let mut config: PekoConfig = toml::from_str(&content)?;
        config.apply_env_overrides();
        Ok(config)
    }

    pub fn from_str(s: &str) -> Result<Self, ConfigError> {
        let mut config: PekoConfig = toml::from_str(s)?;
        config.apply_env_overrides();
        Ok(config)
    }

    fn apply_env_overrides(&mut self) {
        if let Ok(val) = std::env::var("PEKO_API_KEY") {
            if let Some(ref mut p) = self.provider.anthropic {
                p.api_key = Some(val);
            }
        }
        if let Ok(val) = std::env::var("PEKO_MODEL") {
            if let Some(ref mut p) = self.provider.anthropic {
                p.model = val;
            }
        }
        if let Ok(val) = std::env::var("PEKO_MAX_ITERATIONS") {
            if let Ok(n) = val.parse() {
                self.agent.max_iterations = n;
            }
        }
        if let Ok(val) = std::env::var("PEKO_LOG_LEVEL") {
            self.agent.log_level = val;
        }
        if let Ok(val) = std::env::var("PEKO_DATA_DIR") {
            self.agent.data_dir = PathBuf::from(val);
        }
    }
}

impl Default for ToolsConfig {
    fn default() -> Self {
        Self {
            screenshot: true,
            touch: true,
            key_event: true,
            text_input: true,
            sms: true,
            call: true,
            ui_dump: true,
            notification: true,
            filesystem: true,
            shell: true,
            filesystem_config: FilesystemConfig::default(),
            shell_config: ShellConfig::default(),
        }
    }
}

impl Default for FilesystemConfig {
    fn default() -> Self {
        Self {
            allowed_paths: default_allowed_paths(),
        }
    }
}

impl Default for ShellConfig {
    fn default() -> Self {
        Self {
            timeout_seconds: default_shell_timeout(),
        }
    }
}

fn default_max_iterations() -> usize { 50 }
fn default_context_window() -> usize { 200_000 }
fn default_history_share() -> f32 { 0.7 }
fn default_data_dir() -> PathBuf { PathBuf::from("/data/peko") }
fn default_log_level() -> String { "info".to_string() }
fn default_priority() -> Vec<String> { vec!["anthropic".to_string()] }
fn default_max_tokens() -> usize { 8192 }
fn default_allowed_paths() -> Vec<PathBuf> {
    vec![PathBuf::from("/data/peko"), PathBuf::from("/sdcard")]
}
fn default_shell_timeout() -> u64 { 30 }
fn bool_true() -> bool { true }

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_minimal_config() {
        let toml = r#"
[agent]
max_iterations = 10

[provider]
priority = ["anthropic"]

[provider.anthropic]
model = "claude-sonnet-4-20250514"
"#;
        let config = PekoConfig::from_str(toml).unwrap();
        assert_eq!(config.agent.max_iterations, 10);
        assert_eq!(config.agent.context_window, 200_000);
        assert_eq!(config.provider.anthropic.unwrap().model, "claude-sonnet-4-20250514");
    }

    #[test]
    fn test_defaults() {
        let toml = r#"
[agent]

[provider]

[provider.anthropic]
model = "claude-sonnet-4-20250514"
"#;
        let config = PekoConfig::from_str(toml).unwrap();
        assert_eq!(config.agent.max_iterations, 50);
        assert_eq!(config.agent.history_share, 0.7);
        assert!(config.tools.screenshot);
        assert!(config.tools.shell);
    }
}
