//! Read-only access to Copilot CLI session history from ~/.copilot/session-store.db.

use std::path::PathBuf;

use rusqlite::Connection;
use serde::Serialize;

/// Get the path to ~/.copilot/session-store.db
fn session_store_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_default()
        .join(".copilot")
        .join("session-store.db")
}

#[derive(Debug, Serialize)]
pub struct HistorySession {
    pub id: String,
    pub cwd: Option<String>,
    pub repository: Option<String>,
    pub branch: Option<String>,
    pub summary: Option<String>,
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
pub struct HistoryStats {
    pub total_sessions: i64,
    pub total_turns: i64,
    pub repositories: Vec<RepoCount>,
    pub earliest_session: Option<String>,
    pub latest_session: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct RepoCount {
    pub repository: String,
    pub count: i64,
}

/// List all sessions with turn counts, ordered by most recent first.
pub fn list_sessions() -> anyhow::Result<Vec<HistorySession>> {
    let conn = Connection::open(session_store_path())?;
    let mut stmt = conn.prepare(
        "SELECT s.id, s.cwd, s.repository, s.branch, s.summary, s.created_at, s.updated_at,
                COALESCE((SELECT COUNT(*) FROM turns t WHERE t.session_id = s.id), 0) as turn_count
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
    let conn = Connection::open(session_store_path())?;
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
    let conn = Connection::open(session_store_path())?;

    let total_sessions: i64 =
        conn.query_row("SELECT COUNT(*) FROM sessions", [], |r| r.get(0))?;
    let total_turns: i64 = conn.query_row("SELECT COUNT(*) FROM turns", [], |r| r.get(0))?;
    let earliest: Option<String> = conn
        .query_row("SELECT MIN(created_at) FROM sessions", [], |r| r.get(0))
        .ok();
    let latest: Option<String> = conn
        .query_row("SELECT MAX(created_at) FROM sessions", [], |r| r.get(0))
        .ok();

    let mut stmt = conn.prepare(
        "SELECT repository, COUNT(*) as cnt FROM sessions WHERE repository IS NOT NULL
         GROUP BY repository ORDER BY cnt DESC LIMIT 10",
    )?;
    let repos = stmt
        .query_map([], |row| {
            Ok(RepoCount {
                repository: row.get(0)?,
                count: row.get(1)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;

    Ok(HistoryStats {
        total_sessions,
        total_turns,
        repositories: repos,
        earliest_session: earliest,
        latest_session: latest,
    })
}
