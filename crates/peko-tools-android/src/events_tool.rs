//! Phase 23 — events poller. Reads the shared `events.db` SQLite
//! database written by every streaming bridge service (camera frames,
//! GPS samples, ambient audio features, telephony events). peko-agent
//! runs as root so it can open the priv-app's database directly.

use crate::bridge_client::events_db_path;
use peko_core::tool::{Tool, ToolResult};
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Value};
use std::future::Future;
use std::pin::Pin;

pub struct EventsTool;
impl EventsTool { pub fn new() -> Self { Self } }
impl Default for EventsTool { fn default() -> Self { Self::new() } }

impl Tool for EventsTool {
    fn name(&self) -> &str { "events" }

    fn description(&self) -> &str {
        "Poll the streaming-event log written by the PekoOverlay bridge. \
         Every camera frame, GPS sample, ambient-audio window, etc. \
         lands as a row here. Cheap to call; meant for a polling loop. \
         \
         Args: \
         since_ts?:int (millis since epoch) — only events newer; default \
             is 60s ago. \
         type?:string — filter by category (\"frame\", \"location\", \
             \"ambient\"). \
         source?:string — filter by source label \
             (\"camera_stream:cam-123\"). \
         limit?:int — default 50, max 1000. \
         \
         Returns { events: [...], latest_ts } where each event has \
         { id, ts, type, source, data, asset_path? }. asset_path points \
         at the on-device file for heavy payloads (camera JPEGs, etc.). \
         For a polling loop, save `latest_ts` and pass it back as \
         `since_ts` next call."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "since_ts": { "type": "integer" },
                "type": { "type": "string" },
                "source": { "type": "string" },
                "limit": { "type": "integer" }
            }
        })
    }

    fn execute(
        &self,
        args: Value,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<ToolResult>> + Send + '_>> {
        Box::pin(async move {
            let now_ms = chrono::Utc::now().timestamp_millis();
            let since_ts = args["since_ts"].as_i64().unwrap_or(now_ms - 60_000);
            let limit = args["limit"].as_i64().unwrap_or(50).clamp(1, 1000);
            let kind = args["type"].as_str().map(String::from);
            let source = args["source"].as_str().map(String::from);

            let path = events_db_path();
            if !path.exists() {
                return Ok(ToolResult::success(format!(
                    "no events yet — bridge hasn't written {} (no streams active?)",
                    path.display()
                )));
            }
            let conn = match Connection::open(&path) {
                Ok(c) => c,
                Err(e) => return Ok(ToolResult::error(format!(
                    "open {}: {e}", path.display()
                ))),
            };
            // Build dynamic query — fields are bounded so this is safe.
            let mut sql = String::from(
                "SELECT id, ts, type, source, data_json, asset_path \
                 FROM events WHERE ts > ?1"
            );
            let mut bind_idx = 2;
            if kind.is_some() { sql.push_str(&format!(" AND type = ?{}", bind_idx)); bind_idx += 1; }
            if source.is_some() { sql.push_str(&format!(" AND source = ?{}", bind_idx)); bind_idx += 1; }
            sql.push_str(&format!(" ORDER BY ts ASC LIMIT ?{}", bind_idx));
            let mut stmt = conn.prepare(&sql)?;

            let mut events: Vec<Value> = Vec::new();
            let mut latest_ts = since_ts;

            // rusqlite's variadic params handling — easier to branch.
            let row_to_event = |row: &rusqlite::Row| -> rusqlite::Result<Value> {
                let id: i64 = row.get(0)?;
                let ts: i64 = row.get(1)?;
                let kind: String = row.get(2)?;
                let source: String = row.get(3)?;
                let data_json: String = row.get(4)?;
                let asset: Option<String> = row.get(5)?;
                let data: Value = serde_json::from_str(&data_json).unwrap_or(json!({}));
                let mut ev = json!({
                    "id": id, "ts": ts, "type": kind, "source": source, "data": data,
                });
                if let Some(p) = asset { ev["asset_path"] = json!(p); }
                Ok(ev)
            };

            let rows = match (kind.as_deref(), source.as_deref()) {
                (None, None) => stmt.query_map(params![since_ts, limit], row_to_event)?.collect::<Result<Vec<_>, _>>()?,
                (Some(k), None) => stmt.query_map(params![since_ts, k, limit], row_to_event)?.collect::<Result<Vec<_>, _>>()?,
                (None, Some(s)) => stmt.query_map(params![since_ts, s, limit], row_to_event)?.collect::<Result<Vec<_>, _>>()?,
                (Some(k), Some(s)) => stmt.query_map(params![since_ts, k, s, limit], row_to_event)?.collect::<Result<Vec<_>, _>>()?,
            };
            for ev in rows {
                if let Some(t) = ev["ts"].as_i64() { latest_ts = latest_ts.max(t); }
                events.push(ev);
            }

            // Also count total events available for sanity (cheap aggregate).
            let total: i64 = conn.query_row("SELECT COUNT(*) FROM events", [], |r| r.get(0))
                .optional().ok().flatten().unwrap_or(0);

            let out = json!({
                "ok": true,
                "events": events,
                "count": events.len(),
                "latest_ts": latest_ts,
                "since_ts": since_ts,
                "total_in_db": total,
            });
            Ok(ToolResult::success(serde_json::to_string_pretty(&out).unwrap_or_default()))
        })
    }
}
