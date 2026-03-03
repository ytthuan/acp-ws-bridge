//! WebSocket server handling for remote iOS client connections.

use std::sync::Arc;
use std::time::Duration;

use axum::extract::ws::{CloseFrame, Message, WebSocket};
use futures_util::{SinkExt, StreamExt};
use tokio::sync::{mpsc, watch};
use tokio::time::Instant;

use crate::acp::{JsonRpcMessage, NdjsonReader, NdjsonWriter};
use crate::copilot::CopilotProcess;
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

/// Extract available_commands from an ACP session/update notification
/// with type "available_commands_update". Returns the commands value if found.
fn extract_available_commands(json: &str) -> Option<serde_json::Value> {
    let val: serde_json::Value = serde_json::from_str(json).ok()?;
    if val.get("method")?.as_str()? != "session/update" {
        return None;
    }
    let params = val.get("params")?;
    // Format: params.update.type == "available_commands_update"
    if let Some(update) = params.get("update") {
        if update.get("type").and_then(|t| t.as_str()) == Some("available_commands_update") {
            return update.get("commands").cloned();
        }
    }
    // Alt format: params.type == "available_commands_update"
    if params.get("type").and_then(|t| t.as_str()) == Some("available_commands_update") {
        return params
            .get("available_commands")
            .or_else(|| params.get("commands"))
            .cloned();
    }
    None
}

/// Relay messages bidirectionally between a WebSocket connection and an NDJSON TCP connection.
/// The TCP connection to Copilot CLI is established **lazily** — only when the first WebSocket
/// message arrives. This allows "test connection" (ping/pong) to succeed without Copilot CLI running.
pub async fn relay_lazy(
    ws_stream: WebSocket,
    copilot_host: &str,
    copilot_port: u16,
    sm: SessionManager,
    session_id: &str,
    shutdown_rx: watch::Receiver<bool>,
) {
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
                                        tokio::spawn(async move {
                                            tcp_reader_task(tcp_reader, ws_tx_tcp, sm_tcp, &sid_tcp).await;
                                        });
                                    }
                                    Err(e) => {
                                        tracing::error!("Failed to connect to Copilot CLI: {}", e);
                                        let error_msg = format!(
                                            r#"{{"jsonrpc":"2.0","id":null,"error":{{"code":-32000,"message":"Bridge: failed to connect to Copilot CLI: {}"}}}}"#,
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
                            tracing::info!("WS→CLI: {}", truncate(trimmed, 300));
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

        // Periodic WebSocket ping and pong-timeout check
        _ = async {
            loop {
                ping_interval.tick().await;
                let elapsed = Instant::now().duration_since(*last_pong.lock().await);
                if elapsed > PONG_TIMEOUT {
                    tracing::warn!("No pong received in {:.0}s — closing dead connection", elapsed.as_secs_f64());
                    let _ = ws_tx_ping.send(Message::Close(Some(CloseFrame {
                        code: 1001,
                        reason: "pong timeout".into(),
                    }))).await;
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
                code: 1001,
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

/// Relay messages between a WebSocket connection and a per-client Copilot CLI stdio process.
/// Each WebSocket client gets its own `copilot --acp --stdio` child process, spawned lazily
/// on the first inbound message.
pub async fn relay_stdio(
    ws_stream: WebSocket,
    copilot_path: &str,
    copilot_args: &[String],
    sm: SessionManager,
    session_id: &str,
    shutdown_rx: watch::Receiver<bool>,
) {
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
    let copilot_path = copilot_path.to_string();
    let copilot_args = copilot_args.to_vec();
    let mut shutdown_rx = shutdown_rx;

    let last_pong = Arc::new(tokio::sync::Mutex::new(Instant::now()));
    let last_pong_reader = last_pong.clone();
    let mut ping_interval = tokio::time::interval(PING_INTERVAL);

    // State for the lazy stdio child process
    let stdio_writer: Arc<
        tokio::sync::Mutex<Option<tokio::io::BufWriter<tokio::process::ChildStdin>>>,
    > = Arc::new(tokio::sync::Mutex::new(None));
    let stdio_writer_clone = stdio_writer.clone();

    // Hold the child process so it lives as long as the connection
    let child_process: Arc<tokio::sync::Mutex<Option<CopilotProcess>>> =
        Arc::new(tokio::sync::Mutex::new(None));
    let child_process_clone = child_process.clone();

    tokio::select! {
        // WS → stdio: read from WebSocket, spawn child lazily, write to stdin
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

                        // Lazy spawn: start stdio child process on first real message
                        {
                            let mut writer_guard = stdio_writer.lock().await;
                            if writer_guard.is_none() {
                                match CopilotProcess::spawn_stdio(&copilot_path, &copilot_args).await {
                                    Ok((proc, crate::copilot::CopilotTransport::Stdio { stdin, stdout })) => {
                                        *writer_guard = Some(tokio::io::BufWriter::new(stdin));
                                        *child_process.lock().await = Some(proc);
                                        tracing::info!("Stdio Copilot CLI process spawned for session {}", sid);

                                        // Spawn stdout reader task
                                        let ws_tx_stdout = ws_tx.clone();
                                        let sm_stdout = sm_clone.clone();
                                        let sid_stdout = sid.clone();
                                        tokio::spawn(async move {
                                            stdio_reader_task(stdout, ws_tx_stdout, sm_stdout, &sid_stdout).await;
                                        });
                                    }
                                    Ok((_, crate::copilot::CopilotTransport::Tcp { .. })) => {
                                        // Should never happen when calling spawn_stdio
                                        tracing::error!("spawn_stdio returned TCP transport unexpectedly");
                                        break;
                                    }
                                    Err(e) => {
                                        tracing::error!("Failed to spawn Copilot CLI (stdio): {}", e);
                                        let error_msg = format!(
                                            r#"{{"jsonrpc":"2.0","id":null,"error":{{"code":-32000,"message":"Bridge: failed to spawn Copilot CLI: {}"}}}}"#,
                                            e.to_string().replace('"', "'")
                                        );
                                        let _ = ws_tx.send(Message::Text(error_msg)).await;
                                        break;
                                    }
                                }
                            }
                        }

                        // Write to stdin
                        let mut writer_guard = stdio_writer.lock().await;
                        if let Some(ref mut writer) = *writer_guard {
                            use tokio::io::AsyncWriteExt;
                            // Validate JSON before sending
                            if serde_json::from_str::<serde_json::Value>(trimmed).is_err() {
                                tracing::warn!("WS→stdio: invalid JSON, skipping: {}", truncate(trimmed, 100));
                                continue;
                            }
                            let mut line = trimmed.to_string();
                            line.push('\n');
                            if let Err(e) = writer.write_all(line.as_bytes()).await {
                                tracing::error!("Failed to write to stdin: {}", e);
                                break;
                            }
                            if let Err(e) = writer.flush().await {
                                tracing::error!("Failed to flush stdin: {}", e);
                                break;
                            }
                            tracing::info!("WS→CLI: {}", truncate(trimmed, 300));
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
            tracing::info!("WS→stdio relay ended");
        },

        // Periodic WebSocket ping and pong-timeout check
        _ = async {
            loop {
                ping_interval.tick().await;
                let elapsed = Instant::now().duration_since(*last_pong.lock().await);
                if elapsed > PONG_TIMEOUT {
                    tracing::warn!("No pong received in {:.0}s — closing dead connection", elapsed.as_secs_f64());
                    let _ = ws_tx_ping.send(Message::Close(Some(CloseFrame {
                        code: 1001,
                        reason: "pong timeout".into(),
                    }))).await;
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

        // Idle session timeout
        _ = async {
            while shutdown_rx.changed().await.is_ok() {
                if *shutdown_rx.borrow() {
                    break;
                }
            }
        } => {
            tracing::info!(session_id = session_id, "Session idle timeout — sending close frame");
            let close = Message::Close(Some(CloseFrame {
                code: 1001,
                reason: "idle timeout".into(),
            }));
            let _ = ws_tx_idle.send(close).await;
            tokio::time::sleep(Duration::from_millis(250)).await;
        },
    }

    // Cleanup: drop the child process (triggers kill via Drop impl)
    drop(ws_tx);
    drop(ws_tx_pong);
    drop(ws_tx_ping);
    drop(ws_tx_idle);
    drop(stdio_writer_clone);
    drop(child_process_clone);
    let _ = sink_task.await;
}

/// Background task that reads NDJSON lines from a child process stdout and forwards to WebSocket.
async fn stdio_reader_task(
    mut stdout: tokio::io::BufReader<tokio::process::ChildStdout>,
    ws_tx: mpsc::Sender<Message>,
    sm: SessionManager,
    session_id: &str,
) {
    use tokio::io::AsyncBufReadExt;
    let mut line = String::new();
    loop {
        line.clear();
        match stdout.read_line(&mut line).await {
            Ok(0) => {
                tracing::info!("Copilot CLI stdout closed (EOF)");
                break;
            }
            Ok(_) => {
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    continue;
                }
                // Validate JSON
                if serde_json::from_str::<serde_json::Value>(trimmed).is_err() {
                    tracing::warn!(
                        "stdio→WS: non-JSON line from Copilot CLI, skipping: {}",
                        truncate(trimmed, 200)
                    );
                    continue;
                }

                tracing::info!("CLI→WS: {}", truncate(trimmed, 300));
                sm.record_activity(session_id).await;

                if let Some(method) = extract_method(trimmed) {
                    if method == "session/update" {
                        sm.increment_messages(session_id).await;
                    }
                }

                if let Some(copilot_sid) = extract_session_id_from_result(trimmed) {
                    tracing::info!("Captured copilot session ID: {}", copilot_sid);
                    sm.set_copilot_session_id(session_id, copilot_sid).await;
                }

                if let Some(commands) = extract_available_commands(trimmed) {
                    tracing::info!("Captured available_commands for session {}", session_id);
                    sm.set_available_commands(session_id, commands).await;
                }

                if ws_tx
                    .send(Message::Text(trimmed.to_string()))
                    .await
                    .is_err()
                {
                    break;
                }
            }
            Err(e) => {
                tracing::error!("Copilot CLI stdout read error: {}", e);
                break;
            }
        }
    }
}

/// Background task that reads NDJSON lines from TCP and forwards them.
async fn tcp_reader_task(
    mut tcp_reader: NdjsonReader,
    ws_tx: mpsc::Sender<Message>,
    sm: SessionManager,
    session_id: &str,
) {
    loop {
        match tcp_reader.read_line().await {
            Ok(Some(line)) => {
                tracing::info!("CLI→WS: {}", truncate(&line, 300));
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

                if let Some(commands) = extract_available_commands(&line) {
                    tracing::info!("Captured available_commands for session {}", session_id);
                    sm.set_available_commands(session_id, commands).await;
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
        let mut end = max;
        while end > 0 && !s.is_char_boundary(end) {
            end -= 1;
        }
        &s[..end]
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

    #[test]
    fn test_extract_available_commands_nested_update() {
        let json = r#"{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"s1","update":{"type":"available_commands_update","commands":[{"name":"explain"},{"name":"fix"}]}}}"#;
        let cmds = extract_available_commands(json).unwrap();
        assert_eq!(cmds, serde_json::json!([{"name":"explain"},{"name":"fix"}]));
    }

    #[test]
    fn test_extract_available_commands_flat_format() {
        let json = r#"{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"s1","type":"available_commands_update","available_commands":["cmd1","cmd2"]}}"#;
        let cmds = extract_available_commands(json).unwrap();
        assert_eq!(cmds, serde_json::json!(["cmd1", "cmd2"]));
    }

    #[test]
    fn test_extract_available_commands_not_commands_update() {
        let json = r#"{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"s1","type":"turn_update","data":{}}}"#;
        assert!(extract_available_commands(json).is_none());
    }

    #[test]
    fn test_extract_available_commands_wrong_method() {
        let json = r#"{"jsonrpc":"2.0","method":"session/prompt","params":{}}"#;
        assert!(extract_available_commands(json).is_none());
    }

    #[test]
    fn test_extract_available_commands_invalid_json() {
        assert!(extract_available_commands("not json").is_none());
    }
}
