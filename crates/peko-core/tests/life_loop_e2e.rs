//! End-to-end test for the autonomy stack — Phase B.
//!
//! Flips `autonomy.enabled=true`, sets curiosity above the action threshold,
//! and verifies that within a few heartbeat ticks the life loop produces a
//! proposal. Fills the emulator role described in the Full-Life roadmap
//! without requiring an Android device.
//!
//! The task queue is constructed with a NullProvider, so no LLM call ever
//! fires — we run in `propose_only = true` mode and the proposals are
//! synthesized by the Curiosity generator alone.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::Mutex;

use peko_config::AutonomyConfig;
use peko_core::{
    DriveEvent, LifeLoop, MemoryStore, Motivation, TaskQueue, ToolRegistry, UserModel,
};

fn tmp_dir(label: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("peko_life_e2e_{}_{}", label, std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[tokio::test]
async fn life_loop_emits_proposal_when_enabled() {
    let data_dir = tmp_dir("enabled");

    // Minimal state: empty memory, fresh user model, no tools.
    let memory = Arc::new(Mutex::new(MemoryStore::open_in_memory().unwrap()));
    let skills = Arc::new(Mutex::new(
        peko_core::SkillStore::open(&data_dir.join("skills")).unwrap(),
    ));
    let user_model_path = data_dir.join("user_model.json");
    let user_model = Arc::new(Mutex::new(UserModel::load(&user_model_path)));

    // Motivation — crank curiosity high so suggest_action() returns Explore.
    let motivation_path = data_dir.join("motivation.json");
    let motivation = Arc::new(Mutex::new(Motivation::default()));
    {
        let mut m = motivation.lock().await;
        m.curiosity = 0.95; // above 0.70 threshold
    }

    // Empty registry is fine — we're in propose_only, no tool executes.
    let tools = Arc::new(ToolRegistry::new());
    let config_val = Arc::new(Mutex::new(serde_json::json!({
        "agent": {"max_iterations": 5, "context_window": 4096, "history_share": 0.7, "data_dir": data_dir.to_string_lossy()},
        "provider": {"priority": []}
    })));
    let soul = Arc::new(Mutex::new(String::new()));

    let task_queue = TaskQueue::new(
        tools.clone(),
        config_val,
        data_dir.join("state.db"),
        memory.clone(),
        skills.clone(),
        soul,
        user_model.clone(),
        user_model_path.clone(),
        None,       // no brain
        None,       // no reflector
        Some(motivation.clone()),
        Some(motivation_path.clone()),
        8,
    );

    // Autonomy enabled, very fast tick.
    let autonomy = AutonomyConfig {
        enabled: true,
        tick_interval_secs: 5, // clamped minimum inside LifeLoop::spawn
        max_internal_tasks_per_hour: 10,
        max_internal_tasks_per_day: 20,
        max_tokens_per_day: 50_000,
        propose_only: true,
        allowed_tools: vec![],
        memory_gardener: false,
        memory_gardener_cron: "0 6 * * *".to_string(),
        reflection: false,
        curiosity: 0.5,
        goal_generation: false,
    };

    let life = LifeLoop::new(
        autonomy,
        motivation.clone(),
        motivation_path.clone(),
        memory.clone(),
        user_model.clone(),
        tools.clone(),
        task_queue,
    );
    let handle = life.spawn();

    // Wait up to ~12s for first tick (tick_secs clamped to 5, timing jitter ok).
    let mut proposals = Vec::new();
    for _ in 0..12 {
        tokio::time::sleep(Duration::from_secs(1)).await;
        proposals = handle.list_proposals().await;
        if !proposals.is_empty() { break; }
    }

    assert!(
        !proposals.is_empty(),
        "life loop should have produced at least one proposal after a tick"
    );
    let p = &proposals[0];
    assert_eq!(p.status, peko_core::ProposalStatus::Pending);
    // The curiosity path runs ProposeHelpful or Explore. Either way the
    // reasoning string is non-empty.
    assert!(!p.reasoning.is_empty(), "reasoning should be populated");

    // Token budget should have been charged.
    let snap = handle.snapshot(true).await;
    assert!(snap.tokens_last_day > 0, "token budget should have recorded the cost");
    assert_eq!(snap.tokens_max_per_day, 50_000);

    let _ = std::fs::remove_dir_all(&data_dir);
}

#[tokio::test]
async fn life_loop_respects_disabled_flag() {
    let data_dir = tmp_dir("disabled");

    let memory = Arc::new(Mutex::new(MemoryStore::open_in_memory().unwrap()));
    let skills = Arc::new(Mutex::new(
        peko_core::SkillStore::open(&data_dir.join("skills")).unwrap(),
    ));
    let user_model_path = data_dir.join("user_model.json");
    let user_model = Arc::new(Mutex::new(UserModel::load(&user_model_path)));
    let motivation_path = data_dir.join("motivation.json");
    let motivation = Arc::new(Mutex::new(Motivation::default()));
    {
        let mut m = motivation.lock().await;
        m.curiosity = 0.95;
    }

    let tools = Arc::new(ToolRegistry::new());
    let config_val = Arc::new(Mutex::new(serde_json::json!({"agent": {}, "provider": {}})));
    let soul = Arc::new(Mutex::new(String::new()));

    let task_queue = TaskQueue::new(
        tools.clone(), config_val, data_dir.join("state.db"),
        memory.clone(), skills.clone(), soul,
        user_model.clone(), user_model_path.clone(),
        None, None, Some(motivation.clone()), Some(motivation_path.clone()), 8,
    );

    let autonomy = AutonomyConfig {
        enabled: false, // OFF
        tick_interval_secs: 5,
        max_internal_tasks_per_hour: 10,
        max_internal_tasks_per_day: 20,
        max_tokens_per_day: 50_000,
        propose_only: true,
        allowed_tools: vec![],
        memory_gardener: false,
        memory_gardener_cron: "0 6 * * *".to_string(),
        reflection: false,
        curiosity: 0.5,
        goal_generation: false,
    };

    let life = LifeLoop::new(
        autonomy, motivation, motivation_path,
        memory, user_model, tools, task_queue,
    );
    let handle = life.spawn();

    // Give it a generous window — disabled means it should never fire.
    tokio::time::sleep(Duration::from_secs(7)).await;
    assert!(
        handle.list_proposals().await.is_empty(),
        "disabled life loop must not produce proposals"
    );

    let _ = std::fs::remove_dir_all(&data_dir);
}

#[tokio::test]
async fn motivation_persistence_roundtrip_via_drive_events() {
    // Standalone sanity check for the feedback loop used in the task queue:
    // DriveEvent::TaskSucceeded → save → reload yields same competence.
    let dir = tmp_dir("motivation");
    let path = dir.join("motivation.json");

    let mut m = Motivation::default();
    let baseline = m.competence;
    m.record(DriveEvent::TaskSucceeded);
    assert!(m.competence > baseline);
    m.save(&path).unwrap();

    let reloaded = Motivation::load(&path);
    assert!((reloaded.competence - m.competence).abs() < 1e-6);

    let _ = std::fs::remove_dir_all(&dir);
}
