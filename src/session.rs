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
    /// Cached available_commands from ACP session/update notifications.
    pub available_commands: Option<serde_json::Value>,
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
            available_commands: None,
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
    /// Used in tests as a simpler alternative to `register()`.
    #[cfg(test)]
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
            available_commands: None,
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

    /// Store available_commands observed from ACP session/update notifications.
    pub async fn set_available_commands(&self, id: &str, commands: serde_json::Value) {
        if let Some(entry) = self.sessions.lock().await.get_mut(id) {
            entry.available_commands = Some(commands);
        }
    }

    /// Get cached available_commands for a session.
    pub async fn get_available_commands(&self, id: &str) -> Option<serde_json::Value> {
        self.sessions
            .lock()
            .await
            .get(id)
            .and_then(|e| e.available_commands.clone())
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
    #[cfg(test)]
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

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_create_session() {
        let sm = SessionManager::new();
        let info = sm.create_session().await;
        assert_eq!(info.id, "remo_sess_001");
        assert_eq!(info.status, SessionStatus::Connecting);
        assert_eq!(info.prompt_count, 0);
        assert_eq!(info.message_count, 0);
        assert!(info.copilot_session_id.is_none());
    }

    #[tokio::test]
    async fn test_list_sessions() {
        let sm = SessionManager::new();
        assert!(sm.list_sessions().await.is_empty());

        sm.create_session().await;
        sm.create_session().await;
        let sessions = sm.list_sessions().await;
        assert_eq!(sessions.len(), 2);
    }

    #[tokio::test]
    async fn test_get_session() {
        let sm = SessionManager::new();
        let info = sm.create_session().await;
        let fetched = sm.get_session(&info.id).await;
        assert!(fetched.is_some());
        assert_eq!(fetched.unwrap().id, info.id);

        assert!(sm.get_session("nonexistent").await.is_none());
    }

    #[tokio::test]
    async fn test_delete_session() {
        let sm = SessionManager::new();
        let info = sm.create_session().await;
        assert!(sm.delete_session(&info.id).await);
        assert!(sm.list_sessions().await.is_empty());
    }

    #[tokio::test]
    async fn test_delete_nonexistent_session() {
        let sm = SessionManager::new();
        assert!(!sm.delete_session("nonexistent").await);
    }

    #[tokio::test]
    async fn test_session_status_updates() {
        let sm = SessionManager::new();
        let info = sm.create_session().await;
        assert_eq!(info.status, SessionStatus::Connecting);

        sm.update_status(&info.id, SessionStatus::Active).await;
        let updated = sm.get_session(&info.id).await.unwrap();
        assert_eq!(updated.status, SessionStatus::Active);

        sm.update_status(&info.id, SessionStatus::Idle).await;
        let updated = sm.get_session(&info.id).await.unwrap();
        assert_eq!(updated.status, SessionStatus::Idle);

        sm.update_status(&info.id, SessionStatus::Disconnected)
            .await;
        let updated = sm.get_session(&info.id).await.unwrap();
        assert_eq!(updated.status, SessionStatus::Disconnected);

        sm.update_status(&info.id, SessionStatus::Error).await;
        let updated = sm.get_session(&info.id).await.unwrap();
        assert_eq!(updated.status, SessionStatus::Error);
    }

    #[tokio::test]
    async fn test_activity_tracking() {
        let sm = SessionManager::new();
        let info = sm.create_session().await;
        let before = sm
            .get_session(&info.id)
            .await
            .unwrap()
            .last_activity
            .clone();

        // Small delay to ensure timestamp changes
        tokio::time::sleep(Duration::from_millis(10)).await;
        sm.record_activity(&info.id).await;

        let after = sm.get_session(&info.id).await.unwrap().last_activity;
        assert!(after >= before);
    }

    #[tokio::test]
    async fn test_touch_alias() {
        let sm = SessionManager::new();
        let info = sm.create_session().await;
        // touch and record_activity should both work
        sm.touch(&info.id).await;
        let session = sm.get_session(&info.id).await.unwrap();
        assert!(!session.last_activity.is_empty());
    }

    #[tokio::test]
    async fn test_prompt_count_increment() {
        let sm = SessionManager::new();
        let info = sm.create_session().await;
        assert_eq!(info.prompt_count, 0);

        sm.increment_prompts(&info.id).await;
        sm.increment_prompts(&info.id).await;
        sm.increment_prompts(&info.id).await;
        let updated = sm.get_session(&info.id).await.unwrap();
        assert_eq!(updated.prompt_count, 3);
    }

    #[tokio::test]
    async fn test_message_count_increment() {
        let sm = SessionManager::new();
        let info = sm.create_session().await;
        assert_eq!(info.message_count, 0);

        sm.increment_messages(&info.id).await;
        sm.increment_messages(&info.id).await;
        let updated = sm.get_session(&info.id).await.unwrap();
        assert_eq!(updated.message_count, 2);
    }

    #[tokio::test]
    async fn test_set_copilot_session_id() {
        let sm = SessionManager::new();
        let info = sm.create_session().await;
        assert!(info.copilot_session_id.is_none());

        sm.set_copilot_session_id(&info.id, "copilot-123".to_string())
            .await;
        let updated = sm.get_session(&info.id).await.unwrap();
        assert_eq!(updated.copilot_session_id.as_deref(), Some("copilot-123"));
    }

    #[tokio::test]
    async fn test_available_commands() {
        let sm = SessionManager::new();
        let info = sm.create_session().await;

        // Initially none
        assert!(sm.get_available_commands(&info.id).await.is_none());

        // Set commands
        let cmds = serde_json::json!([{"name": "explain"}, {"name": "fix"}]);
        sm.set_available_commands(&info.id, cmds.clone()).await;
        let fetched = sm.get_available_commands(&info.id).await.unwrap();
        assert_eq!(fetched, cmds);

        // Nonexistent session returns None
        assert!(sm.get_available_commands("nonexistent").await.is_none());
    }

    #[tokio::test]
    async fn test_get_stats() {
        let sm = SessionManager::new();

        // Empty stats
        let stats = sm.get_stats().await;
        assert_eq!(stats.total_sessions, 0);
        assert_eq!(stats.active_sessions, 0);
        assert_eq!(stats.idle_sessions, 0);
        assert_eq!(stats.total_prompts, 0);
        assert_eq!(stats.total_messages, 0);

        // Create sessions and update statuses
        let s1 = sm.create_session().await;
        let s2 = sm.create_session().await;
        let s3 = sm.create_session().await;

        sm.update_status(&s1.id, SessionStatus::Active).await;
        sm.update_status(&s2.id, SessionStatus::Active).await;
        sm.update_status(&s3.id, SessionStatus::Idle).await;

        sm.increment_prompts(&s1.id).await;
        sm.increment_prompts(&s2.id).await;
        sm.increment_prompts(&s2.id).await;
        sm.increment_messages(&s1.id).await;
        sm.increment_messages(&s1.id).await;
        sm.increment_messages(&s1.id).await;

        let stats = sm.get_stats().await;
        assert_eq!(stats.total_sessions, 3);
        assert_eq!(stats.active_sessions, 2);
        assert_eq!(stats.idle_sessions, 1);
        assert_eq!(stats.total_prompts, 3);
        assert_eq!(stats.total_messages, 3);
    }

    #[tokio::test]
    async fn test_is_disconnected() {
        let sm = SessionManager::new();
        // Nonexistent session returns true (disconnected)
        assert!(sm.is_disconnected("nonexistent").await);

        let info = sm.create_session().await;
        assert!(!sm.is_disconnected(&info.id).await);

        sm.update_status(&info.id, SessionStatus::Disconnected)
            .await;
        assert!(sm.is_disconnected(&info.id).await);
    }

    #[tokio::test]
    async fn test_register_and_unregister() {
        let sm = SessionManager::new();
        let addr: std::net::SocketAddr = "127.0.0.1:12345".parse().unwrap();
        let handle = sm.register(addr).await;
        assert!(sm.get_session(&handle.id).await.is_some());

        sm.unregister(&handle.id).await;
        assert!(sm.get_session(&handle.id).await.is_none());
    }

    #[tokio::test]
    async fn test_session_handle_touch() {
        let sm = SessionManager::new();
        let addr: std::net::SocketAddr = "127.0.0.1:12345".parse().unwrap();
        let handle = sm.register(addr).await;
        // Should not panic
        handle.touch().await;
        let session = sm.get_session(&handle.id).await.unwrap();
        assert!(!session.last_activity.is_empty());
    }

    #[tokio::test]
    async fn test_sequential_session_ids() {
        let sm = SessionManager::new();
        let s1 = sm.create_session().await;
        let s2 = sm.create_session().await;
        let s3 = sm.create_session().await;
        assert_eq!(s1.id, "remo_sess_001");
        assert_eq!(s2.id, "remo_sess_002");
        assert_eq!(s3.id, "remo_sess_003");
    }

    #[tokio::test]
    async fn test_concurrent_session_access() {
        let sm = SessionManager::new();
        let info = sm.create_session().await;
        let sid = info.id.clone();

        // Spawn multiple tasks that concurrently modify the session
        let mut handles = vec![];
        for _ in 0..10 {
            let sm_clone = sm.clone();
            let sid_clone = sid.clone();
            handles.push(tokio::spawn(async move {
                sm_clone.increment_prompts(&sid_clone).await;
                sm_clone.increment_messages(&sid_clone).await;
                sm_clone.record_activity(&sid_clone).await;
            }));
        }

        for h in handles {
            h.await.unwrap();
        }

        let session = sm.get_session(&sid).await.unwrap();
        assert_eq!(session.prompt_count, 10);
        assert_eq!(session.message_count, 10);
    }

    #[tokio::test]
    async fn test_disconnect_idle() {
        let sm = SessionManager::new();
        let info = sm.create_session().await;
        sm.update_status(&info.id, SessionStatus::Active).await;

        // With a zero timeout, every active session should be disconnected
        sm.disconnect_idle(Duration::from_secs(0)).await;
        let updated = sm.get_session(&info.id).await.unwrap();
        assert_eq!(updated.status, SessionStatus::Disconnected);
    }

    #[tokio::test]
    async fn test_disconnect_idle_skips_non_active() {
        let sm = SessionManager::new();
        let info = sm.create_session().await;
        // Session is in Connecting state, should not be affected
        sm.disconnect_idle(Duration::from_secs(0)).await;
        let updated = sm.get_session(&info.id).await.unwrap();
        assert_eq!(updated.status, SessionStatus::Connecting);
    }

    #[test]
    fn test_session_status_serialization() {
        let json = serde_json::to_string(&SessionStatus::Active).unwrap();
        assert_eq!(json, "\"active\"");
        let json = serde_json::to_string(&SessionStatus::Disconnected).unwrap();
        assert_eq!(json, "\"disconnected\"");

        let status: SessionStatus = serde_json::from_str("\"idle\"").unwrap();
        assert_eq!(status, SessionStatus::Idle);
    }

    #[test]
    fn test_session_info_serialization() {
        let info = SessionInfo {
            id: "test".to_string(),
            copilot_session_id: Some("cop-1".to_string()),
            status: SessionStatus::Active,
            created_at: "2024-01-01T00:00:00Z".to_string(),
            last_activity: "2024-01-01T00:00:00Z".to_string(),
            prompt_count: 5,
            message_count: 10,
        };
        let json = serde_json::to_string(&info).unwrap();
        let deserialized: SessionInfo = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.id, "test");
        assert_eq!(deserialized.prompt_count, 5);
        assert_eq!(deserialized.message_count, 10);
    }

    #[test]
    fn test_session_stats_serialization() {
        let stats = SessionStats {
            total_sessions: 3,
            active_sessions: 2,
            idle_sessions: 1,
            total_prompts: 10,
            total_messages: 20,
        };
        let json = serde_json::to_string(&stats).unwrap();
        let deserialized: SessionStats = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.total_sessions, 3);
        assert_eq!(deserialized.total_prompts, 10);
    }

    #[test]
    fn test_session_manager_default() {
        let sm = SessionManager::default();
        // Should be equivalent to ::new()
        let _ = sm; // just verify it compiles and doesn't panic
    }
}
