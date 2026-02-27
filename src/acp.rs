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
        NdjsonWriter {
            writer: write_half,
        },
    ))
}

impl NdjsonReader {
    /// Read the next NDJSON line. Returns `None` on EOF.
    /// Validates that the line is valid JSON but returns raw bytes.
    pub async fn read_line(&mut self) -> anyhow::Result<Option<String>> {
        let mut line = String::new();
        let n = self.reader.read_line(&mut line).await?;
        if n == 0 {
            return Ok(None);
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            return Ok(None);
        }
        // Validate JSON
        serde_json::from_str::<serde_json::Value>(trimmed)?;
        Ok(Some(trimmed.to_string()))
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
