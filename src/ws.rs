//! WebSocket server handling for remote iOS client connections.

use std::sync::Arc;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::{mpsc, watch};
use tokio::time::Instant;
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;
use tokio_tungstenite::tungstenite::protocol::CloseFrame;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::WebSocketStream;

use crate::acp::{JsonRpcMessage, NdjsonReader, NdjsonWriter};
use crate::session::SessionManager;

const PING_INTERVAL: Duration = Duration::from_secs(30);
const PONG_TIMEOUT: Duration = Duration::from_secs(60);

/// Extract the ACP method name from a JSON string, if present.
fn extract_method(json: &str) -> Option<String> {
    serde_json::from_str::<JsonRpcMessage>(json)
        .ok()
        .and_then(|msg| msg.method)
}

/// Extract the copilot session ID from an ACP session/new response result.
fn extract_session_id_from_result(json: &str) -> Option<String> {
    let msg: JsonRpcMessage = serde_json::from_str(json).ok()?;
    msg.result?
        .get("sessionId")
        .and_then(|s| s.as_str().map(String::from))
}

/// Relay messages bidirectionally between a WebSocket connection and an NDJSON TCP connection.
/// The TCP connection to Copilot CLI is established **lazily** — only when the first WebSocket
/// message arrives. This allows "test connection" (ping/pong) to succeed without Copilot CLI running.
pub async fn relay_lazy<S>(
    ws_stream: WebSocketStream<S>,
    copilot_host: &str,
    copilot_port: u16,
    sm: SessionManager,
    session_id: &str,
    shutdown_rx: watch::Receiver<bool>,
) where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let (mut ws_sink, mut ws_stream_read) = ws_stream.split();

    // Channel for outbound WebSocket messages
    let (ws_tx, mut ws_rx) = mpsc::channel::<Message>(64);

    // Sink task: forward channel messages to ws_sink
    let sink_task = tokio::spawn(async move {
        while let Some(msg) = ws_rx.recv().await {
            if let Err(e) = ws_sink.send(msg).await {
                tracing::error!("Failed to send to WebSocket: {}", e);
                break;
            }
        }
    });

    let ws_tx_pong = ws_tx.clone();
    let ws_tx_ping = ws_tx.clone();
    let ws_tx_idle = ws_tx.clone();
    let sm_clone = sm.clone();
    let sid = session_id.to_string();
    let copilot_host = copilot_host.to_string();
    let mut shutdown_rx = shutdown_rx;

    let last_pong = Arc::new(tokio::sync::Mutex::new(Instant::now()));
    let last_pong_reader = last_pong.clone();
    let mut ping_interval = tokio::time::interval(PING_INTERVAL);

    // State for the lazy TCP connection
    let tcp_writer: Arc<tokio::sync::Mutex<Option<NdjsonWriter>>> =
        Arc::new(tokio::sync::Mutex::new(None));
    let tcp_writer_clone = tcp_writer.clone();

    // Channel to receive TCP→WS lines once the TCP connection is established
    let (tcp_line_tx, mut tcp_line_rx) = mpsc::channel::<String>(64);

    tokio::select! {
        // WS → TCP: read from WebSocket, connect to Copilot lazily, write to TCP
        _ = async {
            while let Some(msg) = ws_stream_read.next().await {
                match msg {
                    Ok(Message::Text(text)) => {
                        let trimmed = text.trim();
                        if trimmed.is_empty() {
                            continue;
                        }

                        sm_clone.record_activity(&sid).await;

                        if let Some(method) = extract_method(trimmed) {
                            if method == "session/prompt" {
                                sm_clone.increment_prompts(&sid).await;
                            }
                        }

                        // Lazy connect: establish TCP connection on first real message
                        {
                            let mut writer_guard = tcp_writer.lock().await;
                            if writer_guard.is_none() {
                                match crate::acp::connect(&copilot_host, copilot_port).await {
                                    Ok((tcp_reader, new_writer)) => {
                                        *writer_guard = Some(new_writer);
                                        tracing::info!("Lazy TCP connection to Copilot CLI established");

                                        // Spawn TCP reader task
                                        let ws_tx_tcp = ws_tx.clone();
                                        let sm_tcp = sm_clone.clone();
                                        let sid_tcp = sid.clone();
                                        let tcp_line_tx = tcp_line_tx.clone();
                                        tokio::spawn(async move {
                                            tcp_reader_task(tcp_reader, ws_tx_tcp, tcp_line_tx, sm_tcp, &sid_tcp).await;
                                        });
                                    }
                                    Err(e) => {
                                        tracing::error!("Failed to connect to Copilot CLI: {}", e);
                                        let error_msg = format!(
                                            r#"{{"jsonrpc":"2.0","error":{{"code":-32000,"message":"Bridge: failed to connect to Copilot CLI: {}"}}}}"#,
                                            e.to_string().replace('"', "'")
                                        );
                                        let _ = ws_tx.send(Message::Text(error_msg)).await;
                                        break;
                                    }
                                }
                            }
                        }

                        // Write to TCP
                        let mut writer_guard = tcp_writer.lock().await;
                        if let Some(ref mut writer) = *writer_guard {
                            if let Err(e) = writer.write_line(trimmed).await {
                                tracing::error!("Failed to write to TCP: {}", e);
                                break;
                            }
                            tracing::debug!("WS→TCP: {}", truncate(trimmed, 200));
                        }
                    }
                    Ok(Message::Close(_)) => {
                        tracing::info!("WebSocket client sent close frame");
                        break;
                    }
                    Ok(Message::Ping(data)) => {
                        let _ = ws_tx_pong.send(Message::Pong(data)).await;
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

        // TCP → WS: forward lines from the TCP reader task
        _ = async {
            while let Some(line) = tcp_line_rx.recv().await {
                sm.record_activity(&session_id.to_string()).await;

                if let Some(method) = extract_method(&line) {
                    if method == "session/update" {
                        sm.increment_messages(&session_id.to_string()).await;
                    }
                }

                if let Some(copilot_sid) = extract_session_id_from_result(&line) {
                    tracing::info!("Captured copilot session ID: {}", copilot_sid);
                    sm.set_copilot_session_id(&session_id.to_string(), copilot_sid).await;
                }

                if ws_tx.send(Message::Text(line)).await.is_err() {
                    break;
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

        // Idle session timeout: watch for shutdown signal from session manager
        _ = async {
            while shutdown_rx.changed().await.is_ok() {
                if *shutdown_rx.borrow() {
                    break;
                }
            }
        } => {
            tracing::info!(session_id = session_id, "Session idle timeout — sending close frame");
            let close = Message::Close(Some(CloseFrame {
                code: CloseCode::Away,
                reason: "idle timeout".into(),
            }));
            let _ = ws_tx_idle.send(close).await;
            tokio::time::sleep(Duration::from_millis(250)).await;
        },
    }

    // Cleanup
    drop(ws_tx);
    drop(ws_tx_pong);
    drop(ws_tx_ping);
    drop(ws_tx_idle);
    drop(tcp_writer_clone);
    let _ = sink_task.await;
}

/// Background task that reads NDJSON lines from TCP and forwards them.
async fn tcp_reader_task(
    mut tcp_reader: NdjsonReader,
    ws_tx: mpsc::Sender<Message>,
    _tcp_line_tx: mpsc::Sender<String>,
    sm: SessionManager,
    session_id: &str,
) {
    loop {
        match tcp_reader.read_line().await {
            Ok(Some(line)) => {
                tracing::debug!("TCP→WS: {}", truncate(&line, 200));
                sm.record_activity(session_id).await;

                if let Some(method) = extract_method(&line) {
                    if method == "session/update" {
                        sm.increment_messages(session_id).await;
                    }
                }

                if let Some(copilot_sid) = extract_session_id_from_result(&line) {
                    tracing::info!("Captured copilot session ID: {}", copilot_sid);
                    sm.set_copilot_session_id(session_id, copilot_sid).await;
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
