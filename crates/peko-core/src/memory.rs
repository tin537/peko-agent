use rusqlite::{params, Connection};
use std::path::Path;
use chrono::Utc;
use uuid::Uuid;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Memory {
    pub id: String,
    pub key: String,
    pub content: String,
    pub category: MemoryCategory,
    pub importance: f64,
    pub source_session: Option<String>,
    pub created_at: String,
    pub accessed_at: Option<String>,
    pub access_count: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum MemoryCategory {
    Fact,
    Preference,
    Procedure,
    Observation,
    Skill,
    /// Self-evaluation written by the Reflector after each task.
    Reflection,
}

impl std::fmt::Display for MemoryCategory {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            Self::Fact => write!(f, "fact"),
            Self::Preference => write!(f, "preference"),
            Self::Procedure => write!(f, "procedure"),
            Self::Observation => write!(f, "observation"),
            Self::Skill => write!(f, "skill"),
            Self::Reflection => write!(f, "reflection"),
        }
    }
}

impl MemoryCategory {
    pub fn from_str(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "preference" => Self::Preference,
            "procedure" => Self::Procedure,
            "observation" => Self::Observation,
            "skill" => Self::Skill,
            "reflection" => Self::Reflection,
            _ => Self::Fact,
        }
    }
}

pub struct MemoryStore {
    conn: Connection,
}

impl MemoryStore {
    pub fn open(path: &Path) -> anyhow::Result<Self> {
        let conn = Connection::open(path)?;
        let store = Self { conn };
        store.init_schema()?;
        Ok(store)
    }

    pub fn open_in_memory() -> anyhow::Result<Self> {
        let conn = Connection::open_in_memory()?;
        let store = Self { conn };
        store.init_schema()?;
        Ok(store)
    }

    fn init_schema(&self) -> anyhow::Result<()> {
        self.conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS memories (
                id TEXT PRIMARY KEY,
                key TEXT NOT NULL,
                content TEXT NOT NULL,
                category TEXT NOT NULL DEFAULT 'fact',
                importance REAL NOT NULL DEFAULT 0.5,
                source_session TEXT,
                created_at TEXT NOT NULL,
                accessed_at TEXT,
                access_count INTEGER NOT NULL DEFAULT 0
            );

            CREATE INDEX IF NOT EXISTS idx_memories_key ON memories(key);
            CREATE INDEX IF NOT EXISTS idx_memories_category ON memories(category);
            CREATE INDEX IF NOT EXISTS idx_memories_importance ON memories(importance DESC);

            CREATE VIRTUAL TABLE IF NOT EXISTS memories_fts USING fts5(
                key, content, category,
                content='memories',
                content_rowid='rowid'
            );

            -- Keep FTS in sync
            CREATE TRIGGER IF NOT EXISTS memories_ai AFTER INSERT ON memories BEGIN
                INSERT INTO memories_fts(rowid, key, content, category)
                VALUES (new.rowid, new.key, new.content, new.category);
            END;

            CREATE TRIGGER IF NOT EXISTS memories_ad AFTER DELETE ON memories BEGIN
                INSERT INTO memories_fts(memories_fts, rowid, key, content, category)
                VALUES ('delete', old.rowid, old.key, old.content, old.category);
            END;

            CREATE TRIGGER IF NOT EXISTS memories_au AFTER UPDATE ON memories BEGIN
                INSERT INTO memories_fts(memories_fts, rowid, key, content, category)
                VALUES ('delete', old.rowid, old.key, old.content, old.category);
                INSERT INTO memories_fts(rowid, key, content, category)
                VALUES (new.rowid, new.key, new.content, new.category);
            END;
            "
        )?;
        Ok(())
    }

    /// Save a new memory. If a memory with the same key exists, update it.
    pub fn save(&self, key: &str, content: &str, category: &MemoryCategory,
                importance: f64, source_session: Option<&str>) -> anyhow::Result<String> {
        let now = Utc::now().to_rfc3339();

        // Check if key already exists — update instead of duplicate
        let existing: Option<String> = self.conn.query_row(
            "SELECT id FROM memories WHERE key = ?1",
            params![key],
            |row| row.get(0),
        ).ok();

        if let Some(id) = existing {
            self.conn.execute(
                "UPDATE memories SET content = ?1, category = ?2, importance = ?3, accessed_at = ?4
                 WHERE id = ?5",
                params![content, category.to_string(), importance, now, id],
            )?;
            Ok(id)
        } else {
            let id = Uuid::new_v4().to_string();
            self.conn.execute(
                "INSERT INTO memories (id, key, content, category, importance, source_session, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![id, key, content, category.to_string(), importance, source_session, now],
            )?;
            Ok(id)
        }
    }

    /// Full-text search across all memories. Returns ranked results.
    pub fn search(&self, query: &str, limit: usize) -> anyhow::Result<Vec<Memory>> {
        let now = Utc::now().to_rfc3339();

        // FTS5 search with ranking
        let mut stmt = self.conn.prepare(
            "SELECT m.id, m.key, m.content, m.category, m.importance,
                    m.source_session, m.created_at, m.accessed_at, m.access_count
             FROM memories_fts f
             JOIN memories m ON f.rowid = m.rowid
             WHERE memories_fts MATCH ?1
             ORDER BY rank * m.importance DESC
             LIMIT ?2"
        )?;

        let rows = stmt.query_map(params![query, limit as i64], |row| {
            Ok(Memory {
                id: row.get(0)?,
                key: row.get(1)?,
                content: row.get(2)?,
                category: MemoryCategory::from_str(&row.get::<_, String>(3)?),
                importance: row.get(4)?,
                source_session: row.get(5)?,
                created_at: row.get(6)?,
                accessed_at: row.get(7)?,
                access_count: row.get(8)?,
            })
        })?;

        let memories: Vec<Memory> = rows.collect::<Result<Vec<_>, _>>()?;

        // Update access stats for returned memories
        for mem in &memories {
            let _ = self.conn.execute(
                "UPDATE memories SET accessed_at = ?1, access_count = access_count + 1 WHERE id = ?2",
                params![now, mem.id],
            );
        }

        Ok(memories)
    }

    /// List all memories, ordered by importance then recency
    pub fn list(&self, limit: usize, category: Option<&str>) -> anyhow::Result<Vec<Memory>> {
        if let Some(cat) = category {
            let mut stmt = self.conn.prepare(
                "SELECT id, key, content, category, importance, source_session, created_at, accessed_at, access_count
                 FROM memories WHERE category = ?1
                 ORDER BY importance DESC, created_at DESC LIMIT ?2"
            )?;
            let rows = stmt.query_map(params![cat, limit as i64], Self::row_to_memory)?;
            Ok(rows.collect::<Result<Vec<_>, _>>()?)
        } else {
            let mut stmt = self.conn.prepare(
                "SELECT id, key, content, category, importance, source_session, created_at, accessed_at, access_count
                 FROM memories ORDER BY importance DESC, created_at DESC LIMIT ?1"
            )?;
            let rows = stmt.query_map(params![limit as i64], Self::row_to_memory)?;
            Ok(rows.collect::<Result<Vec<_>, _>>()?)
        }
    }

    /// Delete a memory by ID or key
    pub fn delete(&self, id_or_key: &str) -> anyhow::Result<bool> {
        let affected = self.conn.execute(
            "DELETE FROM memories WHERE id = ?1 OR key = ?1",
            params![id_or_key],
        )?;
        Ok(affected > 0)
    }

    /// Get memory count
    pub fn count(&self) -> anyhow::Result<usize> {
        let count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM memories", [], |row| row.get(0)
        )?;
        Ok(count as usize)
    }

    // ── Gardener (Phase C) ─────────────────────────────────────
    //
    // Periodically called (daily via Scheduler) to prevent memory from growing
    // without bound. Steps:
    //   1. Delete low-importance, never-accessed, old memories
    //   2. (future) Cluster + summarize — see docs/implementation/Autonomy.md

    /// Prune memories that are:
    ///   - older than `age_days`
    ///   - never accessed since creation (access_count == 0)
    ///   - importance below `min_importance`
    ///
    /// Returns count deleted.
    pub fn prune(&self, age_days: i64, min_importance: f64) -> anyhow::Result<usize> {
        let cutoff = chrono::Utc::now() - chrono::Duration::days(age_days);
        let cutoff_str = cutoff.to_rfc3339();
        let affected = self.conn.execute(
            "DELETE FROM memories
             WHERE created_at < ?1
               AND access_count = 0
               AND importance < ?2
               AND category != 'skill'",   // skills are managed separately
            params![cutoff_str, min_importance],
        )?;
        Ok(affected)
    }

    /// Decay importance of un-accessed memories over time. Drops importance
    /// by `factor` (e.g. 0.95) for any memory not touched in `age_days`.
    /// Helps the prune step catch "was important once, but no longer" items.
    pub fn decay_importance(&self, age_days: i64, factor: f64) -> anyhow::Result<usize> {
        let cutoff = chrono::Utc::now() - chrono::Duration::days(age_days);
        let cutoff_str = cutoff.to_rfc3339();
        let affected = self.conn.execute(
            "UPDATE memories
             SET importance = importance * ?1
             WHERE (accessed_at IS NULL OR accessed_at < ?2)
               AND category != 'skill'",
            params![factor, cutoff_str],
        )?;
        Ok(affected)
    }

    /// Build a text block of relevant memories for system prompt injection
    pub fn build_context(&self, query: &str, max_memories: usize) -> anyhow::Result<String> {
        let memories = self.search(query, max_memories)?;
        if memories.is_empty() {
            return Ok(String::new());
        }

        let mut context = String::from("## Relevant Memories\n\n");
        for (i, mem) in memories.iter().enumerate() {
            context.push_str(&format!(
                "{}. [{}] **{}**: {}\n",
                i + 1,
                mem.category,
                mem.key,
                mem.content
            ));
        }
        Ok(context)
    }

    fn row_to_memory(row: &rusqlite::Row) -> rusqlite::Result<Memory> {
        Ok(Memory {
            id: row.get(0)?,
            key: row.get(1)?,
            content: row.get(2)?,
            category: MemoryCategory::from_str(&row.get::<_, String>(3)?),
            importance: row.get(4)?,
            source_session: row.get(5)?,
            created_at: row.get(6)?,
            accessed_at: row.get(7)?,
            access_count: row.get(8)?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_save_and_search() {
        let store = MemoryStore::open_in_memory().unwrap();

        store.save("user_name", "The user's name is Alex", &MemoryCategory::Fact, 0.8, None).unwrap();
        store.save("wifi_password", "Home WiFi password is abc123", &MemoryCategory::Fact, 0.9, None).unwrap();
        store.save("prefers_concise", "User prefers short, direct responses", &MemoryCategory::Preference, 0.7, None).unwrap();

        let results = store.search("user name", 5).unwrap();
        assert!(!results.is_empty());
        assert!(results[0].content.contains("Alex"));
    }

    #[test]
    fn test_upsert_on_same_key() {
        let store = MemoryStore::open_in_memory().unwrap();

        store.save("timezone", "User is in UTC+7", &MemoryCategory::Fact, 0.5, None).unwrap();
        store.save("timezone", "User moved to UTC+9", &MemoryCategory::Fact, 0.5, None).unwrap();

        let all = store.list(10, None).unwrap();
        assert_eq!(all.len(), 1);
        assert!(all[0].content.contains("UTC+9"));
    }

    #[test]
    fn test_delete() {
        let store = MemoryStore::open_in_memory().unwrap();
        store.save("temp", "temporary memory", &MemoryCategory::Fact, 0.3, None).unwrap();
        assert_eq!(store.count().unwrap(), 1);

        store.delete("temp").unwrap();
        assert_eq!(store.count().unwrap(), 0);
    }

    #[test]
    fn test_build_context() {
        let store = MemoryStore::open_in_memory().unwrap();
        store.save("device_model", "Running on Pixel 4a", &MemoryCategory::Fact, 0.6, None).unwrap();
        store.save("wifi_setup", "WiFi is WPA2, SSID=HomeNet", &MemoryCategory::Procedure, 0.8, None).unwrap();

        let ctx = store.build_context("device", 5).unwrap();
        assert!(ctx.contains("Pixel 4a"));
    }

    #[test]
    fn test_list_by_category() {
        let store = MemoryStore::open_in_memory().unwrap();
        store.save("k1", "fact1", &MemoryCategory::Fact, 0.5, None).unwrap();
        store.save("k2", "pref1", &MemoryCategory::Preference, 0.5, None).unwrap();
        store.save("k3", "fact2", &MemoryCategory::Fact, 0.5, None).unwrap();

        let facts = store.list(10, Some("fact")).unwrap();
        assert_eq!(facts.len(), 2);

        let prefs = store.list(10, Some("preference")).unwrap();
        assert_eq!(prefs.len(), 1);
    }
}
