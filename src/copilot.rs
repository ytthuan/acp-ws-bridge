//! Copilot CLI child-process manager.

use std::process::Stdio;
use tokio::process::{Child, Command};
use tokio::time::{sleep, Duration};

/// Manages the Copilot CLI child process.
pub struct CopilotProcess {
    child: Child,
    port: u16,
}

impl CopilotProcess {
    /// Spawn `copilot --acp --port <port>` and wait until it's ready.
    pub async fn spawn(
        copilot_path: &str,
        port: u16,
        extra_args: &[String],
    ) -> anyhow::Result<Self> {
        tracing::info!(
            "Spawning Copilot CLI: {} --acp --port {}",
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
            .stderr(Stdio::piped());

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

        Ok(Self { child, port })
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
