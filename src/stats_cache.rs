//! Persistent stats cache that incrementally ingests events.jsonl files.
//!
//! Stores pre-aggregated model usage counts in `~/.copilot/remo-stats-cache.db`
//! and reads only the bytes added since the last scan, avoiding full re-scans.

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::Connection;
use tracing::{error, info};

pub struct StatsCache {
    db_path: PathBuf,
    copilot_dir: PathBuf,
}

impl StatsCache {
    pub fn new() -> Self {
        let home = dirs::home_dir().unwrap_or_default();
        let copilot_dir = home.join(".copilot");
        let db_path = copilot_dir.join("remo-stats-cache.db");
        let cache = Self {
            db_path,
            copilot_dir,
        };
        cache.init_db();
        cache
    }

    fn init_db(&self) {
        let conn = match Connection::open(&self.db_path) {
            Ok(c) => c,
            Err(e) => {
                error!("Failed to open stats cache: {}", e);
                return;
            }
        };
        conn.execute_batch(
            "
            PRAGMA journal_mode=WAL;

            CREATE TABLE IF NOT EXISTS model_usage (
                model TEXT PRIMARY KEY,
                count INTEGER DEFAULT 0
            );

            CREATE TABLE IF NOT EXISTS event_counts (
                event_type TEXT PRIMARY KEY,
                count INTEGER DEFAULT 0
            );

            CREATE TABLE IF NOT EXISTS model_changes (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                model TEXT NOT NULL,
                timestamp TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS file_offsets (
                file_path TEXT PRIMARY KEY,
                byte_offset INTEGER DEFAULT 0
            );

            CREATE TABLE IF NOT EXISTS cache_meta (
                key TEXT PRIMARY KEY,
                value TEXT
            );

            CREATE TABLE IF NOT EXISTS sessions (
                id TEXT PRIMARY KEY,
                cwd TEXT,
                repository TEXT,
                branch TEXT,
                summary TEXT,
                created_at TEXT,
                updated_at TEXT,
                turn_count INTEGER DEFAULT 0
            );

            CREATE TABLE IF NOT EXISTS session_files_count (
                total INTEGER DEFAULT 0,
                unique_files INTEGER DEFAULT 0
            );

            CREATE TABLE IF NOT EXISTS repositories (
                name TEXT PRIMARY KEY,
                session_count INTEGER DEFAULT 0
            );

            CREATE TABLE IF NOT EXISTS monthly_stats (
                month TEXT PRIMARY KEY,
                sessions INTEGER DEFAULT 0,
                turns INTEGER DEFAULT 0
            );
        ",
        )
        .ok();
    }

    fn sync_session_store(&self, conn: &Connection) {
        let store_db = self.copilot_dir.join("session-store.db");
        if !store_db.exists() {
            return;
        }

        let source = match Connection::open_with_flags(
            &store_db,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
        ) {
            Ok(c) => c,
            Err(e) => {
                error!("Cannot read session-store.db: {}", e);
                return;
            }
        };

        // Sync sessions
        conn.execute("DELETE FROM sessions", []).ok();
        if let Ok(mut stmt) = source.prepare(
            "SELECT s.id, s.cwd, s.repository, s.branch, s.summary, s.created_at, s.updated_at,
                    (SELECT COUNT(*) FROM turns t WHERE t.session_id = s.id) as turn_count
             FROM sessions s",
        ) {
            if let Ok(rows) = stmt.query_map([], |r| {
                conn.execute(
                    "INSERT OR REPLACE INTO sessions (id, cwd, repository, branch, summary, created_at, updated_at, turn_count)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                    rusqlite::params![
                        r.get::<_, String>(0)?,
                        r.get::<_, Option<String>>(1)?,
                        r.get::<_, Option<String>>(2)?,
                        r.get::<_, Option<String>>(3)?,
                        r.get::<_, Option<String>>(4)?,
                        r.get::<_, Option<String>>(5)?,
                        r.get::<_, Option<String>>(6)?,
                        r.get::<_, i64>(7)?,
                    ],
                ).ok();
                Ok(())
            }) { rows.for_each(drop) }
        }

        // Sync file counts
        let total_files: i64 = source
            .query_row("SELECT COUNT(*) FROM session_files", [], |r| r.get(0))
            .unwrap_or(0);
        let unique_files: i64 = source
            .query_row(
                "SELECT COUNT(DISTINCT file_path) FROM session_files",
                [],
                |r| r.get(0),
            )
            .unwrap_or(0);
        conn.execute("DELETE FROM session_files_count", []).ok();
        conn.execute(
            "INSERT INTO session_files_count (total, unique_files) VALUES (?1, ?2)",
            rusqlite::params![total_files, unique_files],
        )
        .ok();

        // Sync repositories
        conn.execute("DELETE FROM repositories", []).ok();
        if let Ok(mut stmt) = source.prepare(
            "SELECT repository, COUNT(*) FROM sessions WHERE repository IS NOT NULL AND repository != '' GROUP BY repository"
        ) {
            if let Ok(rows) = stmt.query_map([], |r| {
                conn.execute(
                    "INSERT INTO repositories (name, session_count) VALUES (?1, ?2)",
                    rusqlite::params![r.get::<_, String>(0)?, r.get::<_, i64>(1)?],
                ).ok();
                Ok(())
            }) { rows.for_each(drop) }
        }

        // Sync monthly stats
        conn.execute("DELETE FROM monthly_stats", []).ok();
        if let Ok(mut stmt) = source.prepare(
            "SELECT substr(s.created_at, 1, 7) as month, COUNT(DISTINCT s.id), COUNT(t.id)
             FROM sessions s LEFT JOIN turns t ON t.session_id = s.id
             WHERE s.created_at IS NOT NULL GROUP BY month ORDER BY month",
        ) {
            if let Ok(rows) = stmt.query_map([], |r| {
                conn.execute(
                    "INSERT INTO monthly_stats (month, sessions, turns) VALUES (?1, ?2, ?3)",
                    rusqlite::params![
                        r.get::<_, String>(0)?,
                        r.get::<_, i64>(1)?,
                        r.get::<_, i64>(2)?
                    ],
                )
                .ok();
                Ok(())
            }) {
                rows.for_each(drop)
            }
        }

        info!("Stats cache: synced session-store.db data");
    }

    /// Incrementally ingest new events from events.jsonl files.
    /// Only reads bytes AFTER the last known offset for each file.
    pub fn refresh(&self) {
        let session_state = self.copilot_dir.join("session-state");
        if !session_state.exists() {
            return;
        }

        let conn = match Connection::open(&self.db_path) {
            Ok(c) => c,
            Err(e) => {
                error!("Stats cache open failed: {}", e);
                return;
            }
        };

        self.sync_session_store(&conn);

        let entries: Vec<PathBuf> = match std::fs::read_dir(&session_state) {
            Ok(e) => e
                .flatten()
                .map(|e| e.path().join("events.jsonl"))
                .filter(|p| p.exists())
                .collect(),
            Err(_) => return,
        };

        info!("Stats cache: scanning {} event files", entries.len());

        let mut new_events = 0u64;

        for events_file in &entries {
            let file_key = events_file.to_string_lossy().to_string();

            let file_size = match std::fs::metadata(events_file) {
                Ok(m) => m.len() as i64,
                Err(_) => continue,
            };

            let last_offset: i64 = conn
                .query_row(
                    "SELECT byte_offset FROM file_offsets WHERE file_path = ?",
                    [&file_key],
                    |r| r.get(0),
                )
                .unwrap_or(0);

            if last_offset >= file_size {
                continue;
            }

            let content = match std::fs::read_to_string(events_file) {
                Ok(c) => c,
                Err(_) => continue,
            };

            let bytes = content.as_bytes();
            if (last_offset as usize) >= bytes.len() {
                continue;
            }

            let new_content = &content[(last_offset as usize)..];

            conn.execute("BEGIN", []).ok();

            let mut last_model: Option<String> = None;

            for line in new_content.lines() {
                if line.trim().is_empty() {
                    continue;
                }

                if let Ok(obj) = serde_json::from_str::<serde_json::Value>(line) {
                    let event_type = obj
                        .get("type")
                        .and_then(|t| t.as_str())
                        .unwrap_or("unknown");

                    conn.execute(
                        "INSERT INTO event_counts (event_type, count) VALUES (?1, 1) \
                         ON CONFLICT(event_type) DO UPDATE SET count = count + 1",
                        [event_type],
                    )
                    .ok();

                    if event_type == "tool.execution_complete" {
                        if let Some(model) = obj.pointer("/data/model").and_then(|m| m.as_str()) {
                            conn.execute(
                                "INSERT INTO model_usage (model, count) VALUES (?1, 1) \
                                 ON CONFLICT(model) DO UPDATE SET count = count + 1",
                                [model],
                            )
                            .ok();

                            if last_model.as_deref() != Some(model) {
                                let ts = obj
                                    .get("timestamp")
                                    .or_else(|| obj.pointer("/data/timestamp"))
                                    .and_then(|t| t.as_str())
                                    .unwrap_or("");
                                if !ts.is_empty() {
                                    conn.execute(
                                        "INSERT INTO model_changes (model, timestamp) VALUES (?1, ?2)",
                                        rusqlite::params![model, ts],
                                    ).ok();
                                }
                                last_model = Some(model.to_string());
                            }
                        }
                    }
                    new_events += 1;
                }
            }

            conn.execute(
                "INSERT INTO file_offsets (file_path, byte_offset) VALUES (?1, ?2) \
                 ON CONFLICT(file_path) DO UPDATE SET byte_offset = ?2",
                rusqlite::params![file_key, file_size],
            )
            .ok();

            if let Err(e) = conn.execute("COMMIT", []) {
                tracing::error!("Stats cache COMMIT failed: {}", e);
            }
        }

        let now_secs = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
            .to_string();
        conn.execute(
            "INSERT INTO cache_meta (key, value) VALUES ('last_refresh', ?1) \
             ON CONFLICT(key) DO UPDATE SET value = ?1",
            [&now_secs],
        )
        .ok();

        info!("Stats cache: ingested {} new events", new_events);
    }

    /// Read aggregated stats from the cache DB (fast — no events.jsonl scanning).
    pub fn get_stats(&self) -> CachedStats {
        let conn = match Connection::open_with_flags(
            &self.db_path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
        ) {
            Ok(c) => c,
            Err(_) => return CachedStats::default(),
        };

        let model_usage: Vec<(String, u64)> = conn
            .prepare("SELECT model, count FROM model_usage ORDER BY count DESC")
            .ok()
            .map(|mut stmt| {
                stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, u64>(1)?)))
                    .ok()
                    .map(|rows| rows.flatten().collect())
                    .unwrap_or_default()
            })
            .unwrap_or_default();

        let total_tool_executions: u64 = conn
            .query_row(
                "SELECT COALESCE(SUM(count), 0) FROM event_counts \
                 WHERE event_type = 'tool.execution_complete'",
                [],
                |r| r.get(0),
            )
            .unwrap_or(0);

        let total_events: u64 = conn
            .query_row(
                "SELECT COALESCE(SUM(count), 0) FROM event_counts",
                [],
                |r| r.get(0),
            )
            .unwrap_or(0);

        let recent_changes: Vec<(String, String)> = conn
            .prepare(
                "SELECT model, timestamp FROM model_changes \
                 ORDER BY timestamp DESC LIMIT 20",
            )
            .ok()
            .map(|mut stmt| {
                stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
                    .ok()
                    .map(|rows| rows.flatten().collect())
                    .unwrap_or_default()
            })
            .unwrap_or_default();

        let last_refresh: Option<String> = conn
            .query_row(
                "SELECT value FROM cache_meta WHERE key = 'last_refresh'",
                [],
                |r| r.get(0),
            )
            .ok();

        let total_sessions: i64 = conn
            .query_row("SELECT COUNT(*) FROM sessions", [], |r| r.get(0))
            .unwrap_or(0);
        let total_turns: i64 = conn
            .query_row(
                "SELECT COALESCE(SUM(turn_count), 0) FROM sessions",
                [],
                |r| r.get(0),
            )
            .unwrap_or(0);
        let (total_files_edited, unique_files): (i64, i64) = conn
            .query_row(
                "SELECT COALESCE(total, 0), COALESCE(unique_files, 0) FROM session_files_count LIMIT 1",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap_or((0, 0));
        let sessions_by_month: Vec<(String, i64, i64)> = conn
            .prepare("SELECT month, sessions, turns FROM monthly_stats ORDER BY month")
            .ok()
            .map(|mut stmt| {
                stmt.query_map([], |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, i64>(1)?,
                        r.get::<_, i64>(2)?,
                    ))
                })
                .ok()
                .map(|rows| rows.flatten().collect())
                .unwrap_or_default()
            })
            .unwrap_or_default();
        let repositories: Vec<(String, i64)> = conn
            .prepare("SELECT name, session_count FROM repositories ORDER BY session_count DESC")
            .ok()
            .map(|mut stmt| {
                stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))
                    .ok()
                    .map(|rows| rows.flatten().collect())
                    .unwrap_or_default()
            })
            .unwrap_or_default();

        CachedStats {
            model_usage,
            total_tool_executions,
            total_events,
            recent_model_changes: recent_changes,
            last_refresh,
            total_sessions,
            total_turns,
            total_files_edited,
            unique_files,
            sessions_by_month,
            repositories,
        }
    }
}

#[derive(Debug, Default)]
#[allow(dead_code)]
pub struct CachedStats {
    pub model_usage: Vec<(String, u64)>,
    pub total_tool_executions: u64,
    pub total_events: u64,
    pub recent_model_changes: Vec<(String, String)>,
    pub last_refresh: Option<String>,
    pub total_sessions: i64,
    pub total_turns: i64,
    pub total_files_edited: i64,
    pub unique_files: i64,
    pub sessions_by_month: Vec<(String, i64, i64)>, // (month, sessions, turns)
    pub repositories: Vec<(String, i64)>,           // (name, count)
}
