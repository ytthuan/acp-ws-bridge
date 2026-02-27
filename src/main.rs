mod acp;
mod api;
mod bridge;
mod config;
mod session;
mod tls;
mod ws;

use clap::Parser;
use tracing::info;

#[derive(Parser, Debug)]
#[command(name = "acp-ws-bridge", about = "WebSocket bridge for GitHub Copilot CLI (ACP)")]
struct Args {
    /// Port to listen on for WebSocket connections
    #[arg(short, long, default_value_t = 8443)]
    port: u16,

    /// Host to bind to
    #[arg(long, default_value = "0.0.0.0")]
    host: String,

    /// Path to the Copilot CLI executable
    #[arg(long, default_value = "github-copilot-cli")]
    copilot_cli: String,

    /// Enable TLS with auto-generated self-signed certificate
    #[arg(long, default_value_t = true)]
    tls: bool,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "acp_ws_bridge=info".into()),
        )
        .init();

    let args = Args::parse();

    info!(
        "ACP WebSocket Bridge starting on {}:{}",
        args.host, args.port
    );

    // TODO: Initialize bridge components
    // 1. Generate or load TLS certificate
    // 2. Start ACP connection to Copilot CLI
    // 3. Start WebSocket server
    // 4. Bridge messages between ACP and WebSocket

    info!("Bridge ready. Waiting for connections...");

    Ok(())
}
