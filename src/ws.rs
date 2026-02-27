//! WebSocket server handling for remote iOS client connections.

use std::sync::Arc;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::mpsc;
use tokio::time::Instant;
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;
use tokio_tungstenite::tungstenite::protocol::CloseFrame;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::WebSocketStream;

use crate::acp::{NdjsonReader, NdjsonWriter};
use crate::session::SessionManager;

const PING_INTERVAL: Duration = Duration::from_secs(30);
const PONG_TIMEOUT: Duration = Duration::from_secs(60);

/// Extract the ACP method name from a JSON string, if present.
fn extract_method(json: &str) -> Option<String> {
    serde_json::from_str::<serde_json::Value>(json)
        .ok()
        .and_then(|v| v.get("method")?.as_str().map(String::from))
}

/// Extract the copilot session ID from an ACP session/new response result.
fn extract_session_id_from_result(json: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(json).ok()?;
    v.get("result")?
        .get("sessionId")
        .and_then(|s| s.as_str().map(String::from))
}

/// Relay messages bidirectionally between a WebSocket connection and an NDJSON TCP connection.
/// Sends periodic WebSocket pings, monitors pong responses, and respects idle-timeout disconnects.
pub async fn relay<S>(
    ws_stream: WebSocketStream<S>,
    mut tcp_reader: NdjsonReader,
    mut tcp_writer: NdjsonWriter,
    sm: SessionManager,
    session_id: &str,
) where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let (mut ws_sink, mut ws_stream) = ws_stream.split();

    // Channel for messages that need to be sent to the WebSocket sink
    let (ws_tx, mut ws_rx) = mpsc::channel::<Message>(64);

    // Task: forward channel messages to ws_sink
    let sink_task = tokio::spawn(async move {
        while let Some(msg) = ws_rx.recv().await {
            if let Err(e) = ws_sink.send(msg).await {
                tracing::error!("Failed to send to WebSocket: {}", e);
                break;
            }
        }
    });

    let ws_tx2 = ws_tx.clone();
    let ws_tx_ping = ws_tx.clone();
    let sm_ws = sm.clone();
    let sid_ws = session_id.to_string();
    let sm_tcp = sm.clone();
    let sid_tcp = session_id.to_string();
    let sm_idle = sm.clone();
    let sid_idle = session_id.to_string();

    // Track when we last received a pong (or any client data)
    let last_pong = Arc::new(tokio::sync::Mutex::new(Instant::now()));
    let last_pong_reader = last_pong.clone();

    let mut ping_interval = tokio::time::interval(PING_INTERVAL);

    tokio::select! {
        // WS → TCP: read from WebSocket, write to TCP as NDJSON
        _ = async {
            while let Some(msg) = ws_stream.next().await {
                match msg {
                    Ok(Message::Text(text)) => {
                        let trimmed = text.trim();
                        if trimmed.is_empty() {
                            continue;
                        }

                        sm_ws.record_activity(&sid_ws).await;

                        if let Some(method) = extract_method(trimmed) {
                            if method == "session/prompt" {
                                sm_ws.increment_prompts(&sid_ws).await;
                            }
                        }

                        if let Err(e) = tcp_writer.write_line(trimmed).await {
                            tracing::error!("Failed to write to TCP: {}", e);
                            break;
                        }
                        tracing::debug!("WS→TCP: {}", truncate(trimmed, 200));
                    }
                    Ok(Message::Close(_)) => {
                        tracing::info!("WebSocket client sent close frame");
                        break;
                    }
                    Ok(Message::Ping(data)) => {
                        let _ = ws_tx2.send(Message::Pong(data)).await;
                    }
                    Ok(Message::Pong(_)) => {
                        *last_pong_reader.lock().await = Instant::now();
                        tracing::trace!("Pong received");
                    }
                    Ok(_) => {}
                    Err(e) => {
                        tracing::error!("WebSocket read error: {}", e);
                        break;
                    }
                }
            }
        } => {
            tracing::info!("WS→TCP relay ended");
        },

        // TCP → WS: read NDJSON lines from TCP, send via channel
        _ = async {
            loop {
                match tcp_reader.read_line().await {
                    Ok(Some(line)) => {
                        tracing::debug!("TCP→WS: {}", truncate(&line, 200));

                        sm_tcp.record_activity(&sid_tcp).await;

                        if let Some(method) = extract_method(&line) {
                            if method == "session/update" {
                                sm_tcp.increment_messages(&sid_tcp).await;
                            }
                        }

                        if let Some(copilot_sid) = extract_session_id_from_result(&line) {
                            tracing::info!("Captured copilot session ID: {}", copilot_sid);
                            sm_tcp.set_copilot_session_id(&sid_tcp, copilot_sid).await;
                        }

                        if ws_tx.send(Message::Text(line)).await.is_err() {
                            break;
                        }
                    }
                    Ok(None) => {
                        tracing::info!("TCP connection closed (EOF)");
                        break;
                    }
                    Err(e) => {
                        tracing::error!("TCP read error: {}", e);
                        break;
                    }
                }
            }
        } => {
            tracing::info!("TCP→WS relay ended");
        },

        // Periodic WebSocket ping and pong-timeout check
        _ = async {
            loop {
                ping_interval.tick().await;
                let elapsed = Instant::now().duration_since(*last_pong.lock().await);
                if elapsed > PONG_TIMEOUT {
                    tracing::warn!("No pong received in {:.0}s — connection presumed dead", elapsed.as_secs_f64());
                    break;
                }
                if ws_tx_ping.send(Message::Ping(vec![])).await.is_err() {
                    break;
                }
                tracing::trace!("Ping sent");
            }
        } => {
            tracing::info!("Ping/pong keepalive ended — closing connection");
        },

        // Idle session timeout: poll the session manager for disconnect status
        _ = async {
            let mut check = tokio::time::interval(Duration::from_secs(5));
            loop {
                check.tick().await;
                if sm_idle.is_disconnected(&sid_idle).await {
                    break;
                }
            }
        } => {
            tracing::info!(session_id = session_id, "Session idle timeout — sending close frame");
            let close = Message::Close(Some(CloseFrame {
                code: CloseCode::Away,
                reason: "idle timeout".into(),
            }));
            let _ = ws_tx2.send(close).await;
            // Give the sink task a moment to flush the close frame
            tokio::time::sleep(Duration::from_millis(250)).await;
        },
    }

    // Drop senders so the sink task finishes
    drop(ws_tx);
    drop(ws_tx2);
    drop(ws_tx_ping);
    let _ = sink_task.await;
}

fn truncate(s: &str, max: usize) -> &str {
    if s.len() <= max {
        s
    } else {
        &s[..max]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_method_present() {
        let json = r#"{"jsonrpc":"2.0","id":1,"method":"session/prompt","params":{}}"#;
        assert_eq!(extract_method(json), Some("session/prompt".to_string()));
    }

    #[test]
    fn test_extract_method_absent() {
        let json = r#"{"jsonrpc":"2.0","id":1,"result":{}}"#;
        assert_eq!(extract_method(json), None);
    }

    #[test]
    fn test_extract_method_invalid_json() {
        assert_eq!(extract_method("not json"), None);
    }

    #[test]
    fn test_extract_method_notification() {
        let json = r#"{"jsonrpc":"2.0","method":"session/update","params":{"data":"test"}}"#;
        assert_eq!(extract_method(json), Some("session/update".to_string()));
    }

    #[test]
    fn test_extract_session_id_from_result_present() {
        let json = r#"{"jsonrpc":"2.0","id":1,"result":{"sessionId":"copilot-abc-123"}}"#;
        assert_eq!(
            extract_session_id_from_result(json),
            Some("copilot-abc-123".to_string())
        );
    }

    #[test]
    fn test_extract_session_id_from_result_absent() {
        let json = r#"{"jsonrpc":"2.0","id":1,"result":{"status":"ok"}}"#;
        assert_eq!(extract_session_id_from_result(json), None);
    }

    #[test]
    fn test_extract_session_id_no_result() {
        let json = r#"{"jsonrpc":"2.0","id":1,"method":"test"}"#;
        assert_eq!(extract_session_id_from_result(json), None);
    }

    #[test]
    fn test_extract_session_id_invalid_json() {
        assert_eq!(extract_session_id_from_result("not json"), None);
    }

    #[test]
    fn test_truncate_short_string() {
        assert_eq!(truncate("hello", 10), "hello");
    }

    #[test]
    fn test_truncate_exact_length() {
        assert_eq!(truncate("hello", 5), "hello");
    }

    #[test]
    fn test_truncate_long_string() {
        assert_eq!(truncate("hello world", 5), "hello");
    }

    #[test]
    fn test_truncate_empty() {
        assert_eq!(truncate("", 5), "");
    }
}
