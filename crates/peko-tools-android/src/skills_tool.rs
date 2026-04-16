use peko_core::tool::{Tool, ToolResult};
use peko_core::skills::SkillStore;
use serde_json::json;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use tokio::sync::Mutex;

pub struct SkillsTool {
    store: Arc<Mutex<SkillStore>>,
}

impl SkillsTool {
    pub fn new(store: Arc<Mutex<SkillStore>>) -> Self {
        Self { store }
    }
}

impl Tool for SkillsTool {
    fn name(&self) -> &str { "skills" }

    fn description(&self) -> &str {
        "Manage reusable skills — procedures you've learned from experience.\n\n\
         Actions:\n\
         - create: Save a new skill (name, description, steps, category, tags)\n\
         - use: Get a skill's steps to follow (also marks it as used)\n\
         - improve: Update a skill's steps with a better approach\n\
         - success: Record that a skill worked\n\
         - fail: Record that a skill failed (consider improving it)\n\
         - list: Show all available skills\n\
         - search: Find skills by keyword\n\
         - delete: Remove a skill\n\n\
         When you successfully complete a multi-step task, save it as a skill.\n\
         When you follow a skill and it works, call success. If it fails, call fail and consider improving it."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["create", "use", "improve", "success", "fail", "list", "search", "delete"],
                    "description": "Skill operation"
                },
                "name": {
                    "type": "string",
                    "description": "Skill name (for create/use/improve/success/fail/delete)"
                },
                "description": {
                    "type": "string",
                    "description": "What the skill does (for create)"
                },
                "steps": {
                    "type": "string",
                    "description": "Step-by-step procedure (for create/improve)"
                },
                "category": {
                    "type": "string",
                    "description": "Category: navigation, telephony, app_control, system, general (for create)"
                },
                "tags": {
                    "type": "string",
                    "description": "Comma-separated tags (for create)"
                },
                "query": {
                    "type": "string",
                    "description": "Search query (for search)"
                }
            },
            "required": ["action"]
        })
    }

    fn execute(
        &self,
        args: serde_json::Value,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<ToolResult>> + Send + '_>> {
        let store = self.store.clone();
        Box::pin(async move {
            let action = args["action"].as_str()
                .ok_or_else(|| anyhow::anyhow!("missing 'action'"))?;

            let mut s = store.lock().await;

            match action {
                "create" => {
                    let name = args["name"].as_str()
                        .ok_or_else(|| anyhow::anyhow!("missing 'name'"))?;
                    let description = args["description"].as_str().unwrap_or("");
                    let steps = args["steps"].as_str()
                        .ok_or_else(|| anyhow::anyhow!("missing 'steps'"))?;
                    let category = args["category"].as_str().unwrap_or("general");
                    let tags: Vec<String> = args["tags"].as_str()
                        .map(|s| s.split(',').map(|t| t.trim().to_string()).filter(|t| !t.is_empty()).collect())
                        .unwrap_or_default();

                    s.create(name, description, steps, category, tags)?;
                    Ok(ToolResult::success(format!(
                        "Skill '{}' created ({} total skills)", name, s.count()
                    )))
                }

                "use" => {
                    let name = args["name"].as_str()
                        .ok_or_else(|| anyhow::anyhow!("missing 'name'"))?;

                    match s.get(name) {
                        Some(skill) => {
                            Ok(ToolResult::success(format!(
                                "Skill: {}\nDescription: {}\nSuccess rate: {:.0}% ({} uses)\n\nSteps:\n{}",
                                skill.name, skill.description, skill.success_rate(),
                                skill.success_count + skill.fail_count,
                                skill.steps
                            )))
                        }
                        None => Ok(ToolResult::error(format!("Skill '{}' not found", name)))
                    }
                }

                "improve" => {
                    let name = args["name"].as_str()
                        .ok_or_else(|| anyhow::anyhow!("missing 'name'"))?;
                    let steps = args["steps"].as_str()
                        .ok_or_else(|| anyhow::anyhow!("missing 'steps'"))?;

                    s.improve(name, steps)?;
                    Ok(ToolResult::success(format!("Skill '{}' updated with new steps", name)))
                }

                "success" => {
                    let name = args["name"].as_str()
                        .ok_or_else(|| anyhow::anyhow!("missing 'name'"))?;
                    s.record_outcome(name, true)?;
                    let skill = s.get(name).unwrap();
                    Ok(ToolResult::success(format!(
                        "Recorded success for '{}' (now {:.0}% success rate)",
                        name, skill.success_rate()
                    )))
                }

                "fail" => {
                    let name = args["name"].as_str()
                        .ok_or_else(|| anyhow::anyhow!("missing 'name'"))?;
                    s.record_outcome(name, false)?;
                    let skill = s.get(name).unwrap();
                    Ok(ToolResult::success(format!(
                        "Recorded failure for '{}' (now {:.0}% success rate). Consider improving this skill.",
                        name, skill.success_rate()
                    )))
                }

                "list" => {
                    let skills = s.list();
                    if skills.is_empty() {
                        Ok(ToolResult::success("No skills yet. Create one after completing a multi-step task.".to_string()))
                    } else {
                        let mut output = format!("{} skills:\n\n", skills.len());
                        for skill in skills {
                            output.push_str(&format!(
                                "- **{}**: {} ({:.0}% success, {} uses) [{}]\n",
                                skill.name, skill.description, skill.success_rate(),
                                skill.success_count + skill.fail_count, skill.category
                            ));
                        }
                        Ok(ToolResult::success(output))
                    }
                }

                "search" => {
                    let query = args["query"].as_str().unwrap_or(
                        args["name"].as_str().unwrap_or("")
                    );
                    let results = s.search(query);
                    if results.is_empty() {
                        Ok(ToolResult::success(format!("No skills matching '{}'", query)))
                    } else {
                        let mut output = format!("Found {} skills for '{}':\n\n", results.len(), query);
                        for skill in results {
                            output.push_str(&format!(
                                "- **{}**: {}\n  Steps: {}\n\n",
                                skill.name, skill.description,
                                skill.steps.lines().next().unwrap_or("(empty)")
                            ));
                        }
                        Ok(ToolResult::success(output))
                    }
                }

                "delete" => {
                    let name = args["name"].as_str()
                        .ok_or_else(|| anyhow::anyhow!("missing 'name'"))?;
                    if s.delete(name)? {
                        Ok(ToolResult::success(format!("Skill '{}' deleted", name)))
                    } else {
                        Ok(ToolResult::error(format!("Skill '{}' not found", name)))
                    }
                }

                _ => Ok(ToolResult::error(format!("Unknown action: {}", action))),
            }
        })
    }
}
