use rusqlite::{Connection, params};
use std::path::Path;
use chrono::Utc;
use uuid::Uuid;

use crate::message::Message;

pub struct SessionStore {
    conn: Connection,
}

#[derive(Debug)]
pub struct StoredMessage {
    pub role: String,
    pub content: String,
    pub tool_name: Option<String>,
    pub tool_args: Option<String>,
    pub tool_use_id: Option<String>,
    pub is_error: bool,
    pub created_at: String,
    /// Path served by /api/screenshots/<filename> — set for tool_result
    /// messages that produced an image (screenshot, ui_inspect/
    /// screenshot_sf, etc.). Runtime writes the image to disk when the
    /// tool returns, then passes the URL through append_tool_result so
    /// the resume path can re-render it. None for text-only results.
    pub image_url: Option<String>,
}

#[derive(Debug)]
pub struct SessionSummary {
    pub id: String,
    pub started_at: String,
    pub task: String,
    pub status: String,
    pub iterations: i64,
}

impl SessionStore {
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
            "CREATE TABLE IF NOT EXISTS sessions (
                id TEXT PRIMARY KEY,
                started_at TEXT NOT NULL,
                task TEXT NOT NULL,
                status TEXT NOT NULL DEFAULT 'running',
                iterations INTEGER DEFAULT 0,
                completed_at TEXT
            );

            CREATE TABLE IF NOT EXISTS messages (
                id TEXT PRIMARY KEY,
                session_id TEXT NOT NULL,
                role TEXT NOT NULL,
                content TEXT,
                tool_name TEXT,
                tool_args TEXT,
                tool_use_id TEXT,
                is_error INTEGER DEFAULT 0,
                image_url TEXT,
                created_at TEXT NOT NULL,
                FOREIGN KEY (session_id) REFERENCES sessions(id)
            );

            CREATE INDEX IF NOT EXISTS idx_messages_session ON messages(session_id);
            "
        )?;
        // Migrate old DBs: ADD COLUMN is idempotent via the try/ignore
        // pattern — it errors with "duplicate column" when already
        // present, which we discard. Fresh installs hit the CREATE
        // TABLE above with the column already included, so this is a
        // no-op on them.
        let _ = self.conn.execute("ALTER TABLE messages ADD COLUMN image_url TEXT", []);
        Ok(())
    }

    pub fn create_session(&self, task: &str) -> anyhow::Result<String> {
        let id = Uuid::new_v4().to_string();
        let now = Utc::now().to_rfc3339();
        self.conn.execute(
            "INSERT INTO sessions (id, started_at, task, status) VALUES (?1, ?2, ?3, 'running')",
            params![id, now, task],
        )?;
        Ok(id)
    }

    pub fn append_message(&self, session_id: &str, message: &Message) -> anyhow::Result<()> {
        let id = Uuid::new_v4().to_string();
        let now = Utc::now().to_rfc3339();

        match message {
            Message::System(text) => {
                self.conn.execute(
                    "INSERT INTO messages (id, session_id, role, content, created_at) VALUES (?1, ?2, 'system', ?3, ?4)",
                    params![id, session_id, text, now],
                )?;
            }
            Message::User(text) => {
                self.conn.execute(
                    "INSERT INTO messages (id, session_id, role, content, created_at) VALUES (?1, ?2, 'user', ?3, ?4)",
                    params![id, session_id, text, now],
                )?;
            }
            Message::Assistant { text, tool_calls } => {
                let content = text.clone().unwrap_or_default();
                let tool_args = if tool_calls.is_empty() {
                    None
                } else {
                    Some(serde_json::to_string(tool_calls)?)
                };
                self.conn.execute(
                    "INSERT INTO messages (id, session_id, role, content, tool_args, created_at) VALUES (?1, ?2, 'assistant', ?3, ?4, ?5)",
                    params![id, session_id, content, tool_args, now],
                )?;
            }
            Message::ToolResult { tool_use_id, name, content, is_error, .. } => {
                self.conn.execute(
                    "INSERT INTO messages (id, session_id, role, content, tool_name, tool_use_id, is_error, created_at) VALUES (?1, ?2, 'tool_result', ?3, ?4, ?5, ?6, ?7)",
                    params![id, session_id, content, name, tool_use_id, *is_error as i32, now],
                )?;
            }
        }
        Ok(())
    }

    /// Specialised path for tool-result messages that produced an image
    /// (screenshot, ui_inspect screenshot_sf). Stores `image_url`
    /// alongside the usual columns so the resume path can render the
    /// image — the default `append_message` route above leaves
    /// image_url NULL because the `Message::ToolResult` enum carries
    /// raw ImageData (base64) that's not suitable for sqlite storage,
    /// and runtime.rs already saves the decoded bytes to a file whose
    /// URL we accept here.
    ///
    /// Separate from `append_message` to avoid changing that method's
    /// signature (6 call sites, only 2 of which deal with images).
    pub fn append_tool_result(
        &self,
        session_id: &str,
        tool_use_id: &str,
        tool_name: &str,
        content: &str,
        is_error: bool,
        image_url: Option<&str>,
    ) -> anyhow::Result<()> {
        let id = Uuid::new_v4().to_string();
        let now = Utc::now().to_rfc3339();
        self.conn.execute(
            "INSERT INTO messages \
               (id, session_id, role, content, tool_name, tool_use_id, is_error, image_url, created_at) \
               VALUES (?1, ?2, 'tool_result', ?3, ?4, ?5, ?6, ?7, ?8)",
            params![id, session_id, content, tool_name, tool_use_id, is_error as i32, image_url, now],
        )?;
        Ok(())
    }

    pub fn update_status(&self, session_id: &str, status: &str, iterations: usize) -> anyhow::Result<()> {
        let now = Utc::now().to_rfc3339();
        self.conn.execute(
            "UPDATE sessions SET status = ?1, iterations = ?2, completed_at = ?3 WHERE id = ?4",
            params![status, iterations as i64, now, session_id],
        )?;
        Ok(())
    }

    pub fn load_messages(&self, session_id: &str) -> anyhow::Result<Vec<StoredMessage>> {
        let mut stmt = self.conn.prepare(
            "SELECT role, content, tool_name, tool_args, tool_use_id, is_error, image_url, created_at \
             FROM messages WHERE session_id = ?1 ORDER BY created_at ASC"
        )?;
        let rows = stmt.query_map(params![session_id], |row| {
            Ok(StoredMessage {
                role: row.get(0)?,
                content: row.get::<_, Option<String>>(1)?.unwrap_or_default(),
                tool_name: row.get(2)?,
                tool_args: row.get(3)?,
                tool_use_id: row.get(4)?,
                is_error: row.get::<_, i32>(5).unwrap_or(0) != 0,
                // image_url at position 6; row.get will return None for
                // older rows that predate the column.
                image_url: row.get::<_, Option<String>>(6).ok().flatten(),
                created_at: row.get(7)?,
            })
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    pub fn delete_session(&self, session_id: &str) -> anyhow::Result<()> {
        self.conn.execute("DELETE FROM messages WHERE session_id = ?1", params![session_id])?;
        self.conn.execute("DELETE FROM sessions WHERE id = ?1", params![session_id])?;
        Ok(())
    }

    pub fn recent_sessions(&self, limit: usize) -> anyhow::Result<Vec<SessionSummary>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, started_at, task, status, iterations FROM sessions ORDER BY started_at DESC LIMIT ?1"
        )?;
        let rows = stmt.query_map(params![limit as i64], |row| {
            Ok(SessionSummary {
                id: row.get(0)?,
                started_at: row.get(1)?,
                task: row.get(2)?,
                status: row.get(3)?,
                iterations: row.get(4)?,
            })
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_session() {
        let store = SessionStore::open_in_memory().unwrap();
        let id = store.create_session("test task").unwrap();
        assert!(!id.is_empty());

        let sessions = store.recent_sessions(10).unwrap();
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].task, "test task");
        assert_eq!(sessions[0].status, "running");
    }

    #[test]
    fn test_append_and_update() {
        let store = SessionStore::open_in_memory().unwrap();
        let id = store.create_session("test").unwrap();

        store.append_message(&id, &Message::user("hello")).unwrap();
        store.append_message(&id, &Message::assistant_text("hi")).unwrap();
        store.update_status(&id, "completed", 2).unwrap();

        let sessions = store.recent_sessions(10).unwrap();
        assert_eq!(sessions[0].status, "completed");
        assert_eq!(sessions[0].iterations, 2);
    }
}
