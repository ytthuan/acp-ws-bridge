//! ACP (Agent Communication Protocol) client implementation.
//! Handles TCP/NDJSON communication with GitHub Copilot CLI.

use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::net::TcpStream;

/// A JSON-RPC 2.0 message used in ACP.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcMessage {
    pub jsonrpc: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub method: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<serde_json::Value>,
}

/// Reader half of an NDJSON TCP connection.
pub struct NdjsonReader {
    reader: BufReader<OwnedReadHalf>,
}

/// Writer half of an NDJSON TCP connection.
pub struct NdjsonWriter {
    writer: OwnedWriteHalf,
}

/// Connect to a Copilot CLI ACP server over TCP and return split reader/writer.
pub async fn connect(host: &str, port: u16) -> anyhow::Result<(NdjsonReader, NdjsonWriter)> {
    let addr = format!("{host}:{port}");
    tracing::info!("Connecting to Copilot CLI at {}", addr);
    let stream = TcpStream::connect(&addr).await?;
    tracing::info!("Connected to Copilot CLI at {}", addr);
    let (read_half, write_half) = stream.into_split();
    Ok((
        NdjsonReader {
            reader: BufReader::new(read_half),
        },
        NdjsonWriter { writer: write_half },
    ))
}

impl NdjsonReader {
    /// Read the next NDJSON line. Returns `None` on EOF.
    /// Validates that the line is valid JSON but returns raw bytes.
    pub async fn read_line(&mut self) -> anyhow::Result<Option<String>> {
        loop {
            let mut line = String::new();
            let n = self.reader.read_line(&mut line).await?;
            if n == 0 {
                return Ok(None); // actual EOF
            }
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue; // skip empty lines
            }
            // Validate JSON
            serde_json::from_str::<serde_json::Value>(trimmed)?;
            return Ok(Some(trimmed.to_string()));
        }
    }
}

impl NdjsonWriter {
    /// Write a JSON string as an NDJSON line (appends newline).
    pub async fn write_line(&mut self, json: &str) -> anyhow::Result<()> {
        // Validate JSON before sending
        serde_json::from_str::<serde_json::Value>(json)?;
        self.writer.write_all(json.as_bytes()).await?;
        self.writer.write_all(b"\n").await?;
        self.writer.flush().await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_jsonrpc_request_serialize() {
        let msg = JsonRpcMessage {
            jsonrpc: "2.0".to_string(),
            id: Some(serde_json::json!(1)),
            method: Some("session/prompt".to_string()),
            params: Some(serde_json::json!({"sessionId": "abc"})),
            result: None,
            error: None,
        };
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains("\"jsonrpc\":\"2.0\""));
        assert!(json.contains("\"id\":1"));
        assert!(json.contains("\"method\":\"session/prompt\""));
        assert!(json.contains("\"params\""));
        // result and error should be absent (skip_serializing_if)
        assert!(!json.contains("\"result\""));
        assert!(!json.contains("\"error\""));
    }

    #[test]
    fn test_jsonrpc_response_deserialize() {
        let json = r#"{"jsonrpc":"2.0","id":1,"result":{"sessionId":"xyz"}}"#;
        let msg: JsonRpcMessage = serde_json::from_str(json).unwrap();
        assert_eq!(msg.jsonrpc, "2.0");
        assert_eq!(msg.id, Some(serde_json::json!(1)));
        assert!(msg.method.is_none());
        assert!(msg.params.is_none());
        assert_eq!(msg.result, Some(serde_json::json!({"sessionId": "xyz"})));
        assert!(msg.error.is_none());
    }

    #[test]
    fn test_jsonrpc_notification_no_id() {
        let msg = JsonRpcMessage {
            jsonrpc: "2.0".to_string(),
            id: None,
            method: Some("session/update".to_string()),
            params: Some(serde_json::json!({"data": "test"})),
            result: None,
            error: None,
        };
        let json = serde_json::to_string(&msg).unwrap();
        assert!(!json.contains("\"id\""));
        assert!(json.contains("\"method\":\"session/update\""));
    }

    #[test]
    fn test_jsonrpc_error_response() {
        let json =
            r#"{"jsonrpc":"2.0","id":2,"error":{"code":-32600,"message":"Invalid Request"}}"#;
        let msg: JsonRpcMessage = serde_json::from_str(json).unwrap();
        assert_eq!(msg.id, Some(serde_json::json!(2)));
        assert!(msg.result.is_none());
        let err = msg.error.unwrap();
        assert_eq!(err["code"], -32600);
        assert_eq!(err["message"], "Invalid Request");
    }

    #[test]
    fn test_invalid_json_rejected() {
        let bad = "not valid json at all";
        let result = serde_json::from_str::<JsonRpcMessage>(bad);
        assert!(result.is_err());
    }

    #[test]
    fn test_message_roundtrip() {
        let original = JsonRpcMessage {
            jsonrpc: "2.0".to_string(),
            id: Some(serde_json::json!(42)),
            method: Some("initialize".to_string()),
            params: Some(serde_json::json!({"capabilities": {}})),
            result: None,
            error: None,
        };
        let serialized = serde_json::to_string(&original).unwrap();
        let deserialized: JsonRpcMessage = serde_json::from_str(&serialized).unwrap();
        assert_eq!(deserialized.jsonrpc, original.jsonrpc);
        assert_eq!(deserialized.id, original.id);
        assert_eq!(deserialized.method, original.method);
        assert_eq!(deserialized.params, original.params);
    }

    #[test]
    fn test_ndjson_line_framing() {
        // Each NDJSON message should be a single JSON object; when serialized
        // the bridge appends \n. Verify serialization produces valid single-line JSON.
        let msg = JsonRpcMessage {
            jsonrpc: "2.0".to_string(),
            id: Some(serde_json::json!(1)),
            method: Some("test".to_string()),
            params: None,
            result: None,
            error: None,
        };
        let line = serde_json::to_string(&msg).unwrap();
        assert!(
            !line.contains('\n'),
            "serialized message must not contain newlines"
        );
        // Simulate NDJSON framing
        let framed = format!("{}\n", line);
        assert!(framed.ends_with('\n'));
        // Parse back without the trailing newline
        let parsed: JsonRpcMessage = serde_json::from_str(framed.trim()).unwrap();
        assert_eq!(parsed.method, Some("test".to_string()));
    }

    #[test]
    fn test_string_id_supported() {
        let json = r#"{"jsonrpc":"2.0","id":"req-1","method":"session/new","params":{}}"#;
        let msg: JsonRpcMessage = serde_json::from_str(json).unwrap();
        assert_eq!(msg.id, Some(serde_json::json!("req-1")));
    }

    #[tokio::test]
    async fn test_ndjson_reader_writer_roundtrip() {
        // Create a TCP listener and connect to test the reader/writer
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let (read_half, _write_half) = stream.into_split();
            let mut reader = NdjsonReader {
                reader: tokio::io::BufReader::new(read_half),
            };
            // Read lines sent by the client
            let line1 = reader.read_line().await.unwrap();
            let line2 = reader.read_line().await.unwrap();
            (line1, line2)
        });

        let client = tokio::spawn(async move {
            let stream = tokio::net::TcpStream::connect(addr).await.unwrap();
            let (_read_half, write_half) = stream.into_split();
            let mut writer = NdjsonWriter { writer: write_half };
            writer
                .write_line(r#"{"jsonrpc":"2.0","id":1,"method":"test"}"#)
                .await
                .unwrap();
            writer
                .write_line(r#"{"jsonrpc":"2.0","id":2,"method":"test2"}"#)
                .await
                .unwrap();
            drop(writer);
        });

        client.await.unwrap();
        let (line1, line2) = server.await.unwrap();
        assert!(line1.unwrap().contains("\"method\":\"test\""));
        assert!(line2.unwrap().contains("\"method\":\"test2\""));
    }

    #[tokio::test]
    async fn test_ndjson_writer_rejects_invalid_json() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        let _server = tokio::spawn(async move {
            let (_stream, _) = listener.accept().await.unwrap();
        });

        let stream = tokio::net::TcpStream::connect(addr).await.unwrap();
        let (_read_half, write_half) = stream.into_split();
        let mut writer = NdjsonWriter { writer: write_half };
        let result = writer.write_line("not json").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_ndjson_reader_eof() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let (read_half, _write_half) = stream.into_split();
            let mut reader = NdjsonReader {
                reader: tokio::io::BufReader::new(read_half),
            };
            reader.read_line().await
        });

        // Connect and immediately drop (EOF)
        let stream = tokio::net::TcpStream::connect(addr).await.unwrap();
        drop(stream);

        let result = server.await.unwrap().unwrap();
        assert!(result.is_none());
    }
}
