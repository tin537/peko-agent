# Session Persistence

> SQLite + FTS5 for conversation history and full-text search.

---

## Purpose

`SessionStore` in [[peko-core]] persists every conversation, tool call, and outcome to SQLite. This enables:

1. **Task resumption** — reload a previous conversation and continue
2. **History search** — "When did I last send an SMS to +1234567890?"
3. **Debugging** — full audit trail of every agent action
4. **Learning** — future agents can reference past successful task patterns

## Database Schema

Located at `/data/peko/state.db`:

```sql
CREATE TABLE sessions (
    id TEXT PRIMARY KEY,          -- UUID
    started_at TEXT NOT NULL,     -- ISO 8601 timestamp
    task TEXT NOT NULL,           -- Original user input
    status TEXT NOT NULL,         -- 'running', 'completed', 'interrupted', 'error'
    iterations INTEGER DEFAULT 0,
    completed_at TEXT
);

CREATE TABLE messages (
    id TEXT PRIMARY KEY,          -- UUID
    session_id TEXT NOT NULL,     -- FK to sessions
    role TEXT NOT NULL,           -- 'system', 'user', 'assistant', 'tool_result'
    content TEXT,                 -- Text content
    tool_name TEXT,               -- For tool calls and results
    tool_args TEXT,               -- JSON string of tool arguments
    tool_use_id TEXT,             -- Links tool_result to tool_call
    is_error INTEGER DEFAULT 0,  -- For tool results
    created_at TEXT NOT NULL,
    FOREIGN KEY (session_id) REFERENCES sessions(id)
);

-- Full-text search index on message content
CREATE VIRTUAL TABLE messages_fts USING fts5(
    content,
    tool_name,
    tool_args,
    content='messages',
    content_rowid='rowid'
);

-- Triggers to keep FTS index in sync
CREATE TRIGGER messages_ai AFTER INSERT ON messages BEGIN
    INSERT INTO messages_fts(rowid, content, tool_name, tool_args)
    VALUES (new.rowid, new.content, new.tool_name, new.tool_args);
END;
```

## FTS5 Search

SQLite's FTS5 extension enables fast full-text search across all past conversations:

```rust
impl SessionStore {
    /// Search across all messages
    pub fn search(&self, query: &str) -> Result<Vec<SearchResult>> {
        let sql = "SELECT m.*, s.task FROM messages_fts
                   JOIN messages m ON messages_fts.rowid = m.rowid
                   JOIN sessions s ON m.session_id = s.id
                   WHERE messages_fts MATCH ?
                   ORDER BY rank";
        // ...
    }
}
```

Example queries:
- `"SMS +1234567890"` — find all SMS-related messages to this number
- `"error modem"` — find modem errors
- `"screenshot settings"` — find when the agent navigated to settings

## SessionStore API

```rust
pub struct SessionStore {
    conn: rusqlite::Connection,
}

impl SessionStore {
    /// Open or create database at path
    pub fn open(path: &Path) -> Result<Self>;

    /// Create a new session
    pub fn create_session(&self, task: &str) -> Result<String>;

    /// Append a message to a session
    pub fn append_message(&self, session_id: &str, message: &Message) -> Result<()>;

    /// Update session status
    pub fn update_status(&self, session_id: &str, status: &str) -> Result<()>;

    /// Load full conversation for a session
    pub fn load_conversation(&self, session_id: &str) -> Result<Vec<Message>>;

    /// Full-text search across all sessions
    pub fn search(&self, query: &str) -> Result<Vec<SearchResult>>;

    /// List recent sessions
    pub fn recent_sessions(&self, limit: usize) -> Result<Vec<SessionSummary>>;
}
```

## Data Flow

```
Agent loop iteration:
  1. LLM returns assistant message with tool calls
     → store Message::Assistant { text, tool_calls }

  2. Each tool executes
     → store Message::ToolResult { name, content, is_error }

  3. Task completes
     → update session status to 'completed'
```

Everything is persisted **immediately** — if the process crashes, the database has a complete record up to the last completed operation.

## Why SQLite?

| Alternative | Why not |
|---|---|
| Flat files | No search, no transactions, messy |
| RocksDB | Overkill, larger binary, no SQL |
| Postgres/MySQL | Requires a server process — defeats the single-binary goal |

SQLite fits perfectly:
- Single file, zero server processes
- `rusqlite` with `bundled` feature compiles SQLite into the binary
- FTS5 provides powerful full-text search
- ACID transactions protect against data corruption on crash
- Tiny memory footprint

## Storage Estimates

| Content | Size per instance | 50 iterations |
|---|---|---|
| Text message | ~200 bytes | ~10 KB |
| Tool call (no screenshot) | ~500 bytes | ~25 KB |
| Screenshot (base64 PNG) | ~500 KB | ~25 MB |

For long-running agents, screenshots dominate storage. Consider:
- Storing screenshots as separate files, with only the path in SQLite
- Compressing old screenshots
- Pruning sessions older than N days

## Related

- [[peko-core]] — Where SessionStore lives
- [[ReAct-Loop]] — When messages are persisted
- [[Context-Compression]] — Compressed conversation still searchable in full DB

---

#implementation #persistence #sqlite #storage
