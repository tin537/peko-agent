package com.peko.overlay.bridge

import android.content.ContentValues
import android.content.Context
import android.database.sqlite.SQLiteDatabase
import android.database.sqlite.SQLiteOpenHelper
import org.json.JSONArray
import org.json.JSONObject
import java.io.File

/**
 * Single source of truth for streaming events emitted by every bridge
 * service (GPS samples, telephony deltas, camera frame metadata,
 * ambient-audio features). peko-agent polls via raw SQLite — root
 * can read priv-app private dirs even though `other` perm bits are 0.
 *
 * Schema:
 *   events(id INTEGER PK, ts INTEGER, type TEXT, source TEXT,
 *          data_json TEXT, asset_path TEXT)
 *
 * - `ts` is millis since epoch — monotonic enough for "since" polling.
 * - `type` is a coarse category ("location", "signal", "frame", "ambient").
 * - `source` distinguishes between e.g. "gps_stream:abc" vs "camera_stream:def".
 * - `data_json` carries everything else; deliberately schemaless so each
 *    pipeline can evolve independently.
 * - `asset_path` is the on-disk path to the heavy payload (jpeg, wav)
 *    when the event has one; otherwise null.
 *
 * Auto-prune: rows older than 24h are dropped on each insert (cheap;
 * a single index lookup).
 */
class EventStore private constructor(ctx: Context) :
    SQLiteOpenHelper(ctx, DB_NAME, null, DB_VERSION) {

    override fun onCreate(db: SQLiteDatabase) {
        db.execSQL("""
            CREATE TABLE IF NOT EXISTS events (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                ts INTEGER NOT NULL,
                type TEXT NOT NULL,
                source TEXT NOT NULL,
                data_json TEXT NOT NULL,
                asset_path TEXT
            )
        """.trimIndent())
        db.execSQL("CREATE INDEX IF NOT EXISTS idx_events_ts ON events(ts)")
        db.execSQL("CREATE INDEX IF NOT EXISTS idx_events_type_ts ON events(type, ts)")
    }

    override fun onUpgrade(db: SQLiteDatabase, oldV: Int, newV: Int) {
        // Future migrations.
    }

    fun append(type: String, source: String, data: JSONObject, assetPath: String? = null): Long {
        val cv = ContentValues().apply {
            put("ts", System.currentTimeMillis())
            put("type", type)
            put("source", source)
            put("data_json", data.toString())
            if (assetPath != null) put("asset_path", assetPath)
        }
        val id = writableDatabase.insert("events", null, cv)
        // Cheap bounded prune: drop anything older than 24h.
        try {
            writableDatabase.delete(
                "events", "ts < ?",
                arrayOf((System.currentTimeMillis() - 24L * 3600_000L).toString()),
            )
        } catch (_: Throwable) {}
        return id
    }

    fun query(sinceTs: Long, type: String?, limit: Int): JSONArray {
        val arr = JSONArray()
        val cols = arrayOf("id", "ts", "type", "source", "data_json", "asset_path")
        val (sel, args) = if (type != null)
            "ts > ? AND type = ?" to arrayOf(sinceTs.toString(), type)
        else
            "ts > ?" to arrayOf(sinceTs.toString())
        readableDatabase.query(
            "events", cols, sel, args,
            null, null, "ts ASC", limit.coerceIn(1, 1000).toString(),
        ).use { c ->
            while (c.moveToNext()) {
                val row = JSONObject()
                    .put("id", c.getLong(0))
                    .put("ts", c.getLong(1))
                    .put("type", c.getString(2))
                    .put("source", c.getString(3))
                    .put("data", JSONObject(c.getString(4)))
                if (!c.isNull(5)) row.put("asset_path", c.getString(5))
                arr.put(row)
            }
        }
        return arr
    }

    companion object {
        private const val DB_NAME = "events.db"
        private const val DB_VERSION = 1

        @Volatile private var instance: EventStore? = null
        fun get(ctx: Context): EventStore =
            instance ?: synchronized(this) {
                instance ?: EventStore(ctx.applicationContext).also { instance = it }
            }

        /** Path the agent reads via root sqlite. */
        fun dbPath(ctx: Context): File =
            ctx.applicationContext.getDatabasePath(DB_NAME)
    }
}
