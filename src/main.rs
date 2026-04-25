mod web;
mod telegram;

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Context;
use tokio::sync::Mutex;
use tracing::{info, warn};

use peko_config::PekoConfig;
use peko_core::{ToolRegistry, MemoryStore, SkillStore, SystemPrompt, Scheduler, ScheduledTask, TelegramSender, UserModel, McpServerConfig, register_mcp_tools, TaskQueue, DualBrain, Reflector, CallStore, spawn_call_pipeline};
use peko_core::runtime::{build_dual_brain, build_provider_helper};
use peko_llm::{EmbeddedProvider, LlmEngineConfig};
use peko_hal::{InputDevice, SerialModem, UInputDevice};
use peko_tools_android::{
    AudioTool, CallTool, DrawTool, FileSystemTool, KeyEventTool, MemoryTool, PackageManagerTool,
    ScreenshotTool, DelegateTool, SensorsTool, ShellTool, SkillsTool, SmsTool, TextInputTool,
    TouchTool, UiAutomationTool, WifiTool,
};

fn register_tools(config: &PekoConfig) -> ToolRegistry {
    let mut registry = ToolRegistry::new();

    // Screenshot tool — backend selection (DRM/fbdev/screencap) is now
    // resolved per-call inside the tool. Hardware overrides in config
    // still apply via the `screenshot { mode = "fb" }` arg the LLM passes.
    if config.tools.screenshot {
        registry.register(ScreenshotTool::new());
    }

    // Touch tool — direct /dev/input evdev write
    if config.tools.touch {
        let touch_result = match config.hardware.as_ref().and_then(|h| h.touchscreen_device.as_deref()) {
            Some(path) => InputDevice::open(std::path::Path::new(path)),
            None => InputDevice::find_touchscreen(),
        };
        match touch_result {
            Ok(dev) => registry.register(TouchTool::new(dev)),
            Err(e) => warn!(error = %e, "touchscreen device not available, touch tool disabled"),
        }
    }

    // Key event tool — direct /dev/input evdev write
    if config.tools.key_event {
        let key_result = match config.hardware.as_ref().and_then(|h| h.touchscreen_device.as_deref()) {
            Some(path) => InputDevice::open(std::path::Path::new(path)),
            None => InputDevice::find_touchscreen(),
        };
        match key_result {
            Ok(dev) => registry.register(KeyEventTool::new(dev)),
            Err(e) => warn!(error = %e, "input device not available, key_event tool disabled"),
        }
    }

    // Text input tool — synthetic key injection via /dev/uinput
    if config.tools.text_input {
        match UInputDevice::create_touchscreen("peko-keyboard", 1080, 1920) {
            Ok(dev) => registry.register(TextInputTool::new(dev)),
            Err(e) => warn!(error = %e, "uinput device not available, text_input tool disabled"),
        }
    }

    // Resolve modem path once — explicit config wins, else cached, else probe.
    // Handles fajita/fog/most Qualcomm layouts automatically (/dev/smd11 etc).
    let modem_path = peko_core::resolve_modem(
        config.hardware.as_ref().and_then(|h| h.modem_device.as_deref()),
        &config.agent.data_dir,
    );

    // Seed the lockscreen PIN (if configured) so ensure_awake() can
    // auto-unlock after waking the display. Updated live by the
    // /api/config POST handler when the user changes it in the UI.
    peko_tools_android::screen_state::set_lock_pin(
        config.security.as_ref().and_then(|s| s.lock_pin.clone())
    );

    // Unlock tool — one-shot "wake + dismiss + PIN" that the agent
    // reaches for when the user asks to unlock / log in. Always available
    // (it only uses shell primitives + the PIN static).
    registry.register(peko_tools_android::UnlockDeviceTool::new());

    // SMS tool — register one of two backends under the same tool name
    // ("sms") based on config.tools.sms_config.backend:
    //   - "framework" (default): use the priv-app shim that talks to
    //     SmsManager.sendTextMessage(). Works on every modern Android
    //     device with a SIM. No hardware access needed.
    //   - "modem": legacy AT-over-serial. Requires an accessible modem
    //     path (/dev/smd*, /dev/ttyUSB*). Fails on OnePlus / Pixel /
    //     Samsung because RILD owns the AT channel.
    //   - "off": never register; tool absent from the agent's manifest.
    if config.tools.sms {
        let backend = config.tools.sms_config.backend.as_str();
        match backend {
            "off" => {
                info!("SMS tool disabled by config (sms_config.backend = \"off\")");
            }
            "modem" => {
                if let Some(ref path) = modem_path {
                    match SerialModem::open(path) {
                        Ok(modem) => {
                            info!(path = %path.display(), "SMS tool: modem (AT) backend");
                            registry.register(SmsTool::new(modem));
                        }
                        Err(e) => warn!(error = %e, path = %path.display(),
                            "modem open failed, sms tool disabled"),
                    }
                } else {
                    warn!("SMS backend=modem but no modem_device resolved; tool disabled");
                }
            }
            _ => {
                // default / "framework"
                info!("SMS tool: framework (priv-app shim) backend");
                registry.register(peko_tools_android::SmsFrameworkTool::new(
                    config.tools.sms_config.clone(),
                ));
            }
        }
    }

    // Call tool — prefer the framework path (am start ACTION_CALL,
    // works on every modern Android with a Dialer and a SIM) over the
    // AT-over-serial path, which requires modem access RILD denies us.
    // Symmetric to the SMS backend switch above; modem fallback lives
    // here for the rare rooted dev boards where /dev/smd* actually
    // talks back.
    if config.tools.call {
        // Borrow the SMS backend setting — one "framework" / "modem" /
        // "off" knob controls both so the config stays tidy.
        let backend = config.tools.sms_config.backend.as_str();
        match backend {
            "off" => {
                info!("Call tool disabled by config (sms_config.backend = \"off\")");
            }
            "modem" => {
                if let Some(ref path) = modem_path {
                    match SerialModem::open(path) {
                        Ok(modem) => {
                            info!(path = %path.display(), "Call tool: modem (AT) backend");
                            registry.register(CallTool::new(modem));
                        }
                        Err(e) => warn!(error = %e, path = %path.display(),
                            "modem open failed, call tool disabled"),
                    }
                } else {
                    warn!("Call backend=modem but no modem_device resolved; tool disabled");
                }
            }
            _ => {
                info!("Call tool: framework (am start ACTION_CALL) backend");
                registry.register(peko_tools_android::CallFrameworkTool::new());
            }
        }
    }

    // Shell tool — direct on-device sh execution
    if config.tools.shell {
        registry.register(ShellTool::new(config.tools.shell_config.timeout_seconds));
    }

    // Filesystem tool — direct POSIX file I/O
    if config.tools.filesystem {
        registry.register(FileSystemTool::new(config.tools.filesystem_config.allowed_paths.clone()));
    }

    // UI automation — uiautomator / screencap (hybrid mode)
    if config.tools.ui_dump {
        registry.register(UiAutomationTool::new());
    }

    // Package manager — direct pm/am/installd
    registry.register(PackageManagerTool::new());

    // Sensors — direct IIO + power_supply + input subsystem reads.
    // No config flag yet; sensors are read-only and harmless. Add a
    // [tools] sensors = false override later if a user wants to disable.
    registry.register(SensorsTool::new());

    // Wi-Fi — cmd wifi (Lane B) → wpa_supplicant ctrl socket (Lane A).
    // Read paths (status/scan/list_networks) are harmless. Write paths
    // (connect/disconnect/enable/disable) gate at the agent level via
    // user approval rather than a config flag.
    registry.register(WifiTool::new());

    // Audio — ALSA topology + tinymix + media volume. PCM record/play
    // are deferred to Phase 5 (overlay APK shim).
    registry.register(AudioTool::new());

    // Draw — pure-Rust 2D renderer for Lane A status overlays.
    // Returns a PNG; in Lane A this can be blitted to /dev/graphics/fb0
    // by a future blit step.
    registry.register(DrawTool::new());

    registry
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let config_path = args.iter()
        .position(|a| a == "--config")
        .and_then(|i| args.get(i + 1))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("config/config.toml"));

    let port: u16 = args.iter()
        .position(|a| a == "--port")
        .and_then(|i| args.get(i + 1))
        .and_then(|p| p.parse().ok())
        .unwrap_or(8080);

    let config = PekoConfig::load(&config_path)
        .context(format!("failed to load config from {}", config_path.display()))?;

    tracing_subscriber::fmt()
        .with_env_filter(&config.agent.log_level)
        .with_target(false)
        .init();

    info!(config = %config_path.display(), "peko-agent starting");

    let mut registry = register_tools(&config);
    let db_path = config.agent.data_dir.join("state.db");
    let memory_path = config.agent.data_dir.join("memory.db");
    std::fs::create_dir_all(&config.agent.data_dir)?;

    // Memory system
    let memory_store = Arc::new(Mutex::new(
        MemoryStore::open(&memory_path)
            .context("failed to open memory database")?
    ));
    registry.register(MemoryTool::new(memory_store.clone()));
    info!("memory system initialized");

    // Skills system
    let skills_path = config.agent.data_dir.join("skills");
    let skill_store = Arc::new(Mutex::new(
        SkillStore::open(&skills_path)
            .context("failed to open skills directory")?
    ));
    registry.register(SkillsTool::new(skill_store.clone()));
    info!("skills system initialized");

    // Load SOUL.md personality
    let system_prompt = SystemPrompt::load_from_dir(&config.agent.data_dir);

    // User model
    let user_model_path = config.agent.data_dir.join("user_model.json");
    let user_model = Arc::new(Mutex::new(UserModel::load(&user_model_path)));
    info!("user model loaded");

    // Motivation drives (Phase D) — persisted between runs, default 0.5 each.
    let motivation_path = config.agent.data_dir.join("motivation.json");
    let motivation = Arc::new(Mutex::new(
        peko_core::Motivation::load(&motivation_path)
    ));
    info!("motivation state loaded");

    // MCP servers — connect and register tools
    if !config.mcp.is_empty() {
        let mcp_configs: Vec<McpServerConfig> = config.mcp.iter().map(|c| McpServerConfig {
            name: c.name.clone(),
            command: c.command.clone(),
            args: c.args.clone(),
            url: c.url.clone(),
            env: c.env.clone(),
        }).collect();
        let _mcp_clients = register_mcp_tools(&mcp_configs, &mut registry).await;
        info!(servers = config.mcp.len(), "MCP integration initialized");
    }

    let config_json = serde_json::to_value(&config)?;

    // Brain router: three modes, selected by `provider.brain` string
    //   "local:anthropic"       → Dual (classify + route + escalate tool)
    //   "local"  / "embedded"   → LocalOnly (no routing, no escalate)
    //   "anthropic" / "openrouter" → CloudOnly
    //
    // Two backend paths:
    //   "embedded..."           → load GGUF model in-process via candle
    //   anything else           → UDS/HTTP provider via build_dual_brain()
    let brain: Option<Arc<DualBrain>> = {
        let brain_str = config_json["provider"]["brain"].as_str().unwrap_or("");
        if brain_str.is_empty() {
            None
        } else if brain_str == "embedded" || brain_str.starts_with("embedded:") {
            // In-process candle GGUF path.
            let cloud_name: Option<&str> = brain_str.strip_prefix("embedded:")
                .filter(|s| !s.is_empty());
            match build_embedded_brain(&config_json, cloud_name) {
                Ok(b) => {
                    info!(
                        mode = %b.mode(),
                        local = b.local_model_name(),
                        cloud = b.cloud_model_name(),
                        "brain EMBEDDED: GGUF model loaded in-process"
                    );
                    Some(Arc::new(b))
                }
                Err(e) => {
                    warn!(error = %e, "embedded brain failed to load");
                    None
                }
            }
        } else {
            // UDS/HTTP path covers all three modes based on brain_str parsing.
            build_dual_brain(&config_json).map(|b| {
                info!(
                    mode = %b.mode(),
                    local = b.local_model_name(),
                    cloud = b.cloud_model_name(),
                    "brain enabled"
                );
                Arc::new(b)
            })
        }
    };

    let config_arc = Arc::new(Mutex::new(config_json));
    let soul_arc = Arc::new(Mutex::new(system_prompt.soul_text().to_string()));

    let tools_arc = Arc::new(registry);

    // Reflector (Phase A) — only wired when autonomy.reflection is on.
    // Uses a provider built from config. Reflection runs in the background
    // per task; failures are logged at warn and don't affect the user.
    let reflector: Option<Arc<Reflector>> = if config.autonomy.reflection {
        let cfg_val = config_arc.lock().await.clone();
        match build_provider_helper(&cfg_val) {
            Ok(p) => {
                let provider_arc: Arc<dyn peko_transport::LlmProvider> = Arc::from(p);
                Some(Arc::new(Reflector::new(provider_arc, memory_store.clone())))
            }
            Err(e) => {
                warn!(error = %e, "reflector: no provider available, disabling reflection");
                None
            }
        }
    } else {
        info!("autonomy.reflection=false — reflector disabled");
        None
    };

    // Create the centralized task queue — all callers submit here
    let task_queue = TaskQueue::new(
        tools_arc.clone(),
        config_arc.clone(),
        db_path.clone(),
        memory_store.clone(),
        skill_store.clone(),
        soul_arc.clone(),
        user_model.clone(),
        user_model_path.clone(),
        brain.clone(),
        reflector.clone(),
        Some(motivation.clone()),
        Some(motivation_path.clone()),
        32, // max queue size
    );

    // Memory gardener (Phase C) — daily prune + decay pass on the memory store.
    // Independent of the life loop; runs whether autonomy is enabled or not,
    // as long as autonomy.memory_gardener is true.
    if config.autonomy.memory_gardener {
        let gcfg = peko_core::GardenerConfig {
            cron: config.autonomy.memory_gardener_cron.clone(),
            ..Default::default()
        };
        let _ = peko_core::spawn_gardener(memory_store.clone(), gcfg);
        info!(cron = %config.autonomy.memory_gardener_cron, "memory gardener started");
    } else {
        info!("autonomy.memory_gardener=false — gardener disabled");
    }

    // Voice-call pipeline — always spawned, always has a CallStore.
    // The watcher re-reads `[calls]` from the live config each tick,
    // so toggling `enabled` from the Config UI takes effect on the
    // next poll (~10s) without a daemon restart. When disabled it
    // idles. Store is opened unconditionally so `/api/calls` can
    // surface historical records even while the pipeline is off.
    let call_store_arc: Option<Arc<Mutex<CallStore>>> = {
        let path = config.agent.data_dir.join("calls.db");
        match CallStore::open(&path) {
            Ok(store) => {
                let store_arc = Arc::new(Mutex::new(store));
                let cfg_val = config_arc.lock().await.clone();
                let provider_arc: Option<Arc<dyn peko_transport::LlmProvider>> =
                    build_provider_helper(&cfg_val).ok()
                        .map(|p| Arc::from(p) as Arc<dyn peko_transport::LlmProvider>);
                let _handle = spawn_call_pipeline(
                    config_arc.clone(),
                    store_arc.clone(),
                    memory_store.clone(),
                    provider_arc,
                );
                info!(db = %path.display(), enabled = config.calls.enabled,
                      "call pipeline spawned (hot-reloads [calls] from live config)");
                Some(store_arc)
            }
            Err(e) => {
                warn!(error = %e, "call store open failed, /api/calls will return empty");
                None
            }
        }
    };

    // Life loop (Phase B) — spawned only when autonomy is enabled.
    let life_loop_handle = {
        let life = peko_core::LifeLoop::new(
            config.autonomy.clone(),
            motivation.clone(),
            motivation_path.clone(),
            memory_store.clone(),
            user_model.clone(),
            tools_arc.clone(),
            task_queue.clone(),
        );
        life.spawn()
    };

    let mut app_state = web::api::AppState {
        tools: tools_arc.clone(),
        config: config_arc.clone(),
        config_path: config_path.clone(),
        session_db_path: db_path.clone(),
        memory: memory_store.clone(),
        skills: skill_store.clone(),
        soul: soul_arc.clone(),
        user_model: user_model.clone(),
        user_model_path: user_model_path.clone(),
        task_queue: task_queue.clone(),
        brain: brain.clone(),
        motivation: motivation.clone(),
        motivation_path: motivation_path.clone(),
        autonomy: config.autonomy.clone(),
        life_loop: life_loop_handle,
        scheduler_tasks: None,
        calls: call_store_arc,
    };

    // Start Telegram bot if configured
    if let Some(ref tg_config) = config.telegram {
        let bot = telegram::TelegramBot::new(
            tg_config.clone(),
            config_arc.clone(),
            tools_arc.clone(),
            db_path.clone(),
            memory_store.clone(),
            skill_store.clone(),
            soul_arc.clone(),
        );
        tokio::spawn(async move { bot.run().await });
        info!("telegram bot started");
    }

    // Start scheduler if tasks are configured
    if !config.schedule.is_empty() {
        let tasks: Vec<ScheduledTask> = config.schedule.iter().map(|s| {
            ScheduledTask {
                name: s.name.clone(),
                cron: s.cron.clone(),
                task: s.task.clone(),
                notify: s.notify.clone(),
                enabled: s.enabled,
                last_run: None,
                run_count: 0,
                last_result: None,
                last_error: None,
            }
        }).collect();

        let tg_sender = config.telegram.as_ref().map(|tg| {
            TelegramSender::new(tg.bot_token.clone(), tg.allowed_users.clone())
        });

        let scheduler = Scheduler::new(
            tasks,
            tools_arc.clone(),
            config_arc.clone(),
            db_path.clone(),
            memory_store.clone(),
            skill_store.clone(),
            soul_arc.clone(),
            tg_sender,
        );

        // Store task handle for API access
        app_state.scheduler_tasks = Some(scheduler.tasks_handle());

        tokio::spawn(async move { scheduler.run().await });
        info!(count = config.schedule.len(), "scheduler started");
    }

    let app = web::api::router(app_state);

    let addr = format!("0.0.0.0:{}", port);
    info!(address = %addr, "web UI available at http://localhost:{}", port);

    let listener = tokio::net::TcpListener::bind(&addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}

/// Build a brain with an embedded GGUF model as the local side.
/// If `cloud_name` is None → LocalOnly mode (no cloud provider required).
/// If `cloud_name` is Some → Dual mode; falls back to LocalOnly if the cloud
/// provider can't be built (e.g. missing API key).
fn build_embedded_brain(
    config: &serde_json::Value,
    cloud_name: Option<&str>,
) -> anyhow::Result<DualBrain> {
    use peko_transport::{AnthropicProvider, OpenAICompatProvider};
    use peko_core::BrainMode;

    let entry = &config["provider"]["embedded"];
    let model_path = entry["model"].as_str()
        .ok_or_else(|| anyhow::anyhow!("provider.embedded.model (GGUF path) is required"))?;

    let engine_config = LlmEngineConfig {
        model_path: PathBuf::from(model_path),
        tokenizer_path: entry["tokenizer"].as_str().map(PathBuf::from),
        hf_model_id: entry["hf_model_id"].as_str().map(String::from),
        context_size: entry["context_window"].as_u64().unwrap_or(2048) as u32,
        temperature: entry["temperature"].as_f64().unwrap_or(0.7) as f32,
        top_p: entry["top_p"].as_f64().unwrap_or(0.9) as f32,
        repeat_penalty: entry["repeat_penalty"].as_f64().unwrap_or(1.1) as f32,
        max_tokens: entry["max_tokens"].as_u64().unwrap_or(1024) as u32,
        model_name: PathBuf::from(model_path)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("embedded")
            .to_string(),
        threads: entry["threads"].as_u64().unwrap_or(4) as u32,
    };

    info!(
        model = %engine_config.model_name,
        path = model_path,
        ctx = engine_config.context_size,
        "loading embedded GGUF model..."
    );

    let engine = peko_llm::load_gguf(engine_config)?;
    let engine = Arc::new(tokio::sync::Mutex::new(engine));

    let build_embedded = || -> Box<dyn peko_transport::LlmProvider> {
        Box::new(
            EmbeddedProvider::new(engine.clone())
                .with_model_name(
                    entry["model"].as_str().unwrap_or("embedded").to_string()
                )
                .with_max_context(entry["context_window"].as_u64().unwrap_or(2048) as usize),
        )
    };

    let local = build_embedded();

    // No cloud requested → LocalOnly mode. We still need a "cloud" slot on the
    // DualBrain struct, so hand it a second instance pointing at the same engine.
    let Some(cloud_name) = cloud_name else {
        info!("brain LOCAL-ONLY (embedded): no cloud provider, no escalation");
        return Ok(DualBrain::new_local_only(local, build_embedded()));
    };

    // Cloud requested — try to build it; fall back to LocalOnly if unavailable.
    let cloud_entry = &config["provider"][cloud_name];
    let cloud_result: Option<Box<dyn peko_transport::LlmProvider>> = if cloud_entry.is_null() {
        warn!(cloud = %cloud_name, "cloud brain not configured");
        None
    } else {
        let api_key = cloud_entry["api_key"].as_str().unwrap_or("").to_string();
        let model = cloud_entry["model"].as_str().unwrap_or("").to_string();
        let base_url = cloud_entry["base_url"].as_str().unwrap_or("").to_string();
        let max_tokens = cloud_entry["max_tokens"].as_u64().unwrap_or(4096) as usize;

        match cloud_name {
            "anthropic" => {
                let key = if api_key.is_empty() {
                    std::env::var("ANTHROPIC_API_KEY").unwrap_or_default()
                } else { api_key };
                if key.is_empty() { None } else {
                    Some(Box::new(AnthropicProvider::new(
                        key, model, max_tokens,
                        if base_url.is_empty() { None } else { Some(base_url) },
                    )))
                }
            }
            "openrouter" => {
                let key = if api_key.is_empty() {
                    std::env::var("OPENROUTER_API_KEY").unwrap_or_default()
                } else { api_key };
                if key.is_empty() { None } else {
                    let url = if base_url.is_empty() { "https://openrouter.ai/api/v1".to_string() } else { base_url };
                    Some(Box::new(OpenAICompatProvider::new(key, model, url, max_tokens)))
                }
            }
            _ => {
                let url = if base_url.is_empty() { "http://localhost:11434/v1".to_string() } else { base_url };
                Some(Box::new(OpenAICompatProvider::new(api_key, model, url, max_tokens)))
            }
        }
    };

    match cloud_result {
        Some(cloud) => Ok(DualBrain::with_mode(BrainMode::Dual, local, cloud)),
        None => {
            warn!(cloud = %cloud_name, "cloud provider unavailable — falling back to LOCAL-ONLY mode");
            Ok(DualBrain::new_local_only(local, build_embedded()))
        }
    }
}
