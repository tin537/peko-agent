mod web;
mod telegram;

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Context;
use tokio::sync::Mutex;
use tracing::{info, warn};

use peko_config::PekoConfig;
use peko_core::{ToolRegistry, MemoryStore, SkillStore, SystemPrompt, Scheduler, ScheduledTask, TelegramSender};
use peko_hal::{FramebufferDevice, InputDevice, SerialModem, UInputDevice};
use peko_tools_android::{
    CallTool, FileSystemTool, KeyEventTool, MemoryTool, PackageManagerTool, ScreenshotTool,
    ShellTool, SkillsTool, SmsTool, TextInputTool, TouchTool, UiAutomationTool,
};

fn register_tools(config: &PekoConfig) -> ToolRegistry {
    let mut registry = ToolRegistry::new();

    // Screenshot tool — direct framebuffer mmap, fallback to screencap
    if config.tools.screenshot {
        match config.hardware.as_ref().and_then(|h| h.framebuffer_device.as_deref()) {
            Some(path) => match FramebufferDevice::open(std::path::Path::new(path)) {
                Ok(fb) => registry.register(ScreenshotTool::new(fb)),
                Err(e) => {
                    warn!(error = %e, "framebuffer open failed, using screencap fallback");
                    registry.register(ScreenshotTool::unavailable());
                }
            },
            None => match FramebufferDevice::open_default() {
                Ok(fb) => registry.register(ScreenshotTool::new(fb)),
                Err(e) => {
                    warn!(error = %e, "default framebuffer not found, using screencap fallback");
                    registry.register(ScreenshotTool::unavailable());
                }
            },
        }
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

    // SMS tool — AT commands via serial modem
    if config.tools.sms {
        if let Some(modem_path) = config.hardware.as_ref().and_then(|h| h.modem_device.as_deref()) {
            match SerialModem::open(std::path::Path::new(modem_path)) {
                Ok(modem) => registry.register(SmsTool::new(modem)),
                Err(e) => warn!(error = %e, "modem not available, sms tool disabled"),
            }
        }
    }

    // Call tool — AT commands via serial modem
    if config.tools.call {
        if let Some(modem_path) = config.hardware.as_ref().and_then(|h| h.modem_device.as_deref()) {
            match SerialModem::open(std::path::Path::new(modem_path)) {
                Ok(modem) => registry.register(CallTool::new(modem)),
                Err(e) => warn!(error = %e, "modem not available, call tool disabled"),
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

    let config_json = serde_json::to_value(&config)?;

    let tools_arc = Arc::new(registry);
    let config_arc = Arc::new(Mutex::new(config_json));
    let soul_arc = Arc::new(Mutex::new(system_prompt.soul_text().to_string()));

    let mut app_state = web::api::AppState {
        tools: tools_arc.clone(),
        config: config_arc.clone(),
        config_path: config_path.clone(),
        session_db_path: db_path.clone(),
        memory: memory_store.clone(),
        skills: skill_store.clone(),
        soul: soul_arc.clone(),
        scheduler_tasks: None,
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
