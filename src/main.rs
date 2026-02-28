mod acp;
mod api;
mod bridge;
mod config;
mod copilot;
mod history;
mod session;
mod tls;
mod ws;

use std::path::Path;
use std::time::Duration;

use bridge::Bridge;
use clap::Parser;
use config::Config;
use session::{SessionManager, spawn_idle_checker};
use tracing::info;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let config = Config::parse();

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| format!("acp_ws_bridge={}", config.log_level).into()),
        )
        .init();

    // Generate self-signed cert and exit if requested
    if config.generate_cert {
        let cert_path = config.tls_cert.as_deref().unwrap_or("cert.pem");
        let key_path = config.tls_key.as_deref().unwrap_or("key.pem");
        let hostnames: Vec<String> = config
            .cert_hostnames
            .split(',')
            .map(|s| s.trim().to_string())
            .collect();
        tls::generate_self_signed_cert(Path::new(cert_path), Path::new(key_path), &hostnames)?;
        info!(
            "Generated self-signed certificate: {}, {}",
            cert_path, key_path
        );
        return Ok(());
    }

    // Optionally spawn Copilot CLI as a child process
    let _copilot_process = if !config.spawn_copilot {
        info!("Copilot CLI auto-spawn disabled (--spawn-copilot false)");
        None
    } else if config.copilot_mode == "stdio" {
        // In stdio mode, each WebSocket client spawns its own Copilot CLI process.
        // No shared process needed at startup.
        info!(
            "Copilot mode is 'stdio' — each WebSocket client will spawn its own \
             Copilot CLI process via stdin/stdout pipes."
        );
        None
    } else {
        match copilot::CopilotProcess::spawn_tcp(
            &config.copilot_path,
            config.copilot_port,
            &config.copilot_args,
        )
        .await
        {
            Ok((proc, _transport)) => {
                info!("Copilot CLI running on port {}", proc.port());
                Some(proc)
            }
            Err(e) => {
                tracing::warn!(
                    "Failed to spawn Copilot CLI: {}. Bridge will still start — connect to an existing Copilot CLI instance manually.",
                    e
                );
                None
            }
        }
    };

    info!("ACP WebSocket Bridge starting...");
    info!("WebSocket: {}:{}", config.listen_addr, config.ws_port);
    if config.copilot_mode == "stdio" {
        info!("Copilot CLI mode: stdio (per-client process)");
    } else {
        info!(
            "Copilot CLI: {}:{}",
            config.copilot_host, config.copilot_port
        );
    }

    let session_manager = SessionManager::new();

    // Spawn idle session checker
    let idle_timeout = Duration::from_secs(config.idle_timeout_secs);
    let _idle_checker = spawn_idle_checker(session_manager.clone(), idle_timeout);
    info!("Idle session timeout: {}s", config.idle_timeout_secs);

    // Spawn REST API server on separate port (non-fatal if port in use)
    let api_port = config.api_port.unwrap_or(config.ws_port.saturating_add(1));
    let api_router = api::api_router(session_manager.clone());
    let api_addr = std::net::SocketAddr::from(([0, 0, 0, 0], api_port));
    info!("REST API: http://{}:{}", config.listen_addr, api_port);

    tokio::spawn(async move {
        match tokio::net::TcpListener::bind(api_addr).await {
            Ok(listener) => {
                info!("REST API listening on port {}", api_port);
                if let Err(e) = axum::serve(listener, api_router).await {
                    tracing::error!("REST API server error: {}", e);
                }
            }
            Err(e) => {
                tracing::warn!(
                    "REST API failed to bind port {}: {} (continuing without REST API)",
                    api_port, e
                );
            }
        }
    });

    let bridge = Bridge::new(config, session_manager);

    bridge.run().await
}
