//! Copilot CLI child-process manager.

use std::process::Stdio;
use tokio::process::{Child, Command};
use tokio::time::{sleep, Duration};

#[derive(Clone, Debug, PartialEq, Eq)]
struct CopilotSpawnCommand {
    program: String,
    args: Vec<String>,
    display: String,
}

impl CopilotSpawnCommand {
    fn default_tcp(copilot_path: &str, port: u16, extra_args: &[String]) -> Self {
        let mut args = vec![
            "--acp".to_string(),
            "--port".to_string(),
            port.to_string(),
            "--resume".to_string(),
        ];
        args.extend(extra_args.iter().cloned());

        let mut display = format!("{copilot_path} --acp --port {port} --resume");
        if !extra_args.is_empty() {
            display.push(' ');
            display.push_str(&extra_args.join(" "));
        }

        Self {
            program: copilot_path.to_string(),
            args,
            display,
        }
    }

    fn default_stdio(copilot_path: &str, extra_args: &[String]) -> Self {
        let mut args = vec![
            "--acp".to_string(),
            "--stdio".to_string(),
            "--resume".to_string(),
        ];
        args.extend(extra_args.iter().cloned());

        let mut display = format!("{copilot_path} --acp --stdio --resume");
        if !extra_args.is_empty() {
            display.push(' ');
            display.push_str(&extra_args.join(" "));
        }

        Self {
            program: copilot_path.to_string(),
            args,
            display,
        }
    }

    fn exact_override(raw: &str) -> anyhow::Result<Self> {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            anyhow::bail!("Custom ACP command cannot be empty")
        }

        let tokens = shlex::split(trimmed)
            .ok_or_else(|| anyhow::anyhow!("Custom ACP command has invalid shell-style quoting"))?;
        let (program, args) = tokens
            .split_first()
            .ok_or_else(|| anyhow::anyhow!("Custom ACP command cannot be empty"))?;

        Ok(Self {
            program: program.clone(),
            args: args.to_vec(),
            display: "custom ACP command override".to_string(),
        })
    }

    fn into_command(self) -> Command {
        let mut cmd = Command::new(self.program);
        cmd.args(self.args);
        cmd
    }
}

pub fn validate_command_override(command: &str) -> anyhow::Result<()> {
    CopilotSpawnCommand::exact_override(command).map(|_| ())
}

pub fn validate_command_override_for_mode(
    command: &str,
    mode: &str,
    tcp_port: u16,
) -> anyhow::Result<()> {
    let parsed = CopilotSpawnCommand::exact_override(command)?;
    if !parsed.args.iter().any(|arg| arg == "--acp") {
        anyhow::bail!("Custom ACP command must include --acp")
    }

    match mode {
        "stdio" => {
            if !parsed.args.iter().any(|arg| arg == "--stdio") {
                anyhow::bail!(
                    "Custom ACP command must include --stdio when --copilot-mode is stdio"
                );
            }
            if tcp_port_arg(&parsed.args).is_some() {
                anyhow::bail!(
                    "Custom ACP command cannot include --port when --copilot-mode is stdio"
                );
            }
        }
        "tcp" => {
            if parsed.args.iter().any(|arg| arg == "--stdio") {
                anyhow::bail!(
                    "Custom ACP command cannot include --stdio when --copilot-mode is tcp"
                );
            }
            match tcp_port_arg(&parsed.args) {
                Some(port) if port == tcp_port => {}
                Some(port) => {
                    anyhow::bail!(
                        "Custom ACP command uses --port {}, but --copilot-port is configured as {}",
                        port,
                        tcp_port
                    );
                }
                None => {
                    anyhow::bail!(
                        "Custom ACP command must include --port {} when --copilot-mode is tcp",
                        tcp_port
                    );
                }
            }
        }
        _ => {}
    }

    Ok(())
}

pub fn effective_command_program(copilot_path: &str, custom_command: Option<&str>) -> String {
    custom_command
        .and_then(|command| CopilotSpawnCommand::exact_override(command).ok())
        .map(|command| command.program)
        .unwrap_or_else(|| copilot_path.to_string())
}

fn tcp_port_arg(args: &[String]) -> Option<u16> {
    let mut args_iter = args.iter();
    while let Some(arg) = args_iter.next() {
        if arg == "--port" {
            return args_iter.next()?.parse().ok();
        }
        if let Some(value) = arg.strip_prefix("--port=") {
            return value.parse().ok();
        }
    }
    None
}

fn version_probe_command(
    copilot_path: &str,
    custom_command: Option<&str>,
) -> anyhow::Result<CopilotSpawnCommand> {
    if let Some(command) = custom_command {
        let parsed = CopilotSpawnCommand::exact_override(command)?;
        let acp_index = parsed
            .args
            .iter()
            .position(|arg| arg == "--acp")
            .unwrap_or(parsed.args.len());

        let mut args = parsed.args.into_iter().take(acp_index).collect::<Vec<_>>();
        args.push("--version".to_string());

        return Ok(CopilotSpawnCommand {
            program: parsed.program,
            args,
            display: "custom ACP version probe".to_string(),
        });
    }

    Ok(CopilotSpawnCommand {
        program: copilot_path.to_string(),
        args: vec!["--version".to_string()],
        display: format!("{copilot_path} --version"),
    })
}

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
        copilot_host: &str,
        port: u16,
        extra_args: &[String],
        custom_command: Option<&str>,
    ) -> anyhow::Result<(Self, CopilotTransport)> {
        let spawn_command = match custom_command {
            Some(command) => CopilotSpawnCommand::exact_override(command)?,
            None => CopilotSpawnCommand::default_tcp(copilot_path, port, extra_args),
        };
        tracing::info!("Spawning Copilot CLI (TCP): {}", spawn_command.display);

        let mut cmd = spawn_command.into_command();
        cmd.env("COPILOT_CLI", "1") // Let git hooks detect Copilot CLI subprocesses
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
        let ready = wait_for_port(copilot_host, port, Duration::from_secs(30)).await;
        if !ready {
            tracing::warn!(
                "Copilot CLI may not be ready yet ({}:{} not reachable after 30s), proceeding anyway",
                copilot_host,
                port
            );
        } else {
            tracing::info!("Copilot CLI ready on {}:{}", copilot_host, port);
        }

        Ok((Self { child, port }, CopilotTransport::Tcp { port }))
    }

    /// Spawn `copilot --acp --stdio` and return piped stdin/stdout.
    pub async fn spawn_stdio(
        copilot_path: &str,
        extra_args: &[String],
        custom_command: Option<&str>,
    ) -> anyhow::Result<(Self, CopilotTransport)> {
        let spawn_command = match custom_command {
            Some(command) => CopilotSpawnCommand::exact_override(command)?,
            None => CopilotSpawnCommand::default_stdio(copilot_path, extra_args),
        };
        tracing::info!("Spawning Copilot CLI (stdio): {}", spawn_command.display);

        let mut cmd = spawn_command.into_command();
        cmd.env("COPILOT_CLI", "1") // Let git hooks detect Copilot CLI subprocesses
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

        let stdin = child.stdin.take().ok_or_else(|| {
            anyhow::anyhow!("Failed to capture stdin pipe from Copilot CLI process")
        })?;
        let stdout = child.stdout.take().ok_or_else(|| {
            anyhow::anyhow!("Failed to capture stdout pipe from Copilot CLI process")
        })?;

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

/// Detect the installed Copilot CLI version by running `copilot --version`.
pub async fn detect_version(copilot_path: &str, custom_command: Option<&str>) -> Option<String> {
    let probe = version_probe_command(copilot_path, custom_command).ok()?;
    let mut cmd = probe.into_command();
    match cmd
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .await
    {
        Ok(output) if output.status.success() => {
            let raw = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if raw.is_empty() {
                None
            } else {
                Some(raw)
            }
        }
        Ok(_) => None,
        Err(e) => {
            tracing::debug!("Could not detect Copilot CLI version: {}", e);
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_custom_command_parses_with_quotes() {
        let command = CopilotSpawnCommand::exact_override(
            r#"copilot --acp --stdio --profile "allow all tools""#,
        )
        .unwrap();

        assert_eq!(command.program, "copilot");
        assert_eq!(
            command.args,
            vec![
                "--acp".to_string(),
                "--stdio".to_string(),
                "--profile".to_string(),
                "allow all tools".to_string()
            ]
        );
    }

    #[test]
    fn test_custom_command_rejects_empty_string() {
        assert!(validate_command_override("   ").is_err());
    }

    #[test]
    fn test_validate_command_override_for_stdio_mode() {
        assert!(
            validate_command_override_for_mode("copilot --acp --stdio --yolo", "stdio", 3000)
                .is_ok()
        );
        assert!(
            validate_command_override_for_mode("copilot --acp --port 3000", "stdio", 3000).is_err()
        );
    }

    #[test]
    fn test_validate_command_override_for_tcp_mode() {
        assert!(
            validate_command_override_for_mode("copilot --acp --port 3000", "tcp", 3000).is_ok()
        );
        assert!(
            validate_command_override_for_mode("copilot --acp --port=4000", "tcp", 3000).is_err()
        );
        assert!(validate_command_override_for_mode(
            "copilot --acp --stdio --port 3000",
            "tcp",
            3000
        )
        .is_err());
    }

    #[test]
    fn test_default_stdio_command_includes_resume_and_extra_args() {
        let command =
            CopilotSpawnCommand::default_stdio("copilot", &["--allow-all-tools".to_string()]);

        assert_eq!(command.program, "copilot");
        assert_eq!(
            command.args,
            vec![
                "--acp".to_string(),
                "--stdio".to_string(),
                "--resume".to_string(),
                "--allow-all-tools".to_string()
            ]
        );
    }

    #[test]
    fn test_version_probe_strips_acp_flags_from_custom_command() {
        let command =
            version_probe_command("copilot", Some("node /tmp/copilot.js --acp --stdio --yolo"))
                .unwrap();

        assert_eq!(command.program, "node");
        assert_eq!(
            command.args,
            vec!["/tmp/copilot.js".to_string(), "--version".to_string()]
        );
    }

    #[test]
    fn test_effective_command_program_prefers_override_program() {
        assert_eq!(
            effective_command_program("copilot", Some("node /tmp/copilot.js --acp --stdio")),
            "node"
        );
    }
}
