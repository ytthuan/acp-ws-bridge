//! Copilot CLI child-process manager.

use std::process::Stdio;
use tokio::process::{Child, Command};
use tokio::time::{sleep, Duration};

/// Describes how the bridge communicates with the Copilot CLI process.
#[allow(dead_code)]
pub enum CopilotTransport {
    /// TCP mode — Copilot CLI listens on a port, we connect to it.
    Tcp { port: u16 },
    /// Stdio mode — we own the child's stdin/stdout pipes directly.
    Stdio {
        stdin: tokio::process::ChildStdin,
        stdout: tokio::io::BufReader<tokio::process::ChildStdout>,
    },
}

/// Manages the Copilot CLI child process.
pub struct CopilotProcess {
    child: Child,
    port: u16,
}

impl CopilotProcess {
    /// Spawn `copilot --acp --port <port>` (TCP mode) and wait until it's ready.
    pub async fn spawn_tcp(
        copilot_path: &str,
        port: u16,
        extra_args: &[String],
    ) -> anyhow::Result<(Self, CopilotTransport)> {
        tracing::info!(
            "Spawning Copilot CLI (TCP): {} --acp --port {}",
            copilot_path,
            port
        );

        let mut cmd = Command::new(copilot_path);
        cmd.arg("--acp")
            .arg("--port")
            .arg(port.to_string())
            .args(extra_args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());

        let child = cmd.spawn().map_err(|e| {
            anyhow::anyhow!(
                "Failed to spawn '{}': {}. Make sure Copilot CLI is installed and authenticated.",
                copilot_path,
                e
            )
        })?;

        tracing::info!("Copilot CLI spawned (PID: {:?})", child.id());

        // Wait for the TCP port to become available
        let ready = wait_for_port("127.0.0.1", port, Duration::from_secs(30)).await;
        if !ready {
            tracing::warn!(
                "Copilot CLI may not be ready yet (port {} not open after 30s), proceeding anyway",
                port
            );
        } else {
            tracing::info!("Copilot CLI ready on port {}", port);
        }

        Ok((Self { child, port }, CopilotTransport::Tcp { port }))
    }

    /// Spawn `copilot --acp --stdio` and return piped stdin/stdout.
    pub async fn spawn_stdio(
        copilot_path: &str,
        extra_args: &[String],
    ) -> anyhow::Result<(Self, CopilotTransport)> {
        tracing::info!(
            "Spawning Copilot CLI (stdio): {} --acp --stdio",
            copilot_path,
        );

        let mut cmd = Command::new(copilot_path);
        cmd.arg("--acp")
            .arg("--stdio")
            .args(extra_args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());

        let mut child = cmd.spawn().map_err(|e| {
            anyhow::anyhow!(
                "Failed to spawn '{}': {}. Make sure Copilot CLI is installed and authenticated.",
                copilot_path,
                e
            )
        })?;

        let stdin = child.stdin.take().ok_or_else(|| anyhow::anyhow!("Failed to capture stdin pipe from Copilot CLI process"))?;
        let stdout = child.stdout.take().ok_or_else(|| anyhow::anyhow!("Failed to capture stdout pipe from Copilot CLI process"))?;

        tracing::info!("Copilot CLI spawned in stdio mode (PID: {:?})", child.id());

        Ok((
            Self { child, port: 0 },
            CopilotTransport::Stdio {
                stdin,
                stdout: tokio::io::BufReader::new(stdout),
            },
        ))
    }

    pub fn port(&self) -> u16 {
        self.port
    }
}

impl Drop for CopilotProcess {
    fn drop(&mut self) {
        tracing::info!("Shutting down Copilot CLI");
        let _ = self.child.start_kill();
    }
}

/// Wait for a TCP port to become available.
async fn wait_for_port(host: &str, port: u16, timeout: Duration) -> bool {
    let addr = format!("{}:{}", host, port);
    let start = std::time::Instant::now();

    while start.elapsed() < timeout {
        match tokio::net::TcpStream::connect(&addr).await {
            Ok(_) => return true,
            Err(_) => {
                sleep(Duration::from_millis(500)).await;
            }
        }
    }

    false
}
