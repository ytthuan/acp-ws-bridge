//! Read-only access to Copilot CLI session history from ~/.copilot/session-store.db.

use std::collections::HashMap;
use std::path::PathBuf;

use rusqlite::Connection;
use serde::Serialize;

/// Get the path to ~/.copilot/session-store.db
fn session_store_path() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join(".copilot").join("session-store.db"))
}

#[derive(Debug, Serialize)]
pub struct HistorySession {
    pub id: String,
    pub cwd: Option<String>,
    pub repository: Option<String>,
    pub branch: Option<String>,
    pub summary: Option<String>,
    pub preview: Option<String>,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
    pub turn_count: i64,
}

#[derive(Debug, Serialize)]
pub struct HistoryTurn {
    pub turn_index: i64,
    pub user_message: Option<String>,
    pub assistant_response: Option<String>,
    pub timestamp: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct DateCount {
    pub date: String,
    pub count: i64,
}

#[derive(Debug, Serialize)]
pub struct MonthCount {
    pub month: String,
    pub count: i64,
}

#[derive(Debug, Serialize)]
pub struct TopRepo {
    pub name: String,
    pub session_count: i64,
    pub turn_count: i64,
}

#[derive(Debug, Serialize)]
pub struct NameCount {
    pub name: String,
    pub count: i64,
}

#[derive(Debug, Serialize)]
pub struct HourCount {
    pub hour: i64,
    pub count: i64,
}

#[derive(Debug, Serialize)]
pub struct RecentSession {
    pub id: String,
    pub summary: Option<String>,
    pub created_at: Option<String>,
    pub repository: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct HistoryStats {
    pub total_sessions: i64,
    pub total_turns: i64,
    pub total_repositories: i64,
    pub total_files_edited: i64,
    pub sessions_today: i64,
    pub sessions_this_week: i64,
    pub sessions_this_month: i64,
    pub turns_today: i64,
    pub turns_this_week: i64,
    pub turns_this_month: i64,
    pub sessions_by_day: Vec<DateCount>,
    pub sessions_by_month: Vec<MonthCount>,
    pub turns_by_day: Vec<DateCount>,
    pub top_repositories: Vec<TopRepo>,
    pub top_branches: Vec<NameCount>,
    pub recent_sessions: Vec<RecentSession>,
    pub average_turns_per_session: f64,
    pub average_session_duration: String,
    pub tools_used: Vec<NameCount>,
    pub active_hours: Vec<HourCount>,
    pub earliest_session: Option<String>,
    pub latest_session: Option<String>,
}

/// List all sessions with turn counts, ordered by most recent first.
pub fn list_sessions() -> anyhow::Result<Vec<HistorySession>> {
    let db_path = match session_store_path() {
        Some(p) => p,
        None => return Ok(vec![]),
    };
    let conn = Connection::open(db_path)?;
    let mut stmt = conn.prepare(
        "SELECT s.id, s.cwd, s.repository, s.branch, s.summary, s.created_at, s.updated_at,
                COALESCE((SELECT COUNT(*) FROM turns t WHERE t.session_id = s.id), 0) as turn_count,
                (SELECT substr(t2.user_message, 1, 100) FROM turns t2 WHERE t2.session_id = s.id AND t2.user_message IS NOT NULL AND t2.user_message != '' ORDER BY t2.turn_index LIMIT 1) as preview
         FROM sessions s
         ORDER BY s.created_at DESC",
    )?;
    let sessions = stmt
        .query_map([], |row| {
            Ok(HistorySession {
                id: row.get(0)?,
                cwd: row.get(1)?,
                repository: row.get(2)?,
                branch: row.get(3)?,
                summary: row.get(4)?,
                preview: row.get(8)?,
                created_at: row.get(5)?,
                updated_at: row.get(6)?,
                turn_count: row.get(7)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(sessions)
}

/// Get turns for a specific session, ordered by turn index.
pub fn get_session_turns(session_id: &str) -> anyhow::Result<Vec<HistoryTurn>> {
    let db_path = match session_store_path() {
        Some(p) => p,
        None => return Ok(vec![]),
    };
    let conn = Connection::open(db_path)?;
    let mut stmt = conn.prepare(
        "SELECT turn_index, user_message, assistant_response, timestamp
         FROM turns WHERE session_id = ? ORDER BY turn_index",
    )?;
    let turns = stmt
        .query_map([session_id], |row| {
            Ok(HistoryTurn {
                turn_index: row.get(0)?,
                user_message: row.get(1)?,
                assistant_response: row.get(2)?,
                timestamp: row.get(3)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(turns)
}

/// Get aggregate statistics across all sessions.
pub fn get_history_stats() -> anyhow::Result<HistoryStats> {
    let db_path = match session_store_path() {
        Some(p) => p,
        None => anyhow::bail!("Could not determine home directory"),
    };
    let conn = Connection::open(db_path)?;

    let total_sessions: i64 =
        conn.query_row("SELECT COUNT(*) FROM sessions", [], |r| r.get(0))?;
    let total_turns: i64 =
        conn.query_row("SELECT COUNT(*) FROM turns", [], |r| r.get(0))?;
    let total_repositories: i64 = conn.query_row(
        "SELECT COUNT(DISTINCT repository) FROM sessions \
         WHERE repository IS NOT NULL AND repository != ''",
        [],
        |r| r.get(0),
    )?;
    let total_files_edited: i64 = conn
        .query_row(
            "SELECT COUNT(DISTINCT file_path) FROM session_files",
            [],
            |r| r.get(0),
        )
        .unwrap_or(0);

    // Time-period counts
    let sessions_today: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sessions WHERE date(created_at) = date('now')",
            [],
            |r| r.get(0),
        )
        .unwrap_or(0);
    let sessions_this_week: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sessions WHERE created_at >= date('now', '-7 days')",
            [],
            |r| r.get(0),
        )
        .unwrap_or(0);
    let sessions_this_month: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sessions WHERE created_at >= date('now', '-30 days')",
            [],
            |r| r.get(0),
        )
        .unwrap_or(0);
    let turns_today: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM turns WHERE date(timestamp) = date('now')",
            [],
            |r| r.get(0),
        )
        .unwrap_or(0);
    let turns_this_week: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM turns WHERE timestamp >= date('now', '-7 days')",
            [],
            |r| r.get(0),
        )
        .unwrap_or(0);
    let turns_this_month: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM turns WHERE timestamp >= date('now', '-30 days')",
            [],
            |r| r.get(0),
        )
        .unwrap_or(0);

    let sessions_by_day = query_vec(
        &conn,
        "SELECT date(created_at) as d, COUNT(*) FROM sessions \
         WHERE created_at >= date('now', '-30 days') AND created_at IS NOT NULL \
         GROUP BY d ORDER BY d",
        |row| Ok(DateCount { date: row.get(0)?, count: row.get(1)? }),
    );
    let sessions_by_month = query_vec(
        &conn,
        "SELECT strftime('%Y-%m', created_at) as m, COUNT(*) FROM sessions \
         WHERE created_at IS NOT NULL GROUP BY m ORDER BY m",
        |row| Ok(MonthCount { month: row.get(0)?, count: row.get(1)? }),
    );
    let turns_by_day = query_vec(
        &conn,
        "SELECT date(timestamp) as d, COUNT(*) FROM turns \
         WHERE timestamp >= date('now', '-30 days') AND timestamp IS NOT NULL \
         GROUP BY d ORDER BY d",
        |row| Ok(DateCount { date: row.get(0)?, count: row.get(1)? }),
    );
    let top_repositories = query_vec(
        &conn,
        "SELECT s.repository, COUNT(DISTINCT s.id), \
                COALESCE((SELECT COUNT(*) FROM turns t WHERE t.session_id IN \
                    (SELECT id FROM sessions WHERE repository = s.repository)), 0) \
         FROM sessions s WHERE s.repository IS NOT NULL AND s.repository != '' \
         GROUP BY s.repository ORDER BY COUNT(DISTINCT s.id) DESC LIMIT 10",
        |row| {
            Ok(TopRepo {
                name: row.get(0)?,
                session_count: row.get(1)?,
                turn_count: row.get(2)?,
            })
        },
    );
    let top_branches = query_vec(
        &conn,
        "SELECT branch, COUNT(*) as cnt FROM sessions \
         WHERE branch IS NOT NULL AND branch != '' \
         GROUP BY branch ORDER BY cnt DESC LIMIT 10",
        |row| Ok(NameCount { name: row.get(0)?, count: row.get(1)? }),
    );
    let recent_sessions = query_vec(
        &conn,
        "SELECT id, summary, created_at, repository FROM sessions \
         ORDER BY created_at DESC LIMIT 10",
        |row| {
            Ok(RecentSession {
                id: row.get(0)?,
                summary: row.get(1)?,
                created_at: row.get(2)?,
                repository: row.get(3)?,
            })
        },
    );

    let average_turns_per_session = if total_sessions > 0 {
        (total_turns as f64) / (total_sessions as f64)
    } else {
        0.0
    };
    let avg_minutes: f64 = conn
        .query_row(
            "SELECT COALESCE(AVG((julianday(updated_at) - julianday(created_at)) * 24.0 * 60.0), 0) \
             FROM sessions WHERE updated_at IS NOT NULL AND created_at IS NOT NULL \
             AND updated_at != created_at",
            [],
            |r| r.get(0),
        )
        .unwrap_or(0.0);
    let average_session_duration = format_duration_minutes(avg_minutes);

    let tools_used = query_vec(
        &conn,
        "SELECT tool_name, COUNT(*) as cnt FROM session_files \
         WHERE tool_name IS NOT NULL AND tool_name != '' \
         GROUP BY tool_name ORDER BY cnt DESC",
        |row| Ok(NameCount { name: row.get(0)?, count: row.get(1)? }),
    );
    let active_hours = query_vec(
        &conn,
        "SELECT CAST(strftime('%H', created_at) AS INTEGER) as h, COUNT(*) \
         FROM sessions WHERE created_at IS NOT NULL GROUP BY h ORDER BY h",
        |row| Ok(HourCount { hour: row.get(0)?, count: row.get(1)? }),
    );

    let earliest: Option<String> = conn
        .query_row("SELECT MIN(created_at) FROM sessions", [], |r| r.get(0))
        .ok();
    let latest: Option<String> = conn
        .query_row("SELECT MAX(created_at) FROM sessions", [], |r| r.get(0))
        .ok();

    Ok(HistoryStats {
        total_sessions,
        total_turns,
        total_repositories,
        total_files_edited,
        sessions_today,
        sessions_this_week,
        sessions_this_month,
        turns_today,
        turns_this_week,
        turns_this_month,
        sessions_by_day,
        sessions_by_month,
        turns_by_day,
        top_repositories,
        top_branches,
        recent_sessions,
        average_turns_per_session,
        average_session_duration,
        tools_used,
        active_hours,
        earliest_session: earliest,
        latest_session: latest,
    })
}

/// Run a query and collect results into a Vec, returning empty on error.
fn query_vec<T>(
    conn: &Connection,
    sql: &str,
    mapper: impl FnMut(&rusqlite::Row<'_>) -> rusqlite::Result<T>,
) -> Vec<T> {
    conn.prepare(sql)
        .and_then(|mut stmt| {
            stmt.query_map([], mapper)?
                .collect::<Result<Vec<_>, _>>()
        })
        .unwrap_or_default()
}

// ---- Copilot usage stats (events.jsonl + session-store.db) ----

#[derive(Debug, Serialize)]
pub struct ModelUsage {
    pub model: String,
    pub count: u64,
}

#[derive(Debug, Serialize)]
pub struct MonthUsage {
    pub month: String,
    pub sessions: i64,
    pub turns: i64,
}

#[derive(Debug, Serialize)]
pub struct ModelChange {
    pub model: String,
    pub timestamp: String,
}

#[derive(Debug, Serialize)]
pub struct CopilotUsageStats {
    #[serde(rename = "totalSessions")]
    pub total_sessions: i64,
    #[serde(rename = "totalTurns")]
    pub total_turns: i64,
    #[serde(rename = "totalFilesEdited")]
    pub total_files_edited: i64,
    #[serde(rename = "modelUsage")]
    pub model_usage: Vec<ModelUsage>,
    #[serde(rename = "sessionsByMonth")]
    pub sessions_by_month: Vec<MonthUsage>,
    #[serde(rename = "recentModelChanges")]
    pub recent_model_changes: Vec<ModelChange>,
    #[serde(rename = "totalToolExecutions")]
    pub total_tool_executions: u64,
    #[serde(rename = "eventTypeCounts")]
    pub event_type_counts: HashMap<String, u64>,
}

/// Aggregate usage statistics from the pre-built stats cache only.
/// session-store.db data is merged into the cache during refresh().
pub fn get_copilot_usage(cache: &crate::stats_cache::StatsCache) -> anyhow::Result<CopilotUsageStats> {
    let cached = cache.get_stats();

    Ok(CopilotUsageStats {
        total_sessions: cached.total_sessions,
        total_turns: cached.total_turns,
        total_files_edited: cached.total_files_edited,
        model_usage: cached.model_usage.into_iter().map(|(m, c)| ModelUsage { model: m, count: c }).collect(),
        sessions_by_month: cached.sessions_by_month.into_iter().map(|(m, s, t)| MonthUsage { month: m, sessions: s, turns: t }).collect(),
        recent_model_changes: cached.recent_model_changes.into_iter().map(|(m, t)| ModelChange { model: m, timestamp: t }).collect(),
        total_tool_executions: cached.total_tool_executions,
        event_type_counts: HashMap::new(),
    })
}

/// Format minutes as a human-readable duration string.
fn format_duration_minutes(minutes: f64) -> String {
    if minutes < 1.0 {
        return "< 1m".to_string();
    }
    let total = minutes as u64;
    let h = total / 60;
    let m = total % 60;
    if h > 0 {
        format!("{}h {}m", h, m)
    } else {
        format!("{}m", m)
    }
}
