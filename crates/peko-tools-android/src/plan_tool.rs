//! Plan tool — draft, store, execute multi-step plans.
//!
//! For tasks that need 5+ tool calls or several subgoals, the agent
//! drafts a plan first, then either:
//!   - sends it to the user via Telegram for approval (default), or
//!   - auto-executes when the plan is *both* internally-generated
//!     (autonomy / schedule) AND uses only read-mostly tools.
//!
//! Plans are persisted as `kind="plan"` brain notes so they're durable,
//! readable, linkable, and queryable like any other brain content.
//! Status is stored in the brain note's tags (pending / approved /
//! executing / executed / cancelled) — no parallel store.
//!
//! Approval flow with the Telegram bot (Phase 18C wires this up):
//!   1. Agent calls `plan draft { task, body, tools_used, internal? }`.
//!   2. Plan tool saves the brain note + returns a magic marker
//!        `[plan:approve:<slug>]` (or `[plan:auto:<slug>]` for combo
//!        auto-approve).
//!   3. The agent emits the marker in its final response text.
//!   4. bot.rs detects the marker, replaces the response with an
//!      inline-button message ("Approve / Cancel"), or — for `:auto:`
//!      — silently fires a follow-up agent task that calls
//!      `plan execute <slug>`.
//!   5. On approval the bot sends a new agent input
//!      "execute approved plan <slug>"; agent calls `plan execute`,
//!      gets the body back, runs it via normal ReAct.

use peko_core::tool::{Tool, ToolResult};
use peko_core::BrainStore;
use serde_json::json;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use tokio::sync::Mutex;

/// Read-mostly tools — calling only these is safe enough for combo
/// auto-approve. Anything that touches the device's input methods,
/// telephony, package state, or shell escapes the safe set and forces
/// approval.
pub(crate) const READ_ONLY_TOOLS: &[&str] = &[
    "screenshot", "ui_inspect", "memory", "skills", "sensors",
    "wifi", "audio", "draw", "web", "ocr", "brain", "research",
    "filesystem",
];

/// Tags used to track plan status. Stored in the brain note's tags
/// list so existing brain.search / brain.list_recent filters work.
const TAG_PENDING: &str = "plan-pending";
const TAG_APPROVED: &str = "plan-approved";
const TAG_EXECUTING: &str = "plan-executing";
const TAG_EXECUTED: &str = "plan-executed";
const TAG_CANCELLED: &str = "plan-cancelled";

pub struct PlanTool {
    brain: Arc<Mutex<BrainStore>>,
}

impl PlanTool {
    pub fn new(brain: Arc<Mutex<BrainStore>>) -> Self {
        Self { brain }
    }
}

impl Tool for PlanTool {
    fn name(&self) -> &str { "plan" }

    fn description(&self) -> &str {
        "Draft and execute multi-step plans. Use for tasks that need \
         5+ tool calls or several distinct subgoals. \
         \
         Actions: \
         draft { task, body, tools_used: [string], internal?: bool } \
           — save the plan as a brain note (kind=plan), return a \
             marker the Telegram bot turns into approve/cancel \
             buttons. The combo auto-approve fires when internal=true \
             AND every tool in tools_used is read-only — in that case \
             the marker is `[plan:auto:<slug>]` and the bot executes \
             the plan without asking the user. \
         show { slug } — fetch a plan with current status. \
         execute { slug } — mark approved+executing, return the plan \
             body so the agent can drive the next ReAct iteration with \
             it as context. \
         cancel { slug } — mark cancelled, no further action. \
         list { status? } — pending|approved|executing|executed|cancelled. \
         \
         Plans are persisted at <data_dir>/brain/<slug>.md."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["draft", "show", "execute", "cancel", "list"]
                },
                "task": { "type": "string" },
                "body": { "type": "string" },
                "tools_used": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Tool names the plan will invoke. Drives the safe-tool check."
                },
                "internal": {
                    "type": "boolean",
                    "description": "True when the plan is autonomy-generated (curiosity/schedule). Required + true for combo auto-approve."
                },
                "slug": { "type": "string" },
                "status": { "type": "string" }
            },
            "required": ["action"]
        })
    }

    fn execute(
        &self,
        args: serde_json::Value,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<ToolResult>> + Send + '_>> {
        let brain = self.brain.clone();
        Box::pin(async move {
            let action = args["action"].as_str().unwrap_or("").to_string();
            let store = brain.lock().await;
            dispatch(&store, &action, &args)
        })
    }
}

fn dispatch(
    store: &BrainStore,
    action: &str,
    args: &serde_json::Value,
) -> anyhow::Result<ToolResult> {
    match action {
        "draft" => draft(store, args),
        "show" => show(store, args),
        "execute" => execute(store, args),
        "cancel" => cancel(store, args),
        "list" => list(store, args),
        "" => Ok(ToolResult::error("missing 'action'".to_string())),
        other => Ok(ToolResult::error(format!(
            "unknown action '{other}'. valid: draft, show, execute, cancel, list"
        ))),
    }
}

/// Hardening for combo auto-approve: scan the plan body's free text
/// for references to known non-read-only tool names. The LLM can lie
/// in `tools_used`, but writing "step 1: call shell ..." in the body
/// is the natural way to express the intent — and we can detect it.
/// False positives (e.g. the word "shell" in prose) cost an extra
/// approval tap; that's the right side of the safety/UX trade.
fn body_mentions_unsafe_tool(body: &str, declared: &[String]) -> bool {
    // All currently-registered non-read-only tools. Keep this list
    // in sync with main.rs::register_tools — when we add a new
    // dangerous tool, add its name here too. Better safe than sorry:
    // an unknown tool name in this list just means an extra approval.
    const UNSAFE: &[&str] = &[
        "shell", "package_manager", "sms", "call", "telephony",
        "touch", "key_event", "text_input", "unlock_device",
        "audio_pcm", "camera", "gps", "delegate", "bg",
    ];
    let lc = body.to_lowercase();
    let declared_lc: Vec<String> = declared.iter().map(|s| s.to_lowercase()).collect();
    UNSAFE.iter().any(|t| {
        // Skip tools the plan declared — those are accounted for in
        // is_plan_safe; we're catching the UNDECLARED ones here.
        if declared_lc.iter().any(|d| d == t) { return false; }
        // Word-boundary-ish match: surrounded by whitespace, punctuation,
        // backtick, or string boundary. Avoids matching "shell" inside
        // "shellac" but does match "`shell`" and "shell:".
        let needle = *t;
        lc.split(|c: char| !c.is_alphanumeric() && c != '_').any(|w| w == needle)
    })
}

fn draft(store: &BrainStore, args: &serde_json::Value) -> anyhow::Result<ToolResult> {
    let Some(task) = args["task"].as_str() else {
        return Ok(ToolResult::error("missing 'task'".to_string()));
    };
    let Some(body) = args["body"].as_str() else {
        return Ok(ToolResult::error("missing 'body'".to_string()));
    };
    let internal = args["internal"].as_bool().unwrap_or(false);
    let tools_used: Vec<String> = args["tools_used"]
        .as_array()
        .map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect())
        .unwrap_or_default();

    // Combo auto-approve has THREE conditions, all must hold:
    //   1) internal=true (came from autonomy loop, not a user message)
    //   2) every declared tool is in READ_ONLY_TOOLS
    //   3) the plan body's free-text doesn't reference any non-read-only
    //      tool by name. The agent could declare safe tools but write
    //      "step 1: shell rm -rf /" in the body — body_mentions_unsafe
    //      catches that. Without this gate the LLM could trivially
    //      bypass approval.
    let safe = is_plan_safe(&tools_used);
    let body_clean = !body_mentions_unsafe_tool(body, &tools_used);
    let needs_approval = !(internal && safe && body_clean);

    let title = format!("Plan: {task}");
    let safety_line = if safe {
        "✅ all tools in plan are read-only"
    } else {
        "⚠️ plan touches non-read-only tools (write paths, framework, IO)"
    };
    let auto_line = if needs_approval {
        "→ requires Telegram approval"
    } else {
        "→ auto-approved (internal autonomy task with read-only tools)"
    };
    let content = format!(
        "## Task\n\n{task}\n\n## Steps\n\n{body}\n\n## Tools used\n\n{}\n\n\
         ## Approval\n\n{safety_line}\n{auto_line}\n\n## Status\n\nPending\n",
        if tools_used.is_empty() {
            "_(none declared)_".to_string()
        } else {
            tools_used.iter().map(|t| format!("- `{t}`")).collect::<Vec<_>>().join("\n")
        }
    );

    let saved = store.save(&title, &content, Some("plan"), &[TAG_PENDING.to_string()])?;

    let marker = if needs_approval {
        format!("[plan:approve:{}]", saved.slug)
    } else {
        format!("[plan:auto:{}]", saved.slug)
    };

    Ok(ToolResult::success(format!(
        "{marker}\n\n📋 **{}** ({})\n\n{}\n\n_slug: {}_",
        saved.title,
        if needs_approval { "needs approval" } else { "auto-approved" },
        body,
        saved.slug,
    )))
}

fn show(store: &BrainStore, args: &serde_json::Value) -> anyhow::Result<ToolResult> {
    let Some(slug) = args["slug"].as_str() else {
        return Ok(ToolResult::error("missing 'slug'".to_string()));
    };
    match store.get(slug)? {
        Some(n) if n.kind == "plan" => Ok(ToolResult::success(format!(
            "# {}\n\nslug: {}\nstatus tag: {}\nupdated: {}\n\n{}",
            n.title,
            n.slug,
            n.tags.join(", "),
            n.updated_at,
            n.content
        ))),
        Some(_) => Ok(ToolResult::error(format!(
            "note '{slug}' exists but is not a plan"
        ))),
        None => Ok(ToolResult::error(format!("no plan '{slug}'"))),
    }
}

fn execute(store: &BrainStore, args: &serde_json::Value) -> anyhow::Result<ToolResult> {
    let Some(slug) = args["slug"].as_str() else {
        return Ok(ToolResult::error("missing 'slug'".to_string()));
    };
    let Some(plan) = store.get(slug)? else {
        return Ok(ToolResult::error(format!("no plan '{slug}'")));
    };
    if plan.kind != "plan" {
        return Ok(ToolResult::error(format!("'{slug}' is not a plan")));
    }
    if plan.tags.iter().any(|t| t == TAG_CANCELLED) {
        return Ok(ToolResult::error(format!("plan '{slug}' is cancelled")));
    }
    if plan.tags.iter().any(|t| t == TAG_EXECUTED) {
        return Ok(ToolResult::error(format!(
            "plan '{slug}' already executed; draft a new one if you want to re-run"
        )));
    }

    // Status transition: → executing (no longer pending/approved).
    let new_tags: Vec<String> = plan
        .tags
        .iter()
        .filter(|t| {
            *t != TAG_PENDING
                && *t != TAG_APPROVED
                && *t != TAG_EXECUTED
                && *t != TAG_CANCELLED
        })
        .cloned()
        .chain(std::iter::once(TAG_EXECUTING.to_string()))
        .collect();
    let new_content = update_status_block(&plan.content, "Executing");
    store.save(&plan.title, &new_content, Some("plan"), &new_tags)?;

    Ok(ToolResult::success(format!(
        "Executing plan '{slug}'. Use this body to drive the next steps:\n\n{}",
        plan.content
    )))
}

fn cancel(store: &BrainStore, args: &serde_json::Value) -> anyhow::Result<ToolResult> {
    let Some(slug) = args["slug"].as_str() else {
        return Ok(ToolResult::error("missing 'slug'".to_string()));
    };
    let Some(plan) = store.get(slug)? else {
        return Ok(ToolResult::error(format!("no plan '{slug}'")));
    };
    if plan.kind != "plan" {
        return Ok(ToolResult::error(format!("'{slug}' is not a plan")));
    }
    let new_tags: Vec<String> = plan
        .tags
        .iter()
        .filter(|t| {
            *t != TAG_PENDING && *t != TAG_APPROVED && *t != TAG_EXECUTING
        })
        .cloned()
        .chain(std::iter::once(TAG_CANCELLED.to_string()))
        .collect();
    let new_content = update_status_block(&plan.content, "Cancelled");
    store.save(&plan.title, &new_content, Some("plan"), &new_tags)?;
    Ok(ToolResult::success(format!("Plan '{slug}' cancelled.")))
}

fn list(store: &BrainStore, args: &serde_json::Value) -> anyhow::Result<ToolResult> {
    let status_filter = args["status"].as_str();
    let plans = store.list_recent(50, Some("plan"))?;
    let filtered: Vec<_> = plans
        .into_iter()
        .filter(|n| match status_filter {
            None => true,
            Some(s) => {
                let want = format!("plan-{s}");
                n.tags.iter().any(|t| t == &want)
            }
        })
        .collect();
    if filtered.is_empty() {
        return Ok(ToolResult::success(format!(
            "No plans matching status={}",
            status_filter.unwrap_or("any")
        )));
    }
    let mut out = format!(
        "{} plan(s) (status={}):\n",
        filtered.len(),
        status_filter.unwrap_or("any")
    );
    for n in filtered {
        let status = n
            .tags
            .iter()
            .find(|t| t.starts_with("plan-"))
            .map(|t| t.trim_start_matches("plan-").to_string())
            .unwrap_or_else(|| "unknown".to_string());
        out.push_str(&format!(
            "\n  • [{}] {} (slug: {}, updated: {})",
            status, n.title, n.slug,
            n.updated_at.split('T').next().unwrap_or(&n.updated_at)
        ));
    }
    Ok(ToolResult::success(out))
}

pub(crate) fn is_plan_safe(tools: &[String]) -> bool {
    if tools.is_empty() {
        // No declared tools = can't verify safety.
        return false;
    }
    tools.iter().all(|t| READ_ONLY_TOOLS.contains(&t.as_str()))
}

fn update_status_block(content: &str, new_status: &str) -> String {
    // Simple find-and-replace for the "## Status" trailing block.
    if let Some(idx) = content.rfind("## Status") {
        let head = &content[..idx];
        format!("{head}## Status\n\n{new_status}\n")
    } else {
        format!("{content}\n\n## Status\n\n{new_status}\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fresh() -> Arc<Mutex<BrainStore>> {
        Arc::new(Mutex::new(BrainStore::open_in_memory().unwrap()))
    }

    #[test]
    fn is_plan_safe_rejects_empty() {
        assert!(!is_plan_safe(&[]));
    }

    #[test]
    fn is_plan_safe_accepts_read_only_tools() {
        assert!(is_plan_safe(&["brain".into(), "web".into(), "ocr".into()]));
        assert!(is_plan_safe(&["screenshot".into()]));
    }

    #[test]
    fn is_plan_safe_rejects_unsafe_tools() {
        assert!(!is_plan_safe(&["brain".into(), "shell".into()]));
        assert!(!is_plan_safe(&["sms".into()]));
        assert!(!is_plan_safe(&["touch".into()]));
        assert!(!is_plan_safe(&["package_manager".into()]));
    }

    #[tokio::test]
    async fn draft_marks_auto_when_internal_and_safe() {
        let brain = fresh();
        let tool = PlanTool::new(brain);
        let r = tool
            .execute(json!({
                "action": "draft",
                "task": "summarise weekly research",
                "body": "1. brain search...\n2. write summary",
                "tools_used": ["brain"],
                "internal": true
            }))
            .await
            .unwrap();
        assert!(!r.is_error);
        assert!(r.content.contains("[plan:auto:"));
    }

    #[tokio::test]
    async fn draft_marks_approve_when_external() {
        let brain = fresh();
        let tool = PlanTool::new(brain);
        let r = tool
            .execute(json!({
                "action": "draft",
                "task": "x",
                "body": "do it",
                "tools_used": ["brain"],
                "internal": false
            }))
            .await
            .unwrap();
        assert!(r.content.contains("[plan:approve:"));
    }

    #[tokio::test]
    async fn draft_marks_approve_when_unsafe_tools_even_if_internal() {
        let brain = fresh();
        let tool = PlanTool::new(brain);
        let r = tool
            .execute(json!({
                "action": "draft",
                "task": "send sms",
                "body": "1. sms send",
                "tools_used": ["sms"],
                "internal": true
            }))
            .await
            .unwrap();
        assert!(r.content.contains("[plan:approve:"));
    }

    #[tokio::test]
    async fn execute_transitions_status() {
        let brain = fresh();
        let tool = PlanTool::new(brain.clone());
        let r = tool
            .execute(json!({
                "action": "draft",
                "task": "x",
                "body": "do it",
                "tools_used": ["brain"],
                "internal": true
            }))
            .await
            .unwrap();
        let slug = r
            .content
            .lines()
            .find_map(|l| {
                l.find("[plan:auto:").map(|i| {
                    let start = i + "[plan:auto:".len();
                    let end = l[start..].find(']').unwrap();
                    l[start..start + end].to_string()
                })
            })
            .expect("slug from marker");

        let _ = tool
            .execute(json!({"action": "execute", "slug": slug}))
            .await
            .unwrap();

        let store = brain.lock().await;
        let plan = store.get(&slug).unwrap().unwrap();
        assert!(plan.tags.iter().any(|t| t == "plan-executing"));
        assert!(!plan.tags.iter().any(|t| t == "plan-pending"));
    }

    #[tokio::test]
    async fn cancel_marks_status() {
        let brain = fresh();
        let tool = PlanTool::new(brain.clone());
        let r = tool
            .execute(json!({
                "action": "draft", "task": "x", "body": "y",
                "tools_used": ["brain"], "internal": false
            }))
            .await
            .unwrap();
        let slug: String = r
            .content
            .lines()
            .find_map(|l| {
                l.find("[plan:approve:").map(|i| {
                    let start = i + "[plan:approve:".len();
                    let end = l[start..].find(']').unwrap();
                    l[start..start + end].to_string()
                })
            })
            .unwrap();

        let _ = tool.execute(json!({"action": "cancel", "slug": slug})).await.unwrap();
        let exec = tool
            .execute(json!({"action": "execute", "slug": slug}))
            .await
            .unwrap();
        assert!(exec.is_error, "cancelled plan must not execute");
    }
}
