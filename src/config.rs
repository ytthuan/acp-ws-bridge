//! Configuration types and loading.

use clap::Parser;

#[derive(Parser, Debug, Clone)]
#[command(name = "acp-ws-bridge", about = "WebSocket bridge for GitHub Copilot CLI ACP")]
pub struct Config {
    /// WebSocket server listen port
    #[arg(long, default_value = "8765")]
    pub ws_port: u16,

    /// Copilot CLI ACP TCP port
    #[arg(long, default_value = "3000")]
    pub copilot_port: u16,

    /// Copilot CLI ACP TCP host
    #[arg(long, default_value = "127.0.0.1")]
    pub copilot_host: String,

    /// WebSocket listen address
    #[arg(long, default_value = "0.0.0.0")]
    pub listen_addr: String,

    /// TLS certificate path (enables TLS if set)
    #[arg(long)]
    pub tls_cert: Option<String>,

    /// TLS key path
    #[arg(long)]
    pub tls_key: Option<String>,

    /// Idle timeout in seconds (default 7 days)
    #[arg(long, default_value = "604800")]
    pub idle_timeout_secs: u64,

    /// Generate self-signed certificate and exit
    #[arg(long)]
    pub generate_cert: bool,

    /// Hostnames for self-signed certificate (comma-separated)
    #[arg(long, default_value = "localhost,127.0.0.1")]
    pub cert_hostnames: String,

    /// REST API listen port (default: ws_port + 1)
    #[arg(long)]
    pub api_port: Option<u16>,

    /// Log level
    #[arg(long, default_value = "info")]
    pub log_level: String,
}
