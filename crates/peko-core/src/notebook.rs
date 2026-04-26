//! Second-brain / knowledge-graph store.
//!
//! Two layers, both durable, both queryable:
//!
//!   1. **SQLite (`notes` + `note_links` + FTS5 virtual table)**
//!      The fast index. Every note has a row + an FTS shadow. Wikilinks
//!      `[[other_note]]` parsed at write time and materialised into
//!      `note_links(source_id, target_slug, context)` so backlinks +
//!      1-hop graph expansion are cheap joins.
//!
//!   2. **Markdown files in `<data_dir>/brain/<slug>.md`**
//!      Source-of-truth on disk. Frontmatter holds metadata. A user
//!      can `cd /data/peko/brain` and read or edit notes by hand;
//!      next agent write rehydrates the index from the file. Git-able.
//!      Survives DB corruption.
//!
//! Hybrid by design: the FTS path ships in this module (Phase 18A);
//! semantic embeddings layer plugs into the same schema later via the
//! reserved `embedding BLOB` column (Phase 19).
//!
//! Naming: the Rust module is `notebook` because `peko_core::brain`
//! is already taken by the dual-brain router. The user-facing tool +
//! config still call it "brain" / "second brain".

use anyhow::Context;
use chrono::Utc;
use rusqlite::{params, Connection};
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct Note {
    pub id: String,
    pub slug: String,
    pub title: String,
    pub content: String,
    pub kind: String,
    pub tags: Vec<String>,
    pub created_at: String,
    pub updated_at: String,
}

pub struct BrainStore {
    conn: Connection,
    /// Filesystem root for markdown export. Notes land at
    /// `brain_dir.join(format!("{slug}.md"))`. Created on first write.
    brain_dir: PathBuf,
}

impl BrainStore {
    pub fn open(db_path: &Path, brain_dir: &Path) -> anyhow::Result<Self> {
        let conn = Connection::open(db_path)
            .with_context(|| format!("open brain db at {}", db_path.display()))?;
        fs::create_dir_all(brain_dir)
            .with_context(|| format!("mkdir brain_dir {}", brain_dir.display()))?;
        let store = Self { conn, brain_dir: brain_dir.to_path_buf() };
        store.init_schema()?;
        Ok(store)
    }

    pub fn open_in_memory() -> anyhow::Result<Self> {
        let conn = Connection::open_in_memory()?;
        let dir = std::env::temp_dir().join(format!("peko-brain-mem-{}", rand_token()));
        fs::create_dir_all(&dir)?;
        let store = Self { conn, brain_dir: dir };
        store.init_schema()?;
        Ok(store)
    }

    fn init_schema(&self) -> anyhow::Result<()> {
        self.conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS notes (
                id TEXT PRIMARY KEY,
                slug TEXT NOT NULL UNIQUE,
                title TEXT NOT NULL,
                content TEXT NOT NULL,
                kind TEXT NOT NULL DEFAULT 'note',
                tags TEXT NOT NULL DEFAULT '',
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                embedding BLOB,
                embedding_model TEXT
            );

            CREATE INDEX IF NOT EXISTS idx_notes_kind ON notes(kind);
            CREATE INDEX IF NOT EXISTS idx_notes_updated ON notes(updated_at DESC);

            CREATE VIRTUAL TABLE IF NOT EXISTS notes_fts USING fts5(
                slug, title, content, tags,
                content='notes',
                content_rowid='rowid'
            );

            CREATE TRIGGER IF NOT EXISTS notes_ai AFTER INSERT ON notes BEGIN
                INSERT INTO notes_fts(rowid, slug, title, content, tags)
                VALUES (new.rowid, new.slug, new.title, new.content, new.tags);
            END;
            CREATE TRIGGER IF NOT EXISTS notes_ad AFTER DELETE ON notes BEGIN
                INSERT INTO notes_fts(notes_fts, rowid, slug, title, content, tags)
                VALUES ('delete', old.rowid, old.slug, old.title, old.content, old.tags);
            END;
            CREATE TRIGGER IF NOT EXISTS notes_au AFTER UPDATE ON notes BEGIN
                INSERT INTO notes_fts(notes_fts, rowid, slug, title, content, tags)
                VALUES ('delete', old.rowid, old.slug, old.title, old.content, old.tags);
                INSERT INTO notes_fts(rowid, slug, title, content, tags)
                VALUES (new.rowid, new.slug, new.title, new.content, new.tags);
            END;

            CREATE TABLE IF NOT EXISTS note_links (
                source_id TEXT NOT NULL,
                target_slug TEXT NOT NULL,
                context TEXT,
                created_at TEXT NOT NULL,
                PRIMARY KEY (source_id, target_slug)
            );
            CREATE INDEX IF NOT EXISTS idx_note_links_target ON note_links(target_slug);"
        )?;
        Ok(())
    }

    /// Create or update a note. Slug is derived from the title; passing
    /// the same title twice updates in place. Wikilinks `[[Other Note]]`
    /// in `content` are parsed and stored in `note_links` so backlink
    /// queries are cheap.
    pub fn save(
        &self,
        title: &str,
        content: &str,
        kind: Option<&str>,
        tags: &[String],
    ) -> anyhow::Result<Note> {
        let now = Utc::now().to_rfc3339();
        let slug = slugify(title);
        if slug.is_empty() {
            anyhow::bail!("title produced empty slug: '{title}'");
        }
        let kind = kind.unwrap_or("note").to_string();
        let tags_str = tags.join(",");

        // Upsert by slug. Reuse existing id so backlinks survive renames.
        let existing_id: Option<String> = self.conn.query_row(
            "SELECT id FROM notes WHERE slug = ?1",
            params![slug],
            |r| r.get(0),
        ).ok();

        let id = match existing_id {
            Some(id) => {
                self.conn.execute(
                    "UPDATE notes SET title = ?1, content = ?2, kind = ?3, tags = ?4, updated_at = ?5 WHERE id = ?6",
                    params![title, content, kind, tags_str, now, id],
                )?;
                id
            }
            None => {
                let id = Uuid::new_v4().to_string();
                self.conn.execute(
                    "INSERT INTO notes (id, slug, title, content, kind, tags, created_at, updated_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7)",
                    params![id, slug, title, content, kind, tags_str, now],
                )?;
                id
            }
        };

        // Refresh links: drop existing rows for this source, insert fresh ones.
        self.conn.execute(
            "DELETE FROM note_links WHERE source_id = ?1",
            params![id],
        )?;
        for (target_title, ctx) in parse_wikilinks(content) {
            let target_slug = slugify(&target_title);
            if target_slug.is_empty() {
                continue;
            }
            self.conn.execute(
                "INSERT OR IGNORE INTO note_links (source_id, target_slug, context, created_at)
                 VALUES (?1, ?2, ?3, ?4)",
                params![id, target_slug, ctx, now],
            )?;
        }

        // Markdown export.
        let path = self.brain_dir.join(format!("{slug}.md"));
        let frontmatter = format!(
            "---\nid: {id}\ntitle: {}\nslug: {slug}\nkind: {kind}\ntags: [{}]\ncreated_at: {}\nupdated_at: {now}\n---\n\n",
            yaml_escape(title),
            tags.iter().map(|t| yaml_escape(t)).collect::<Vec<_>>().join(", "),
            self.created_at_for(&id).unwrap_or_else(|| now.clone()),
        );
        fs::write(&path, format!("{frontmatter}{content}"))
            .with_context(|| format!("write {}", path.display()))?;

        Ok(Note {
            id,
            slug,
            title: title.to_string(),
            content: content.to_string(),
            kind,
            tags: tags.to_vec(),
            created_at: now.clone(),
            updated_at: now,
        })
    }

    fn created_at_for(&self, id: &str) -> Option<String> {
        self.conn.query_row(
            "SELECT created_at FROM notes WHERE id = ?1",
            params![id],
            |r| r.get(0),
        ).ok()
    }

    pub fn get(&self, slug_or_id: &str) -> anyhow::Result<Option<Note>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, slug, title, content, kind, tags, created_at, updated_at
             FROM notes WHERE id = ?1 OR slug = ?1 LIMIT 1",
        )?;
        let mut rows = stmt.query(params![slug_or_id])?;
        if let Some(row) = rows.next()? {
            return Ok(Some(row_to_note(row)?));
        }
        Ok(None)
    }

    /// FTS5 search. When `expand_links` is true, also include 1-hop
    /// graph neighbours of the top hits (notes that link TO or FROM
    /// the matched notes). Useful for "show me everything related to X".
    pub fn search(
        &self,
        query: &str,
        limit: usize,
        expand_links: bool,
    ) -> anyhow::Result<Vec<Note>> {
        let mut stmt = self.conn.prepare(
            "SELECT n.id, n.slug, n.title, n.content, n.kind, n.tags, n.created_at, n.updated_at
             FROM notes_fts f
             JOIN notes n ON n.rowid = f.rowid
             WHERE notes_fts MATCH ?1
             ORDER BY rank
             LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![query, limit as i64], row_to_note)?;
        let mut out: Vec<Note> = Vec::new();
        let mut seen: HashSet<String> = HashSet::new();
        for r in rows {
            let n = r?;
            seen.insert(n.id.clone());
            out.push(n);
        }
        if expand_links {
            let direct_ids: Vec<String> = out.iter().map(|n| n.id.clone()).collect();
            for src_id in direct_ids {
                for neighbour in self.linked_from_internal(&src_id)? {
                    if seen.insert(neighbour.id.clone()) {
                        out.push(neighbour);
                    }
                }
                // Also pull backlinks (notes that link TO this one).
                if let Some(slug) = out.iter().find(|n| n.id == src_id).map(|n| n.slug.clone()) {
                    for neighbour in self.linked_to(&slug)? {
                        if seen.insert(neighbour.id.clone()) {
                            out.push(neighbour);
                        }
                    }
                }
            }
        }
        Ok(out)
    }

    /// Notes that link TO this slug (backlinks).
    pub fn linked_to(&self, slug: &str) -> anyhow::Result<Vec<Note>> {
        let mut stmt = self.conn.prepare(
            "SELECT n.id, n.slug, n.title, n.content, n.kind, n.tags, n.created_at, n.updated_at
             FROM note_links l
             JOIN notes n ON n.id = l.source_id
             WHERE l.target_slug = ?1
             ORDER BY n.updated_at DESC",
        )?;
        let rows = stmt.query_map(params![slug], row_to_note)?;
        rows.map(|r| r.map_err(Into::into)).collect()
    }

    fn linked_from_internal(&self, source_id: &str) -> anyhow::Result<Vec<Note>> {
        let mut stmt = self.conn.prepare(
            "SELECT n.id, n.slug, n.title, n.content, n.kind, n.tags, n.created_at, n.updated_at
             FROM note_links l
             JOIN notes n ON n.slug = l.target_slug
             WHERE l.source_id = ?1
             ORDER BY n.updated_at DESC",
        )?;
        let rows = stmt.query_map(params![source_id], row_to_note)?;
        rows.map(|r| r.map_err(Into::into)).collect()
    }

    pub fn linked_from(&self, slug: &str) -> anyhow::Result<Vec<Note>> {
        let id: Option<String> = self.conn.query_row(
            "SELECT id FROM notes WHERE slug = ?1",
            params![slug],
            |r| r.get(0),
        ).ok();
        match id {
            Some(id) => self.linked_from_internal(&id),
            None => Ok(Vec::new()),
        }
    }

    pub fn list_recent(&self, limit: usize, kind: Option<&str>) -> anyhow::Result<Vec<Note>> {
        let (sql, p): (&str, Vec<rusqlite::types::Value>) = match kind {
            Some(k) => (
                "SELECT id, slug, title, content, kind, tags, created_at, updated_at
                 FROM notes WHERE kind = ?1 ORDER BY updated_at DESC LIMIT ?2",
                vec![k.to_string().into(), (limit as i64).into()],
            ),
            None => (
                "SELECT id, slug, title, content, kind, tags, created_at, updated_at
                 FROM notes ORDER BY updated_at DESC LIMIT ?1",
                vec![(limit as i64).into()],
            ),
        };
        let mut stmt = self.conn.prepare(sql)?;
        let rows = stmt.query_map(rusqlite::params_from_iter(p.iter()), row_to_note)?;
        rows.map(|r| r.map_err(Into::into)).collect()
    }

    pub fn delete(&self, slug_or_id: &str) -> anyhow::Result<bool> {
        let row: Option<(String, String)> = self.conn.query_row(
            "SELECT id, slug FROM notes WHERE id = ?1 OR slug = ?1 LIMIT 1",
            params![slug_or_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        ).ok();
        let Some((id, slug)) = row else { return Ok(false) };
        self.conn.execute("DELETE FROM note_links WHERE source_id = ?1", params![id])?;
        self.conn.execute("DELETE FROM notes WHERE id = ?1", params![id])?;
        let path = self.brain_dir.join(format!("{slug}.md"));
        let _ = fs::remove_file(path);
        Ok(true)
    }

    pub fn stats(&self) -> anyhow::Result<BrainStats> {
        let note_count: i64 = self.conn.query_row("SELECT COUNT(*) FROM notes", [], |r| r.get(0))?;
        let link_count: i64 = self.conn.query_row("SELECT COUNT(*) FROM note_links", [], |r| r.get(0))?;
        let dangling: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM note_links l
             WHERE NOT EXISTS (SELECT 1 FROM notes n WHERE n.slug = l.target_slug)",
            [], |r| r.get(0),
        )?;
        Ok(BrainStats { note_count, link_count, dangling_links: dangling })
    }
}

#[derive(Debug, Clone)]
pub struct BrainStats {
    pub note_count: i64,
    pub link_count: i64,
    pub dangling_links: i64,
}

fn row_to_note(row: &rusqlite::Row) -> rusqlite::Result<Note> {
    let tags_str: String = row.get(5)?;
    Ok(Note {
        id: row.get(0)?,
        slug: row.get(1)?,
        title: row.get(2)?,
        content: row.get(3)?,
        kind: row.get(4)?,
        tags: tags_str.split(',').filter(|s| !s.is_empty()).map(String::from).collect(),
        created_at: row.get(6)?,
        updated_at: row.get(7)?,
    })
}

/// Convert a free-form title into a URL-safe slug. Lowercases ASCII,
/// keeps Unicode letters as-is (so Thai titles get usable filenames),
/// replaces whitespace + punctuation with `-`, collapses runs.
pub fn slugify(title: &str) -> String {
    let mut out = String::with_capacity(title.len());
    let mut last_was_dash = true;
    for c in title.chars() {
        if c.is_alphanumeric() {
            for lc in c.to_lowercase() {
                out.push(lc);
            }
            last_was_dash = false;
        } else if c.is_whitespace() || c == '-' || c == '_' || c.is_ascii_punctuation() {
            if !last_was_dash && !out.is_empty() {
                out.push('-');
                last_was_dash = true;
            }
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    // Char-aware truncation — Thai / CJK glyphs are 3 bytes each, so
    // `out.truncate(80)` (byte index) panics when the cut lands inside
    // a codepoint. Same class of bug as the Phase 13 compressor fix.
    // Cap at 80 CHARS, which is also a more meaningful filename limit.
    if out.chars().count() > 80 {
        out = out.chars().take(80).collect();
        while out.ends_with('-') {
            out.pop();
        }
    }
    out
}

/// Parse `[[wikilink]]` patterns out of markdown. Returns (link_text,
/// surrounding_context). Only matches simple `[[...]]` — no piped
/// `[[a|b]]` syntax for now (target_text == display_text always).
pub fn parse_wikilinks(content: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let bytes = content.as_bytes();
    let mut i = 0;
    while i + 1 < bytes.len() {
        if bytes[i] == b'[' && bytes[i + 1] == b'[' {
            let start = i + 2;
            let mut end = start;
            while end + 1 < bytes.len() && !(bytes[end] == b']' && bytes[end + 1] == b']') {
                end += 1;
            }
            if end + 1 < bytes.len() {
                if let Ok(link) = std::str::from_utf8(&bytes[start..end]) {
                    // Disallow nested newlines or [[ — guards against
                    // malformed syntax.
                    if !link.contains('\n') && !link.contains("[[") && !link.is_empty() {
                        let ctx_start = start.saturating_sub(40);
                        let ctx_end = (end + 2 + 40).min(bytes.len());
                        let ctx = std::str::from_utf8(&bytes[ctx_start..ctx_end])
                            .unwrap_or("")
                            .replace('\n', " ");
                        // Pipe split: [[target|display]] — keep target.
                        let target = match link.split_once('|') {
                            Some((t, _)) => t.trim().to_string(),
                            None => link.trim().to_string(),
                        };
                        out.push((target, ctx));
                    }
                }
                i = end + 2;
                continue;
            }
        }
        i += 1;
    }
    out
}

fn yaml_escape(s: &str) -> String {
    if s.contains([':', '"', '\n', '\'']) {
        format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
    } else {
        s.to_string()
    }
}

fn rand_token() -> String {
    let pid = std::process::id();
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    format!("{pid}-{nanos}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fresh() -> BrainStore {
        BrainStore::open_in_memory().unwrap()
    }

    #[test]
    fn slugify_lowercases_and_dashes() {
        assert_eq!(slugify("Hello World"), "hello-world");
        assert_eq!(slugify("How does PLL work?"), "how-does-pll-work");
        assert_eq!(slugify("--leading---trailing--"), "leading-trailing");
        assert_eq!(slugify("a/b\\c"), "a-b-c");
    }

    #[test]
    fn slugify_preserves_thai_characters() {
        // Thai shouldn't be stripped — it's alphanumeric per Rust's
        // is_alphanumeric.
        let s = slugify("ระบบ AI");
        assert!(s.contains("ระบบ"));
        assert!(s.contains("ai"));
    }

    #[test]
    fn slugify_empty_safe() {
        assert_eq!(slugify(""), "");
        assert_eq!(slugify("---"), "");
    }

    #[test]
    fn slugify_handles_long_thai_titles_no_panic() {
        // Regression: byte-indexed `out.truncate(80)` panics when the
        // cut lands inside a multi-byte codepoint. Thai chars are 3
        // bytes; a 100-char Thai title hits 300 bytes and slicing at
        // byte 80 lands mid-glyph. Real-world reproducer that crashed
        // the agent during a research task.
        let long_thai = "การวิจัยเรื่องการสร้างเอเจนต์ AI อัตโนมัติทางการตลาดที่สามารถสร้างรายได้ได้จริง ปี 2025 และต่อไป".repeat(2);
        let s = slugify(&long_thai);
        assert!(s.chars().count() <= 80);
        // Must be valid UTF-8 (slicing on byte boundary would have
        // corrupted it pre-fix; with chars().take() we always end on
        // a codepoint).
        assert!(std::str::from_utf8(s.as_bytes()).is_ok());
    }

    #[test]
    fn parse_wikilinks_simple() {
        let links = parse_wikilinks("Hello [[World]] and [[Other Note]] here.");
        assert_eq!(links.len(), 2);
        assert_eq!(links[0].0, "World");
        assert_eq!(links[1].0, "Other Note");
    }

    #[test]
    fn parse_wikilinks_handles_pipes() {
        let links = parse_wikilinks("See [[real-target|display name]].");
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].0, "real-target");
    }

    #[test]
    fn parse_wikilinks_ignores_malformed() {
        let links = parse_wikilinks("[[unclosed text");
        assert!(links.is_empty());
        let links = parse_wikilinks("[[]]");
        assert!(links.is_empty());
    }

    #[test]
    fn save_creates_md_file_with_frontmatter() {
        let store = fresh();
        let n = store
            .save("My First Note", "Hello world", None, &["test".into()])
            .unwrap();
        let path = store.brain_dir.join(format!("{}.md", n.slug));
        assert!(path.exists(), "markdown file should exist at {}", path.display());
        let body = std::fs::read_to_string(&path).unwrap();
        assert!(body.starts_with("---"));
        assert!(body.contains(&format!("id: {}", n.id)));
        assert!(body.contains("title: My First Note"));
        assert!(body.contains("Hello world"));
    }

    #[test]
    fn save_upserts_on_same_slug() {
        let store = fresh();
        let a = store.save("Same Title", "First", None, &[]).unwrap();
        let b = store.save("Same Title", "Updated", None, &[]).unwrap();
        assert_eq!(a.id, b.id, "second save should reuse id");
        assert_eq!(b.content, "Updated");
        let stats = store.stats().unwrap();
        assert_eq!(stats.note_count, 1);
    }

    #[test]
    fn search_returns_fts_hits() {
        let store = fresh();
        store.save("Battery thermal management", "thermal runaway lithium", None, &[]).unwrap();
        store.save("Wifi signal walkthrough", "rssi dbm", None, &[]).unwrap();
        let hits = store.search("thermal", 10, false).unwrap();
        assert_eq!(hits.len(), 1);
        assert!(hits[0].title.contains("Battery"));
    }

    #[test]
    fn wikilinks_create_backlinks() {
        let store = fresh();
        store.save("Source A", "Refers to [[Target]].", None, &[]).unwrap();
        store.save("Source B", "Also discusses [[Target]] briefly.", None, &[]).unwrap();
        store.save("Target", "Receives links.", None, &[]).unwrap();
        let backlinks = store.linked_to("target").unwrap();
        assert_eq!(backlinks.len(), 2);
        let titles: Vec<_> = backlinks.iter().map(|n| n.title.clone()).collect();
        assert!(titles.contains(&"Source A".to_string()));
        assert!(titles.contains(&"Source B".to_string()));
    }

    #[test]
    fn search_with_expand_pulls_neighbours() {
        let store = fresh();
        store.save("Alpha", "links to [[Beta]].", None, &[]).unwrap();
        store.save("Beta", "no links here, just text.", None, &[]).unwrap();
        let no_expand = store.search("alpha", 10, false).unwrap();
        assert_eq!(no_expand.len(), 1);
        let with_expand = store.search("alpha", 10, true).unwrap();
        assert!(with_expand.iter().any(|n| n.title == "Beta"));
    }

    #[test]
    fn delete_removes_md_and_links() {
        let store = fresh();
        let a = store.save("Doomed", "links to [[Survivor]]", None, &[]).unwrap();
        store.save("Survivor", "stays", None, &[]).unwrap();
        let path = store.brain_dir.join(format!("{}.md", a.slug));
        assert!(path.exists());
        let removed = store.delete("doomed").unwrap();
        assert!(removed);
        assert!(!path.exists());
        let stats = store.stats().unwrap();
        assert_eq!(stats.note_count, 1);
        assert_eq!(stats.link_count, 0);
    }

    #[test]
    fn stats_count_dangling_links() {
        let store = fresh();
        store.save("Source", "points to [[Nonexistent]].", None, &[]).unwrap();
        let s = store.stats().unwrap();
        assert_eq!(s.note_count, 1);
        assert_eq!(s.link_count, 1);
        assert_eq!(s.dangling_links, 1);
    }
}
