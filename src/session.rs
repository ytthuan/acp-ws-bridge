//! Session management for ACP/Copilot CLI sessions.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use serde::{Deserialize, Serialize};
use tokio::sync::{watch, Mutex};
use tokio::time::Instant;

/// Status of a bridge session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionStatus {
    Connecting,
    Active,
    Idle,
    Disconnected,
    Error,
}

/// Serializable session info exposed to clients.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionInfo {
    pub id: String,
    pub copilot_session_id: Option<String>,
    pub status: SessionStatus,
    pub created_at: String,
    pub last_activity: String,
    pub prompt_count: u64,
    pub message_count: u64,
}

/// Tracked state for a single WebSocket connection.
struct SessionEntry {
    pub peer_addr: SocketAddr,
    pub info: SessionInfo,
    pub last_activity_instant: Instant,
    /// Send `true` to signal the relay task to shut down.
    shutdown_tx: watch::Sender<bool>,
}

/// Aggregate statistics across all sessions.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionStats {
    pub total_sessions: usize,
    pub active_sessions: usize,
    pub idle_sessions: usize,
    pub total_prompts: u64,
    pub total_messages: u64,
}

/// Shared session tracker used by the bridge and the idle checker.
#[derive(Clone)]
pub struct SessionManager {
    sessions: Arc<Mutex<HashMap<String, SessionEntry>>>,
    counter: Arc<AtomicU64>,
}

/// Handle returned when a session is registered. Holds the shutdown receiver
/// and provides a method to record activity.
pub struct SessionHandle {
    pub id: String,
    pub shutdown_rx: watch::Receiver<bool>,
    manager: SessionManager,
}

impl Default for SessionManager {
    fn default() -> Self {
        Self::new()
    }
}

impl SessionManager {
    pub fn new() -> Self {
        Self {
            sessions: Arc::new(Mutex::new(HashMap::new())),
            counter: Arc::new(AtomicU64::new(0)),
        }
    }

    /// Register a new session and return a handle for the relay task.
    pub async fn register(&self, peer_addr: SocketAddr) -> SessionHandle {
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let seq = self.counter.fetch_add(1, Ordering::Relaxed) + 1;
        let id = format!("remo_sess_{:03}", seq);
        let now = Utc::now().to_rfc3339();

        let info = SessionInfo {
            id: id.clone(),
            copilot_session_id: None,
            status: SessionStatus::Connecting,
            created_at: now.clone(),
            last_activity: now,
            prompt_count: 0,
            message_count: 0,
        };

        let entry = SessionEntry {
            peer_addr,
            info,
            last_activity_instant: Instant::now(),
            shutdown_tx,
        };
        self.sessions.lock().await.insert(id.clone(), entry);
        tracing::info!(session_id = %id, %peer_addr, "Session registered");

        SessionHandle {
            id,
            shutdown_rx,
            manager: self.clone(),
        }
    }

    /// Create a new session (without shutdown signaling), returns the session info.
    pub async fn create_session(&self) -> SessionInfo {
        let (shutdown_tx, _shutdown_rx) = watch::channel(false);
        let seq = self.counter.fetch_add(1, Ordering::Relaxed) + 1;
        let id = format!("remo_sess_{:03}", seq);
        let now = Utc::now().to_rfc3339();

        let info = SessionInfo {
            id: id.clone(),
            copilot_session_id: None,
            status: SessionStatus::Connecting,
            created_at: now.clone(),
            last_activity: now,
            prompt_count: 0,
            message_count: 0,
        };

        let entry = SessionEntry {
            peer_addr: "0.0.0.0:0".parse().unwrap(),
            info: info.clone(),
            last_activity_instant: Instant::now(),
            shutdown_tx,
        };
        self.sessions.lock().await.insert(id, entry);
        info
    }

    /// Remove a session (called when the relay ends).
    pub async fn unregister(&self, id: &str) {
        if let Some(entry) = self.sessions.lock().await.remove(id) {
            tracing::info!(session_id = %id, peer_addr = %entry.peer_addr, "Session unregistered");
        }
    }

    /// List all sessions.
    pub async fn list_sessions(&self) -> Vec<SessionInfo> {
        self.sessions
            .lock()
            .await
            .values()
            .map(|s| s.info.clone())
            .collect()
    }

    /// Get a specific session.
    pub async fn get_session(&self, id: &str) -> Option<SessionInfo> {
        self.sessions.lock().await.get(id).map(|s| s.info.clone())
    }

    /// Update session status.
    pub async fn update_status(&self, id: &str, status: SessionStatus) {
        if let Some(entry) = self.sessions.lock().await.get_mut(id) {
            entry.info.status = status;
            entry.info.last_activity = Utc::now().to_rfc3339();
            entry.last_activity_instant = Instant::now();
        }
    }

    /// Record activity for a session (called on every WS/TCP message).
    pub async fn touch(&self, id: &str) {
        if let Some(entry) = self.sessions.lock().await.get_mut(id) {
            entry.last_activity_instant = Instant::now();
            entry.info.last_activity = Utc::now().to_rfc3339();
        }
    }

    /// Record activity (alias for touch).
    pub async fn record_activity(&self, id: &str) {
        self.touch(id).await;
    }

    /// Increment prompt count.
    pub async fn increment_prompts(&self, id: &str) {
        if let Some(entry) = self.sessions.lock().await.get_mut(id) {
            entry.info.prompt_count += 1;
        }
    }

    /// Increment message count.
    pub async fn increment_messages(&self, id: &str) {
        if let Some(entry) = self.sessions.lock().await.get_mut(id) {
            entry.info.message_count += 1;
        }
    }

    /// Set the copilot session ID for a bridge session.
    pub async fn set_copilot_session_id(&self, id: &str, copilot_id: String) {
        if let Some(entry) = self.sessions.lock().await.get_mut(id) {
            entry.info.copilot_session_id = Some(copilot_id);
        }
    }

    /// Delete a session.
    pub async fn delete_session(&self, id: &str) -> bool {
        self.sessions.lock().await.remove(id).is_some()
    }

    /// Get aggregate statistics.
    pub async fn get_stats(&self) -> SessionStats {
        let sessions = self.sessions.lock().await;
        let mut stats = SessionStats {
            total_sessions: sessions.len(),
            active_sessions: 0,
            idle_sessions: 0,
            total_prompts: 0,
            total_messages: 0,
        };
        for entry in sessions.values() {
            match entry.info.status {
                SessionStatus::Active => stats.active_sessions += 1,
                SessionStatus::Idle => stats.idle_sessions += 1,
                _ => {}
            }
            stats.total_prompts += entry.info.prompt_count;
            stats.total_messages += entry.info.message_count;
        }
        stats
    }

    /// Check if a session has been marked as disconnected.
    pub async fn is_disconnected(&self, id: &str) -> bool {
        self.sessions
            .lock()
            .await
            .get(id)
            .map(|e| e.info.status == SessionStatus::Disconnected)
            .unwrap_or(true)
    }

    /// Scan sessions and disconnect any that have been idle longer than `timeout`.
    pub async fn disconnect_idle(&self, timeout: Duration) {
        let now = Instant::now();
        let mut sessions = self.sessions.lock().await;
        for (id, entry) in sessions.iter_mut() {
            if entry.info.status == SessionStatus::Active
                && now.duration_since(entry.last_activity_instant) > timeout
            {
                tracing::warn!(
                    session_id = %id,
                    peer_addr = %entry.peer_addr,
                    idle_secs = now.duration_since(entry.last_activity_instant).as_secs(),
                    "Session idle timeout — disconnecting"
                );
                entry.info.status = SessionStatus::Disconnected;
                let _ = entry.shutdown_tx.send(true);
            }
        }
    }
}

impl SessionHandle {
    /// Record activity on this session.
    pub async fn touch(&self) {
        self.manager.touch(&self.id).await;
    }
}

/// Spawn a background task that periodically checks for idle sessions.
pub fn spawn_idle_checker(
    session_manager: SessionManager,
    idle_timeout: Duration,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(60));
        loop {
            interval.tick().await;
            session_manager.disconnect_idle(idle_timeout).await;
        }
    })
}
