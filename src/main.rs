mod acp;
mod api;
mod bridge;
mod config;
mod copilot;
mod history;
mod paths;
mod session;
mod stats_cache;
mod tls;
mod ws;

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use bridge::Bridge;
use clap::{CommandFactory, FromArgMatches};
use config::Config;
use session::{spawn_idle_checker, SessionManager};
use tracing::info;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let matches = Config::command().get_matches();
    let mut config = Config::from_arg_matches(&matches)?;

    // Auto-detect TCP mode: if --copilot-port was explicitly provided but --copilot-mode was not
    if config.copilot_mode.is_none()
        && matches.value_source("copilot_port") == Some(clap::parser::ValueSource::CommandLine)
    {
        config.copilot_mode = Some("tcp".to_string());
    }

    let uses_spawned_copilot_command =
        config.effective_copilot_mode() == "stdio" || config.spawn_copilot;

    if let Some(command) = config.acp_command.as_deref() {
        if uses_spawned_copilot_command {
            copilot::validate_command_override(command)?;
            copilot::validate_command_override_for_mode(
                command,
                config.effective_copilot_mode(),
                config.copilot_port,
            )?;
            if !config.copilot_args.is_empty() {
                tracing::warn!(
                    "--copilot-args are ignored when --acp-command/--command is provided"
                );
            }
            info!("Custom ACP command override enabled");
        } else {
            tracing::warn!(
                "--acp-command/--command is ignored when using an external Copilot TCP instance"
            );
        }
    }

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
    } else if config.effective_copilot_mode() == "stdio" {
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
            &config.copilot_host,
            config.copilot_port,
            &config.copilot_args,
            config.acp_command.as_deref(),
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
    if config.effective_copilot_mode() == "stdio" {
        info!("Copilot CLI mode: stdio (per-client process)");
    } else {
        info!(
            "Copilot CLI: {}:{}",
            config.copilot_host, config.copilot_port
        );
    }
    let copilot_dir = config.effective_copilot_dir()?;
    if config.copilot_dir.is_some() {
        info!("Custom Copilot data directory enabled");
    }

    // Detect Copilot CLI version
    let copilot_cli_version = copilot::detect_version(
        &config.copilot_path,
        if uses_spawned_copilot_command {
            config.acp_command.as_deref()
        } else {
            None
        },
    )
    .await;
    if let Some(ref v) = copilot_cli_version {
        info!("Copilot CLI version: {}", v);
    } else {
        tracing::warn!("Could not detect Copilot CLI version");
    }

    let session_manager = SessionManager::new();

    // Build stats cache and start background refresh task.
    let stats_cache = Arc::new(stats_cache::StatsCache::with_copilot_dir(
        copilot_dir.clone(),
    ));
    let cache_for_task = stats_cache.clone();
    tokio::spawn(async move {
        // Initial refresh on a blocking thread so it doesn't stall the async runtime.
        if let Err(e) = tokio::task::spawn_blocking({
            let c = cache_for_task.clone();
            move || c.refresh()
        })
        .await
        {
            tracing::error!("Stats cache refresh failed: {}", e);
        }

        loop {
            tokio::time::sleep(tokio::time::Duration::from_secs(300)).await;
            let c = cache_for_task.clone();
            if let Err(e) = tokio::task::spawn_blocking(move || c.refresh()).await {
                tracing::error!("Stats cache refresh failed: {}", e);
            }
        }
    });

    // Spawn idle session checker
    let idle_timeout = Duration::from_secs(config.idle_timeout_secs);
    let _idle_checker = spawn_idle_checker(session_manager.clone(), idle_timeout);
    info!("Idle session timeout: {}s", config.idle_timeout_secs);

    // Spawn REST API server on separate port (non-fatal if port in use)
    let api_port = config.api_port.unwrap_or(config.ws_port.saturating_add(1));
    let api_router = api::api_router(
        session_manager.clone(),
        stats_cache,
        api::CopilotInfo {
            version: copilot_cli_version,
            path: copilot::effective_command_program(
                &config.copilot_path,
                if uses_spawned_copilot_command {
                    config.acp_command.as_deref()
                } else {
                    None
                },
            ),
            mode: config.effective_copilot_mode().to_string(),
        },
        copilot_dir,
    );
    let api_addr = std::net::SocketAddr::from(([0, 0, 0, 0], api_port));

    // Load TLS config once — shared by both WebSocket and REST API servers
    let tls_acceptor = match (&config.tls_cert, &config.tls_key) {
        (Some(cert), Some(key)) => Some(tls::load_tls_config(cert, key)?),
        _ => None,
    };

    let api_tls = tls_acceptor.clone();
    if api_tls.is_some() {
        info!("REST API: https://{}:{}", config.listen_addr, api_port);
    } else {
        info!("REST API: http://{}:{}", config.listen_addr, api_port);
    }

    tokio::spawn(async move {
        match tokio::net::TcpListener::bind(api_addr).await {
            Ok(listener) => {
                info!("REST API listening on port {}", api_port);
                match api_tls {
                    Some(acceptor) => {
                        if let Err(e) = bridge::serve_with_tls(listener, api_router, acceptor).await
                        {
                            tracing::error!("REST API (HTTPS) server error: {}", e);
                        }
                    }
                    None => {
                        if let Err(e) = axum::serve(listener, api_router.into_make_service()).await
                        {
                            tracing::error!("REST API server error: {}", e);
                        }
                    }
                }
            }
            Err(e) => {
                tracing::warn!(
                    "REST API failed to bind port {}: {} (continuing without REST API)",
                    api_port,
                    e
                );
            }
        }
    });

    let bridge = Bridge::new(config, session_manager, tls_acceptor);

    bridge.run().await
}
