use std::sync::Arc;
use axum::{
    extract::State,
    http::StatusCode,
    response::{Html, IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use serde::Deserialize;
use tokio::sync::Mutex;
use tracing::{info, error};

use peko_core::{AgentRuntime, SessionStore, ToolRegistry};
use peko_config::AgentConfig;
use peko_transport::{AnthropicProvider, OpenAICompatProvider, ProviderChain, LlmProvider};

use super::ui::INDEX_HTML;

pub type SharedConfig = Arc<Mutex<serde_json::Value>>;

#[derive(Clone)]
pub struct AppState {
    pub tools: Arc<ToolRegistry>,
    pub config: SharedConfig,
    pub config_path: std::path::PathBuf,
    pub session_db_path: std::path::PathBuf,
    pub memory: Arc<Mutex<peko_core::MemoryStore>>,
    pub skills: Arc<Mutex<peko_core::SkillStore>>,
    pub soul: Arc<Mutex<String>>,
    pub user_model: Arc<Mutex<peko_core::UserModel>>,
    pub user_model_path: std::path::PathBuf,
    pub task_queue: peko_core::TaskQueue,
    pub brain: Option<Arc<peko_core::DualBrain>>,
    pub scheduler_tasks: Option<Arc<Mutex<Vec<peko_core::ScheduledTask>>>>,
}

pub fn router(state: AppState) -> Router {
    use super::device;
    Router::new()
        .route("/", get(index))
        .route("/api/status", get(status))
        .route("/api/config", get(get_config).post(set_config))
        .route("/api/sessions", get(get_sessions))
        .route("/api/sessions/{id}", get(get_session_messages).delete(delete_session))
        .route("/api/run", post(run_task))
        // Device
        .route("/api/device/profile", get(device::device_profile))
        .route("/api/device/stats", get(device::device_stats))
        .route("/api/device/logs", get(device::log_stream))
        // Apps
        .route("/api/apps", get(device::list_apps))
        .route("/api/apps/action", post(device::app_action))
        // Messages (SMS + notifications)
        .route("/api/messages/stream", get(device::messages_stream))
        // Memory
        .route("/api/memories", get(list_memories))
        .route("/api/memories/{id}", axum::routing::delete(delete_memory))
        // Skills
        .route("/api/skills", get(list_skills))
        .route("/api/skills/{name}", axum::routing::delete(delete_skill))
        // SOUL.md
        .route("/api/soul", get(get_soul).post(set_soul))
        // Scheduler
        .route("/api/schedule", get(list_schedule))
        // User model
        .route("/api/user", get(get_user_model).post(update_user_model))
        // Queue status
        .route("/api/queue", get(queue_status))
        // Screenshots
        .route("/api/screenshots/{filename}", get(serve_screenshot))
        // AGPL §13 compliance — source offer + third-party licenses
        .route("/source", get(source_offer))
        .route("/licenses", get(third_party_licenses))
        .with_state(state)
}

async fn index() -> Html<&'static str> {
    Html(INDEX_HTML)
}

async fn status(State(state): State<AppState>) -> Json<serde_json::Value> {
    let config = state.config.lock().await;
    let priority = config["provider"]["priority"]
        .as_array()
        .and_then(|a| a.first())
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");

    let model = config["provider"][priority]["model"]
        .as_str()
        .unwrap_or("unknown")
        .to_string();

    let memory = peko_core::MemMonitor::snapshot();

    Json(serde_json::json!({
        "status": "ready",
        "model": model,
        "memory": memory,
    }))
}

async fn get_config(State(state): State<AppState>) -> Json<serde_json::Value> {
    let config = state.config.lock().await;
    let mut safe = config.clone();
    if let Some(provider) = safe.get_mut("provider") {
        for key in ["anthropic", "openrouter", "local"] {
            if let Some(entry) = provider.get_mut(key) {
                if let Some(api_key) = entry.get("api_key") {
                    if let Some(k) = api_key.as_str() {
                        if k.len() > 8 {
                            entry["api_key"] = serde_json::Value::String(
                                format!("{}...{}", &k[..4], &k[k.len()-4..])
                            );
                        }
                    }
                }
            }
        }
    }
    Json(safe)
}

async fn set_config(
    State(state): State<AppState>,
    Json(new_config): Json<serde_json::Value>,
) -> StatusCode {
    let mut config = state.config.lock().await;

    if let Some(agent) = new_config.get("agent") {
        // Validate critical agent parameters
        if let Some(max_iter) = agent.get("max_iterations").and_then(|v| v.as_u64()) {
            if max_iter > 500 {
                return StatusCode::BAD_REQUEST;
            }
        }
        if let Some(ctx) = agent.get("context_window").and_then(|v| v.as_u64()) {
            if ctx == 0 || ctx > 2_000_000 {
                return StatusCode::BAD_REQUEST;
            }
        }
        if let Some(share) = agent.get("history_share").and_then(|v| v.as_f64()) {
            if share <= 0.0 || share > 1.0 {
                return StatusCode::BAD_REQUEST;
            }
        }
        if let Some(data_dir) = agent.get("data_dir").and_then(|v| v.as_str()) {
            // Block obviously dangerous paths
            if data_dir == "/" || data_dir.starts_with("/proc") || data_dir.starts_with("/sys") {
                return StatusCode::BAD_REQUEST;
            }
        }
        config["agent"] = agent.clone();
    }
    if let Some(provider) = new_config.get("provider") {
        if let Some(existing_provider) = config.get("provider").cloned() {
            for key in ["anthropic", "openrouter", "local"] {
                if let Some(new_entry) = provider.get(key) {
                    let new_api_key = new_entry.get("api_key")
                        .and_then(|v| v.as_str())
                        .unwrap_or("");

                    if new_api_key.is_empty() || new_api_key.contains("...") {
                        if let Some(existing_key) = existing_provider
                            .get(key)
                            .and_then(|e| e.get("api_key"))
                        {
                            let mut merged = new_entry.clone();
                            merged["api_key"] = existing_key.clone();
                            config["provider"][key] = merged;
                            continue;
                        }
                    }
                    config["provider"][key] = new_entry.clone();
                }
            }
            if let Some(priority) = provider.get("priority") {
                config["provider"]["priority"] = priority.clone();
            }
        } else {
            config["provider"] = provider.clone();
        }
    }
    if let Some(tools) = new_config.get("tools") {
        config["tools"] = tools.clone();
    }

    // Persist to disk so config survives restarts and updates
    let config_path = state.config_path.clone();
    let config_snapshot = config.clone();
    drop(config); // release lock before I/O

    if let Err(e) = persist_config_to_disk(&config_path, &config_snapshot) {
        tracing::error!(error = %e, "failed to persist config to disk");
        return StatusCode::INTERNAL_SERVER_ERROR;
    }

    info!(path = %config_path.display(), "config saved to disk");
    StatusCode::OK
}

fn persist_config_to_disk(path: &std::path::Path, config: &serde_json::Value) -> anyhow::Result<()> {
    // Convert JSON back to TOML for human-readable config file
    let toml_value: toml::Value = json_to_toml(config)?;
    let toml_string = toml::to_string_pretty(&toml_value)?;

    // Write atomically: write to temp file, then rename
    let tmp_path = path.with_extension("toml.tmp");
    std::fs::write(&tmp_path, &toml_string)?;
    std::fs::rename(&tmp_path, path)?;

    Ok(())
}

fn json_to_toml(json: &serde_json::Value) -> anyhow::Result<toml::Value> {
    let cleaned = strip_nulls(json);
    let json_str = serde_json::to_string(&cleaned)?;
    let toml_val: toml::Value = serde_json::from_str(&json_str)
        .map_err(|e| anyhow::anyhow!("JSON to TOML conversion failed: {}", e))?;
    Ok(toml_val)
}

/// Remove null values recursively — TOML has no null type
fn strip_nulls(val: &serde_json::Value) -> serde_json::Value {
    match val {
        serde_json::Value::Object(map) => {
            let cleaned: serde_json::Map<String, serde_json::Value> = map.iter()
                .filter(|(_, v)| !v.is_null())
                .map(|(k, v)| (k.clone(), strip_nulls(v)))
                .collect();
            serde_json::Value::Object(cleaned)
        }
        serde_json::Value::Array(arr) => {
            serde_json::Value::Array(arr.iter().filter(|v| !v.is_null()).map(strip_nulls).collect())
        }
        other => other.clone(),
    }
}

async fn get_sessions(State(state): State<AppState>) -> Json<serde_json::Value> {
    let store = match SessionStore::open(&state.session_db_path) {
        Ok(s) => s,
        Err(_) => return Json(serde_json::Value::Array(vec![])),
    };
    match store.recent_sessions(20) {
        Ok(sessions) => {
            let list: Vec<serde_json::Value> = sessions.iter().map(|s| {
                serde_json::json!({
                    "id": s.id,
                    "task": s.task,
                    "status": s.status,
                    "iterations": s.iterations,
                    "started_at": s.started_at,
                })
            }).collect();
            Json(serde_json::Value::Array(list))
        }
        Err(_) => Json(serde_json::Value::Array(vec![])),
    }
}

async fn get_session_messages(
    State(state): State<AppState>,
    axum::extract::Path(session_id): axum::extract::Path<String>,
) -> Json<serde_json::Value> {
    let store = match SessionStore::open(&state.session_db_path) {
        Ok(s) => s,
        Err(_) => return Json(serde_json::json!({"error": "cannot open session store"})),
    };
    match store.load_messages(&session_id) {
        Ok(messages) => {
            let list: Vec<serde_json::Value> = messages.iter().map(|m| {
                serde_json::json!({
                    "role": m.role,
                    "content": m.content,
                    "tool_name": m.tool_name,
                    "tool_args": m.tool_args,
                    "is_error": m.is_error,
                    "created_at": m.created_at,
                })
            }).collect();
            Json(serde_json::Value::Array(list))
        }
        Err(_) => Json(serde_json::json!({"error": "failed to load messages"})),
    }
}

async fn delete_session(
    State(state): State<AppState>,
    axum::extract::Path(session_id): axum::extract::Path<String>,
) -> StatusCode {
    let store = match SessionStore::open(&state.session_db_path) {
        Ok(s) => s,
        Err(_) => return StatusCode::INTERNAL_SERVER_ERROR,
    };
    match store.delete_session(&session_id) {
        Ok(()) => {
            info!(session = %session_id, "session deleted");
            StatusCode::OK
        }
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

pub fn build_provider_from_json_pub(config: &serde_json::Value) -> anyhow::Result<Box<dyn LlmProvider>> {
    peko_core::build_provider_helper(config)
}

fn build_provider_from_json(config: &serde_json::Value) -> anyhow::Result<Box<dyn LlmProvider>> {
    let priority = config["provider"]["priority"]
        .as_array()
        .map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect::<Vec<_>>())
        .unwrap_or_else(|| vec!["local".to_string()]);

    let mut providers: Vec<Box<dyn LlmProvider>> = Vec::new();

    for name in &priority {
        let entry = &config["provider"][name.as_str()];
        if entry.is_null() { continue; }

        let api_key = entry["api_key"].as_str().unwrap_or("").to_string();
        let model = entry["model"].as_str().unwrap_or("").to_string();
        let base_url = entry["base_url"].as_str().unwrap_or("").to_string();
        let max_tokens = entry["max_tokens"].as_u64().unwrap_or(4096) as usize;

        if model.is_empty() { continue; }

        match name.as_str() {
            "anthropic" => {
                let key = if api_key.is_empty() {
                    std::env::var("ANTHROPIC_API_KEY").unwrap_or_default()
                } else { api_key };
                if key.is_empty() { continue; }
                providers.push(Box::new(AnthropicProvider::new(
                    key, model, max_tokens,
                    if base_url.is_empty() { None } else { Some(base_url) },
                )));
            }
            "openrouter" => {
                let key = if api_key.is_empty() {
                    std::env::var("OPENROUTER_API_KEY").unwrap_or_default()
                } else { api_key };
                if key.is_empty() { continue; }
                let url = if base_url.is_empty() { "https://openrouter.ai/api/v1".to_string() } else { base_url };
                providers.push(Box::new(OpenAICompatProvider::new(key, model, url, max_tokens)));
            }
            "local" | _ => {
                let url = if base_url.is_empty() { "http://localhost:11434/v1".to_string() } else { base_url };
                providers.push(Box::new(OpenAICompatProvider::new(api_key, model, url, max_tokens)));
            }
        }
    }

    if providers.is_empty() {
        anyhow::bail!("no LLM providers configured");
    }

    if providers.len() == 1 {
        Ok(providers.into_iter().next().unwrap())
    } else {
        Ok(Box::new(ProviderChain::new(providers)))
    }
}

#[derive(Deserialize)]
struct RunRequest {
    input: String,
    #[serde(default)]
    session_id: Option<String>,
}

async fn run_task(
    State(state): State<AppState>,
    Json(req): Json<RunRequest>,
) -> Response {
    if req.input.trim().is_empty() {
        return (StatusCode::BAD_REQUEST, "empty input").into_response();
    }

    info!(task = %req.input, session = ?req.session_id, "web UI task submitted");

    // Submit to the centralized task queue
    let (mut rx, _result_rx) = state.task_queue.submit_and_wait(
        req.input.clone(),
        req.session_id.clone(),
        peko_core::TaskSource::WebUI,
    ).await;

    // Stream channel events as SSE
    let stream = async_stream::stream! {
        yield Ok::<_, std::convert::Infallible>(format!("data: {}\n\n", serde_json::json!({
            "type": "status",
            "message": "starting"
        })));

        while let Some(event) = rx.recv().await {
            let data = match event {
                peko_core::runtime::StreamCallback::TextDelta(text) => {
                    serde_json::json!({"type": "text_delta", "text": text})
                }
                peko_core::runtime::StreamCallback::ToolStart { name } => {
                    serde_json::json!({"type": "tool_start", "name": name})
                }
                peko_core::runtime::StreamCallback::ToolResult { name, content, is_error, image } => {
                    let mut ev = serde_json::json!({"type": "tool_result", "name": name, "content": content, "is_error": is_error});
                    if let Some((data_uri, _)) = image {
                        ev["image"] = serde_json::Value::String(data_uri);
                    }
                    ev
                }
                peko_core::runtime::StreamCallback::Thinking(text) => {
                    serde_json::json!({"type": "thinking", "text": text})
                }
                peko_core::runtime::StreamCallback::Done { iterations, session_id } => {
                    serde_json::json!({"type": "done", "iterations": iterations, "session_id": session_id})
                }
                peko_core::runtime::StreamCallback::Error(msg) => {
                    serde_json::json!({"type": "error", "message": msg})
                }
            };
            yield Ok(format!("data: {}\n\n", data));
        }

        yield Ok("data: [DONE]\n\n".to_string());
    };

    let body = axum::body::Body::from_stream(stream);

    Response::builder()
        .header("content-type", "text/event-stream")
        .header("cache-control", "no-cache")
        .header("connection", "keep-alive")
        .body(body)
        .unwrap()
}

// ── Memory API ──

async fn list_memories(State(state): State<AppState>) -> Json<serde_json::Value> {
    let store = state.memory.lock().await;
    match store.list(100, None) {
        Ok(memories) => {
            let list: Vec<serde_json::Value> = memories.iter().map(|m| {
                serde_json::json!({
                    "id": m.id,
                    "key": m.key,
                    "content": m.content,
                    "category": m.category,
                    "importance": m.importance,
                    "created_at": m.created_at,
                    "access_count": m.access_count,
                })
            }).collect();
            Json(serde_json::json!({
                "count": store.count().unwrap_or(0),
                "memories": list
            }))
        }
        Err(e) => Json(serde_json::json!({"error": format!("{}", e)})),
    }
}

async fn delete_memory(
    State(state): State<AppState>,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> StatusCode {
    let store = state.memory.lock().await;
    match store.delete(&id) {
        Ok(true) => StatusCode::OK,
        _ => StatusCode::NOT_FOUND,
    }
}

// ── Skills API ──

async fn list_skills(State(state): State<AppState>) -> Json<serde_json::Value> {
    let store = state.skills.lock().await;
    let skills: Vec<serde_json::Value> = store.list().iter().map(|s| {
        serde_json::json!({
            "name": s.name,
            "description": s.description,
            "category": s.category,
            "success_count": s.success_count,
            "fail_count": s.fail_count,
            "success_rate": s.success_rate(),
            "tags": s.tags,
            "steps": s.steps,
            "updated_at": s.updated_at,
        })
    }).collect();

    Json(serde_json::json!({
        "count": store.count(),
        "skills": skills
    }))
}

async fn delete_skill(
    State(state): State<AppState>,
    axum::extract::Path(name): axum::extract::Path<String>,
) -> StatusCode {
    let mut store = state.skills.lock().await;
    match store.delete(&name) {
        Ok(true) => StatusCode::OK,
        _ => StatusCode::NOT_FOUND,
    }
}

// ── SOUL.md API ──

async fn get_soul(State(state): State<AppState>) -> Json<serde_json::Value> {
    let soul = state.soul.lock().await;
    Json(serde_json::json!({"soul": *soul}))
}

async fn set_soul(
    State(state): State<AppState>,
    Json(body): Json<serde_json::Value>,
) -> StatusCode {
    if let Some(text) = body["soul"].as_str() {
        let mut soul = state.soul.lock().await;
        *soul = text.to_string();

        // Persist to disk
        let config = state.config.lock().await;
        let data_dir = config["agent"]["data_dir"].as_str().unwrap_or("/data/peko");
        let soul_path = std::path::Path::new(data_dir).join("SOUL.md");
        if let Err(e) = std::fs::write(&soul_path, text) {
            tracing::error!(error = %e, "failed to save SOUL.md");
            return StatusCode::INTERNAL_SERVER_ERROR;
        }
        info!(path = %soul_path.display(), "SOUL.md saved");
        StatusCode::OK
    } else {
        StatusCode::BAD_REQUEST
    }
}

// ── Schedule API ──

async fn list_schedule(State(state): State<AppState>) -> Json<serde_json::Value> {
    if let Some(ref tasks) = state.scheduler_tasks {
        let tasks = tasks.lock().await;
        let list: Vec<serde_json::Value> = tasks.iter().map(|t| {
            serde_json::json!({
                "name": t.name,
                "cron": t.cron,
                "task": t.task,
                "notify": t.notify,
                "enabled": t.enabled,
                "last_run": t.last_run,
                "run_count": t.run_count,
                "last_result": t.last_result.as_ref().map(|r| if r.len() > 200 { format!("{}...", &r[..200]) } else { r.clone() }),
                "last_error": t.last_error,
            })
        }).collect();
        Json(serde_json::json!({"count": list.len(), "tasks": list}))
    } else {
        Json(serde_json::json!({"count": 0, "tasks": [], "note": "No scheduled tasks configured. Add [[schedule]] to config.toml"}))
    }
}

// ── User Model API ──

async fn get_user_model(State(state): State<AppState>) -> Json<serde_json::Value> {
    let model = state.user_model.lock().await;
    Json(serde_json::to_value(&*model).unwrap_or(serde_json::json!({})))
}

async fn update_user_model(
    State(state): State<AppState>,
    Json(updates): Json<serde_json::Value>,
) -> StatusCode {
    let mut model = state.user_model.lock().await;

    // Apply updates
    if let Some(name) = updates["name"].as_str() {
        model.set_preference("name", name);
    }
    if let Some(expertise) = updates["expertise"].as_str() {
        model.set_preference("expertise", expertise);
    }
    if let Some(style) = updates["response_style"].as_str() {
        model.set_preference("response_style", style);
    }
    if let Some(v) = updates["verbose"].as_bool() {
        model.set_preference("verbose", if v { "true" } else { "false" });
    }
    if let Some(v) = updates["confirm_dangerous"].as_bool() {
        model.set_preference("confirm_dangerous", if v { "true" } else { "false" });
    }
    if let Some(obs) = updates["observation"].as_str() {
        model.add_observation(obs);
    }
    if let Some(app) = updates["preferred_app"].as_str() {
        if !model.patterns.preferred_apps.contains(&app.to_string()) {
            model.patterns.preferred_apps.push(app.to_string());
        }
    }

    // Save to disk
    if let Err(e) = model.save(&state.user_model_path) {
        tracing::error!(error = %e, "failed to save user model");
        return StatusCode::INTERNAL_SERVER_ERROR;
    }

    info!("user model updated");
    StatusCode::OK
}

// ── Queue Status API ──

async fn queue_status(State(state): State<AppState>) -> Json<serde_json::Value> {
    let status = state.task_queue.status().await;
    Json(serde_json::to_value(&status).unwrap_or(serde_json::json!({})))
}

// ── Screenshot serving ──

async fn serve_screenshot(
    axum::extract::Path(filename): axum::extract::Path<String>,
) -> Response {
    // Strict filename validation: only allow alphanumeric, underscores, hyphens, dots
    // Must end with .jpg, .jpeg, or .png
    let is_safe = filename.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.')
        && !filename.contains("..")
        && (filename.ends_with(".jpg") || filename.ends_with(".jpeg") || filename.ends_with(".png"));

    if !is_safe || filename.is_empty() {
        return (StatusCode::BAD_REQUEST, "invalid filename").into_response();
    }

    // Try allowed directories only
    let allowed_dirs = [
        std::path::PathBuf::from("/data/peko/screenshots"),
        std::path::PathBuf::from("/tmp"),
    ];

    for dir in &allowed_dirs {
        let path = dir.join(&filename);
        // Verify resolved path stays within allowed directory
        if let Ok(canonical) = std::fs::canonicalize(&path) {
            if !canonical.starts_with(dir) {
                continue; // symlink escape attempt
            }
            if let Ok(data) = std::fs::read(&canonical) {
                let content_type = if filename.ends_with(".jpg") || filename.ends_with(".jpeg") {
                    "image/jpeg"
                } else {
                    "image/png"
                };

                return Response::builder()
                    .header("content-type", content_type)
                    .header("cache-control", "public, max-age=3600")
                    .body(axum::body::Body::from(data))
                    .unwrap();
            }
        }
    }

    (StatusCode::NOT_FOUND, "screenshot not found").into_response()
}


// ── AGPL §13 compliance ─────────────────────────────────────────

// When peko-agent is deployed as a network-facing service, AGPL §13 requires
// offering the Corresponding Source Code to every user interacting with it
// remotely. These two endpoints satisfy that obligation — the web UI footer
// links to /source and /licenses.

/// Source offer: redirect to the canonical upstream repo. Fork operators should
/// override this to point at their modified-version source.
async fn source_offer() -> Response {
    let html = r#"<!doctype html>
<html><head><meta charset="utf-8"><title>Source — Peko Agent</title>
<meta name="viewport" content="width=device-width,initial-scale=1">
<style>body{font-family:system-ui,sans-serif;max-width:640px;margin:40px auto;padding:0 20px;line-height:1.6;color:#222}a{color:#4338ca}code{background:#f3f4f6;padding:2px 6px;border-radius:3px}</style>
</head><body>
<h1>Corresponding Source</h1>
<p>Peko Agent is distributed under <a href="https://www.gnu.org/licenses/agpl-3.0.html">AGPL-3.0-or-later</a>. Under AGPL §13, the operator of this service must offer the Corresponding Source Code to every user who interacts with it remotely.</p>
<h2>Unmodified upstream source</h2>
<p><a href="https://github.com/ftmstars/peko-agent">https://github.com/ftmstars/peko-agent</a></p>
<h2>If this is a modified version</h2>
<p>The operator of this deployment is responsible for providing the exact source of the running version. If the link above does not correspond to the running code, please contact the operator of this service for the modified source tree.</p>
<h2>Build instructions</h2>
<p>See <code>README.md</code> in the source tree for build and deployment instructions.</p>
<h2>Third-party licenses</h2>
<p>This binary includes third-party code under compatible licenses (MIT). See <a href="/licenses">/licenses</a>.</p>
<p><a href="/">← back to agent</a></p>
</body></html>"#;
    Html(html).into_response()
}

/// Third-party license notices (covers the MIT-licensed code statically linked
/// into peko-llm-daemon and the Rust dependencies).
async fn third_party_licenses() -> Response {
    let html = r#"<!doctype html>
<html><head><meta charset="utf-8"><title>Third-Party Licenses — Peko Agent</title>
<meta name="viewport" content="width=device-width,initial-scale=1">
<style>body{font-family:system-ui,sans-serif;max-width:760px;margin:40px auto;padding:0 20px;line-height:1.6;color:#222}a{color:#4338ca}pre{background:#f3f4f6;padding:12px;border-radius:4px;overflow:auto;font-size:12px;line-height:1.5}h2{margin-top:2em}</style>
</head><body>
<h1>Third-Party Licenses</h1>
<p>Peko Agent incorporates the following third-party software:</p>
<ul>
<li><b>cpp-httplib</b> v0.18.7 — MIT — © 2025 Yuji Hirose — <a href="https://github.com/yhirose/cpp-httplib">source</a></li>
<li><b>nlohmann/json</b> v3.11.3 — MIT — © 2013-2023 Niels Lohmann — <a href="https://github.com/nlohmann/json">source</a></li>
<li><b>llama.cpp / ggml</b> — MIT — © 2023-2026 The ggml authors — <a href="https://github.com/ggml-org/llama.cpp">source</a> (statically linked into <code>peko-llm-daemon</code>)</li>
<li><b>Rust crates</b> — various (mostly MIT / Apache-2.0) — see <code>Cargo.lock</code></li>
</ul>
<h2>MIT License text (applies to all MIT-licensed components above)</h2>
<pre>Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.</pre>
<p>Full license texts are reproduced in the source repository under <code>third_party/LICENSES/</code>.</p>
<p>Peko Agent itself is licensed under AGPL-3.0-or-later. <a href="/source">Get source</a>.</p>
<p><a href="/">← back to agent</a></p>
</body></html>"#;
    Html(html).into_response()
}
