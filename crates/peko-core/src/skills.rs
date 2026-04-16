use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use tracing::info;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Skill {
    pub name: String,
    pub description: String,
    pub steps: String,
    pub category: String,
    pub created_at: String,
    pub updated_at: String,
    pub success_count: u32,
    pub fail_count: u32,
    pub tags: Vec<String>,
}

impl Skill {
    pub fn success_rate(&self) -> f32 {
        let total = self.success_count + self.fail_count;
        if total == 0 { return 0.0; }
        self.success_count as f32 / total as f32 * 100.0
    }

    fn to_markdown(&self) -> String {
        format!(
            "---\nname: {}\ndescription: {}\ncategory: {}\ncreated: {}\nupdated: {}\nsuccess_count: {}\nfail_count: {}\ntags: {}\n---\n\n{}",
            self.name,
            self.description,
            self.category,
            self.created_at,
            self.updated_at,
            self.success_count,
            self.fail_count,
            self.tags.join(", "),
            self.steps,
        )
    }

    fn from_markdown(content: &str) -> Option<Self> {
        let parts: Vec<&str> = content.splitn(3, "---").collect();
        if parts.len() < 3 { return None; }

        let frontmatter = parts[1].trim();
        let steps = parts[2].trim().to_string();

        let mut fields: HashMap<&str, String> = HashMap::new();
        for line in frontmatter.lines() {
            if let Some((key, val)) = line.split_once(':') {
                fields.insert(key.trim(), val.trim().to_string());
            }
        }

        Some(Skill {
            name: fields.get("name")?.clone(),
            description: fields.get("description").cloned().unwrap_or_default(),
            category: fields.get("category").cloned().unwrap_or_else(|| "general".to_string()),
            created_at: fields.get("created").cloned().unwrap_or_default(),
            updated_at: fields.get("updated").cloned().unwrap_or_default(),
            success_count: fields.get("success_count").and_then(|s| s.parse().ok()).unwrap_or(0),
            fail_count: fields.get("fail_count").and_then(|s| s.parse().ok()).unwrap_or(0),
            tags: fields.get("tags")
                .map(|s| s.split(',').map(|t| t.trim().to_string()).filter(|t| !t.is_empty()).collect())
                .unwrap_or_default(),
            steps,
        })
    }
}

pub struct SkillStore {
    dir: PathBuf,
    skills: HashMap<String, Skill>,
}

impl SkillStore {
    pub fn open(dir: &Path) -> anyhow::Result<Self> {
        fs::create_dir_all(dir)?;

        let mut skills = HashMap::new();
        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("md") {
                if let Ok(content) = fs::read_to_string(&path) {
                    if let Some(skill) = Skill::from_markdown(&content) {
                        skills.insert(skill.name.clone(), skill);
                    }
                }
            }
        }

        info!(count = skills.len(), dir = %dir.display(), "skills loaded");
        Ok(Self { dir: dir.to_path_buf(), skills })
    }

    pub fn create(&mut self, name: &str, description: &str, steps: &str,
                  category: &str, tags: Vec<String>) -> anyhow::Result<()> {
        let now = Utc::now().to_rfc3339();
        let skill = Skill {
            name: name.to_string(),
            description: description.to_string(),
            steps: steps.to_string(),
            category: category.to_string(),
            created_at: now.clone(),
            updated_at: now,
            success_count: 0,
            fail_count: 0,
            tags,
        };

        self.save_to_disk(&skill)?;
        self.skills.insert(name.to_string(), skill);
        Ok(())
    }

    pub fn improve(&mut self, name: &str, new_steps: &str) -> anyhow::Result<()> {
        let skill = self.skills.get_mut(name)
            .ok_or_else(|| anyhow::anyhow!("skill '{}' not found", name))?;

        skill.steps = new_steps.to_string();
        skill.updated_at = Utc::now().to_rfc3339();

        let skill_clone = skill.clone();
        self.save_to_disk(&skill_clone)?;
        Ok(())
    }

    pub fn record_outcome(&mut self, name: &str, success: bool) -> anyhow::Result<()> {
        let skill = self.skills.get_mut(name)
            .ok_or_else(|| anyhow::anyhow!("skill '{}' not found", name))?;

        if success { skill.success_count += 1; } else { skill.fail_count += 1; }
        skill.updated_at = Utc::now().to_rfc3339();

        let skill_clone = skill.clone();
        self.save_to_disk(&skill_clone)?;
        Ok(())
    }

    pub fn get(&self, name: &str) -> Option<&Skill> {
        self.skills.get(name)
    }

    pub fn list(&self) -> Vec<&Skill> {
        let mut skills: Vec<&Skill> = self.skills.values().collect();
        skills.sort_by(|a, b| b.success_count.cmp(&a.success_count));
        skills
    }

    pub fn search(&self, query: &str) -> Vec<&Skill> {
        let lower = query.to_lowercase();
        self.skills.values()
            .filter(|s| {
                s.name.to_lowercase().contains(&lower)
                    || s.description.to_lowercase().contains(&lower)
                    || s.tags.iter().any(|t| t.to_lowercase().contains(&lower))
                    || s.steps.to_lowercase().contains(&lower)
            })
            .collect()
    }

    pub fn delete(&mut self, name: &str) -> anyhow::Result<bool> {
        if self.skills.remove(name).is_some() {
            let path = self.dir.join(format!("{}.md", sanitize_filename(name)));
            let _ = fs::remove_file(&path);
            Ok(true)
        } else {
            Ok(false)
        }
    }

    pub fn count(&self) -> usize {
        self.skills.len()
    }

    /// Build prompt context listing available skills
    pub fn build_context(&self, query: &str) -> String {
        let relevant = if query.is_empty() {
            self.list()
        } else {
            self.search(query)
        };

        if relevant.is_empty() { return String::new(); }

        let mut ctx = String::from("## Available Skills\n\n");
        ctx.push_str("If a skill matches your task, follow its steps. If you find a better approach, use the skills tool to improve it.\n\n");

        for skill in relevant.iter().take(5) {
            ctx.push_str(&format!(
                "### {} — {}\nSuccess rate: {:.0}% ({} uses)\n```\n{}\n```\n\n",
                skill.name, skill.description, skill.success_rate(),
                skill.success_count + skill.fail_count,
                skill.steps
            ));
        }
        ctx
    }

    fn save_to_disk(&self, skill: &Skill) -> anyhow::Result<()> {
        let filename = format!("{}.md", sanitize_filename(&skill.name));
        let path = self.dir.join(&filename);
        fs::write(&path, skill.to_markdown())?;
        Ok(())
    }
}

fn sanitize_filename(name: &str) -> String {
    name.chars()
        .map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '_' })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_skill_markdown_roundtrip() {
        let skill = Skill {
            name: "test_skill".to_string(),
            description: "A test".to_string(),
            steps: "1. Do thing\n2. Do other thing".to_string(),
            category: "general".to_string(),
            created_at: "2026-04-16".to_string(),
            updated_at: "2026-04-16".to_string(),
            success_count: 3,
            fail_count: 1,
            tags: vec!["test".to_string(), "demo".to_string()],
        };

        let md = skill.to_markdown();
        let parsed = Skill::from_markdown(&md).unwrap();
        assert_eq!(parsed.name, "test_skill");
        assert_eq!(parsed.description, "A test");
        assert_eq!(parsed.success_count, 3);
        assert_eq!(parsed.fail_count, 1);
        assert!(parsed.steps.contains("Do thing"));
    }

    #[test]
    fn test_skill_store_crud() {
        let tmp = std::env::temp_dir().join("peko_skill_test");
        let _ = fs::remove_dir_all(&tmp);

        let mut store = SkillStore::open(&tmp).unwrap();
        assert_eq!(store.count(), 0);

        store.create("open_settings", "Navigate to Android Settings",
            "1. Press HOME\n2. Take screenshot\n3. Find Settings icon\n4. Tap it",
            "navigation", vec!["ui".to_string()]).unwrap();
        assert_eq!(store.count(), 1);

        let skill = store.get("open_settings").unwrap();
        assert_eq!(skill.description, "Navigate to Android Settings");

        store.improve("open_settings", "1. Press HOME\n2. Swipe up for app drawer\n3. Find Settings\n4. Tap it").unwrap();
        let skill = store.get("open_settings").unwrap();
        assert!(skill.steps.contains("Swipe up"));

        store.record_outcome("open_settings", true).unwrap();
        store.record_outcome("open_settings", true).unwrap();
        store.record_outcome("open_settings", false).unwrap();
        let skill = store.get("open_settings").unwrap();
        assert_eq!(skill.success_count, 2);
        assert_eq!(skill.fail_count, 1);
        assert!((skill.success_rate() - 66.6).abs() < 1.0);

        // Search
        let results = store.search("settings");
        assert_eq!(results.len(), 1);

        // Verify file on disk
        assert!(tmp.join("open_settings.md").exists());

        // Reload from disk
        let store2 = SkillStore::open(&tmp).unwrap();
        assert_eq!(store2.count(), 1);
        assert_eq!(store2.get("open_settings").unwrap().success_count, 2);

        // Delete
        store.delete("open_settings").unwrap();
        assert_eq!(store.count(), 0);

        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_build_context() {
        let tmp = std::env::temp_dir().join("peko_skill_ctx_test");
        let _ = fs::remove_dir_all(&tmp);
        let mut store = SkillStore::open(&tmp).unwrap();

        store.create("send_sms", "Send an SMS message",
            "1. Use sms tool\n2. Confirm sent", "telephony", vec![]).unwrap();

        let ctx = store.build_context("sms");
        assert!(ctx.contains("send_sms"));
        assert!(ctx.contains("Send an SMS"));

        let empty = store.build_context("nonexistent");
        assert!(empty.is_empty());

        let _ = fs::remove_dir_all(&tmp);
    }
}
