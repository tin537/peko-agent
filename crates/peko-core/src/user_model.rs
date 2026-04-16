use std::path::{Path, PathBuf};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use tracing::info;

/// Persistent user model that deepens over sessions.
/// The agent learns preferences, expertise, patterns, and adapts its behavior.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserModel {
    pub name: Option<String>,
    pub language: String,
    pub expertise_level: ExpertiseLevel,
    pub preferences: UserPreferences,
    pub patterns: UserPatterns,
    pub observations: Vec<String>,
    pub updated_at: String,
    pub interaction_count: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ExpertiseLevel {
    Beginner,
    Intermediate,
    Advanced,
    Developer,
}

impl std::fmt::Display for ExpertiseLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            Self::Beginner => write!(f, "beginner"),
            Self::Intermediate => write!(f, "intermediate"),
            Self::Advanced => write!(f, "advanced"),
            Self::Developer => write!(f, "developer"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserPreferences {
    pub verbose_responses: bool,
    pub confirm_dangerous: bool,
    pub auto_screenshot: bool,
    pub preferred_language: Option<String>,
    pub response_style: ResponseStyle,
    pub custom: std::collections::HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ResponseStyle {
    Concise,
    Detailed,
    Technical,
    Casual,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserPatterns {
    pub common_tasks: Vec<String>,
    pub preferred_apps: Vec<String>,
    pub active_hours: Option<String>,
    pub avg_task_complexity: f32,
    pub total_tasks: u64,
}

impl Default for UserModel {
    fn default() -> Self {
        Self {
            name: None,
            language: "en".to_string(),
            expertise_level: ExpertiseLevel::Intermediate,
            preferences: UserPreferences {
                verbose_responses: false,
                confirm_dangerous: true,
                auto_screenshot: true,
                preferred_language: None,
                response_style: ResponseStyle::Concise,
                custom: std::collections::HashMap::new(),
            },
            patterns: UserPatterns {
                common_tasks: Vec::new(),
                preferred_apps: Vec::new(),
                active_hours: None,
                avg_task_complexity: 0.0,
                total_tasks: 0,
            },
            observations: Vec::new(),
            updated_at: Utc::now().to_rfc3339(),
            interaction_count: 0,
        }
    }
}

impl UserModel {
    /// Load from JSON file, or create default if not found
    pub fn load(path: &Path) -> Self {
        match std::fs::read_to_string(path) {
            Ok(content) => {
                match serde_json::from_str(&content) {
                    Ok(model) => {
                        info!(path = %path.display(), "user model loaded");
                        model
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, "corrupt user model, using default");
                        Self::default()
                    }
                }
            }
            Err(_) => Self::default(),
        }
    }

    /// Save to JSON file
    pub fn save(&self, path: &Path) -> anyhow::Result<()> {
        let json = serde_json::to_string_pretty(self)?;
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, &json)?;
        std::fs::rename(&tmp, path)?;
        Ok(())
    }

    /// Record a new task interaction
    pub fn record_task(&mut self, task: &str, iterations: usize) {
        self.interaction_count += 1;
        self.patterns.total_tasks += 1;
        self.updated_at = Utc::now().to_rfc3339();

        // Track task complexity (rolling average)
        let complexity = iterations as f32;
        let n = self.patterns.total_tasks as f32;
        self.patterns.avg_task_complexity =
            self.patterns.avg_task_complexity * ((n - 1.0) / n) + complexity / n;

        // Track common tasks (keep top 10)
        let task_lower = task.to_lowercase();
        let keywords: Vec<&str> = task_lower.split_whitespace().take(4).collect();
        let task_key = keywords.join(" ");

        if !self.patterns.common_tasks.contains(&task_key) {
            self.patterns.common_tasks.push(task_key);
            if self.patterns.common_tasks.len() > 20 {
                self.patterns.common_tasks.remove(0);
            }
        }
    }

    /// Add an observation about the user
    pub fn add_observation(&mut self, observation: &str) {
        // Avoid duplicates
        let lower = observation.to_lowercase();
        if self.observations.iter().any(|o| o.to_lowercase() == lower) {
            return;
        }

        self.observations.push(observation.to_string());
        self.updated_at = Utc::now().to_rfc3339();

        // Keep max 30 observations
        if self.observations.len() > 30 {
            self.observations.remove(0);
        }
    }

    /// Update a preference
    pub fn set_preference(&mut self, key: &str, value: &str) {
        match key {
            "name" => self.name = Some(value.to_string()),
            "language" => self.language = value.to_string(),
            "expertise" => {
                self.expertise_level = match value {
                    "beginner" => ExpertiseLevel::Beginner,
                    "advanced" => ExpertiseLevel::Advanced,
                    "developer" => ExpertiseLevel::Developer,
                    _ => ExpertiseLevel::Intermediate,
                };
            }
            "verbose" => self.preferences.verbose_responses = value == "true",
            "confirm_dangerous" => self.preferences.confirm_dangerous = value == "true",
            "response_style" => {
                self.preferences.response_style = match value {
                    "detailed" => ResponseStyle::Detailed,
                    "technical" => ResponseStyle::Technical,
                    "casual" => ResponseStyle::Casual,
                    _ => ResponseStyle::Concise,
                };
            }
            _ => { self.preferences.custom.insert(key.to_string(), value.to_string()); }
        }
        self.updated_at = Utc::now().to_rfc3339();
    }

    /// Build prompt context summarizing the user
    pub fn build_context(&self) -> String {
        if self.interaction_count == 0 && self.name.is_none() && self.observations.is_empty() {
            return String::new();
        }

        let mut ctx = String::from("## About the User\n");

        if let Some(ref name) = self.name {
            ctx.push_str(&format!("Name: {}. ", name));
        }

        ctx.push_str(&format!(
            "Expertise: {}. Response style: {:?}. {} interactions so far.\n",
            self.expertise_level,
            self.preferences.response_style,
            self.interaction_count
        ));

        if !self.patterns.common_tasks.is_empty() {
            let recent: Vec<&str> = self.patterns.common_tasks.iter()
                .rev().take(5).map(|s| s.as_str()).collect();
            ctx.push_str(&format!("Common tasks: {}\n", recent.join(", ")));
        }

        if !self.patterns.preferred_apps.is_empty() {
            ctx.push_str(&format!("Preferred apps: {}\n", self.patterns.preferred_apps.join(", ")));
        }

        if !self.observations.is_empty() {
            ctx.push_str("Observations:\n");
            for obs in self.observations.iter().rev().take(10) {
                ctx.push_str(&format!("- {}\n", obs));
            }
        }

        if self.preferences.verbose_responses {
            ctx.push_str("User prefers detailed, verbose responses.\n");
        }
        if !self.preferences.confirm_dangerous {
            ctx.push_str("User has disabled confirmation for dangerous tools.\n");
        }

        ctx
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_model() {
        let model = UserModel::default();
        assert_eq!(model.expertise_level, ExpertiseLevel::Intermediate);
        assert_eq!(model.interaction_count, 0);
        assert!(model.build_context().is_empty());
    }

    #[test]
    fn test_record_task() {
        let mut model = UserModel::default();
        model.record_task("open settings and find wifi", 3);
        model.record_task("send sms to bob", 2);

        assert_eq!(model.interaction_count, 2);
        assert_eq!(model.patterns.total_tasks, 2);
        assert!(model.patterns.common_tasks.len() >= 2);
    }

    #[test]
    fn test_observations() {
        let mut model = UserModel::default();
        model.add_observation("prefers short responses");
        model.add_observation("often asks about wifi");
        model.add_observation("prefers short responses"); // duplicate

        assert_eq!(model.observations.len(), 2);
    }

    #[test]
    fn test_preferences() {
        let mut model = UserModel::default();
        model.set_preference("name", "Alex");
        model.set_preference("expertise", "developer");
        model.set_preference("response_style", "technical");

        assert_eq!(model.name, Some("Alex".to_string()));
        assert_eq!(model.expertise_level, ExpertiseLevel::Developer);
        assert_eq!(model.preferences.response_style, ResponseStyle::Technical);
    }

    #[test]
    fn test_build_context() {
        let mut model = UserModel::default();
        model.set_preference("name", "Bob");
        model.record_task("check battery", 1);
        model.add_observation("prefers concise answers");

        let ctx = model.build_context();
        assert!(ctx.contains("Bob"));
        assert!(ctx.contains("check battery"));
        assert!(ctx.contains("concise answers"));
    }

    #[test]
    fn test_save_load() {
        let tmp = std::env::temp_dir().join("peko_user_model_test.json");

        let mut model = UserModel::default();
        model.set_preference("name", "Test");
        model.record_task("test task", 2);
        model.save(&tmp).unwrap();

        let loaded = UserModel::load(&tmp);
        assert_eq!(loaded.name, Some("Test".to_string()));
        assert_eq!(loaded.interaction_count, 1);

        let _ = std::fs::remove_file(&tmp);
    }
}
