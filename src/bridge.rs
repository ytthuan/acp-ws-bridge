//! Bridge logic connecting ACP streams to WebSocket clients.

use tokio::net::TcpListener;
use tokio_native_tls::TlsAcceptor;
use tokio_tungstenite::accept_async;

use crate::config::Config;
use crate::session::{SessionManager, SessionStatus};
use crate::tls;
use crate::ws;

/// Core bridge that accepts WebSocket connections and relays to Copilot CLI.
pub struct Bridge {
    config: Config,
    session_manager: SessionManager,
}

impl Bridge {
    pub fn new(config: Config, session_manager: SessionManager) -> Self {
        Self {
            config,
            session_manager,
        }
    }

    /// Get a reference to the session manager.
    pub fn session_manager(&self) -> &SessionManager {
        &self.session_manager
    }

    /// Run the WebSocket server and handle connections.
    pub async fn run(&self) -> anyhow::Result<()> {
        let addr = format!("{}:{}", self.config.listen_addr, self.config.ws_port);
        let listener = TcpListener::bind(&addr).await?;

        // Load TLS if configured
        let tls_acceptor = match (&self.config.tls_cert, &self.config.tls_key) {
            (Some(cert), Some(key)) => {
                let acceptor = tls::load_tls_config(cert, key)?;
                tracing::info!("TLS enabled (cert: {}, key: {})", cert, key);
                Some(acceptor)
            }
            (Some(_), None) | (None, Some(_)) => {
                anyhow::bail!("Both --tls-cert and --tls-key must be provided for TLS");
            }
            _ => None,
        };

        let scheme = if tls_acceptor.is_some() { "wss" } else { "ws" };
        tracing::info!("WebSocket server listening on {}://{}", scheme, addr);

        loop {
            let (stream, peer_addr) = listener.accept().await?;
            tracing::info!("New connection from {}", peer_addr);

            let copilot_host = self.config.copilot_host.clone();
            let copilot_port = self.config.copilot_port;
            let tls_acceptor = tls_acceptor.clone();
            let sm = self.session_manager.clone();

            tokio::spawn(async move {
                // Register a session for this connection (with shutdown signaling)
                let handle = sm.register(peer_addr).await;
                let session_id = handle.id.clone();
                let shutdown_rx = handle.shutdown_rx.clone();
                tracing::info!("Registered session {} for {}", session_id, peer_addr);

                if let Err(e) =
                    handle_connection(stream, peer_addr, &copilot_host, copilot_port, tls_acceptor, sm.clone(), &session_id, shutdown_rx)
                        .await
                {
                    tracing::error!("Connection error for {} (session {}): {}", peer_addr, session_id, e);
                    sm.update_status(&session_id, SessionStatus::Error).await;
                }

                // Record final activity and clean up the session
                handle.touch().await;
                sm.unregister(&session_id).await;
                tracing::info!("Connection closed for {} (session {})", peer_addr, session_id);
            });
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn handle_connection(
    stream: tokio::net::TcpStream,
    peer_addr: std::net::SocketAddr,
    copilot_host: &str,
    copilot_port: u16,
    tls_acceptor: Option<TlsAcceptor>,
    sm: SessionManager,
    session_id: &str,
    shutdown_rx: tokio::sync::watch::Receiver<bool>,
) -> anyhow::Result<()> {
    // First, complete the WebSocket handshake (no Copilot CLI connection yet).
    // This allows "test connection" pings to succeed without Copilot CLI running.
    if let Some(acceptor) = tls_acceptor {
        let tls_stream = acceptor.accept(stream).await?;
        let ws_stream = accept_async(tls_stream).await?;
        tracing::info!("WSS handshake complete for {}", peer_addr);
        sm.update_status(session_id, SessionStatus::Active).await;
        ws::relay_lazy(ws_stream, copilot_host, copilot_port, sm, session_id, shutdown_rx).await;
    } else {
        let ws_stream = accept_async(stream).await?;
        tracing::info!("WebSocket handshake complete for {}", peer_addr);
        sm.update_status(session_id, SessionStatus::Active).await;
        ws::relay_lazy(ws_stream, copilot_host, copilot_port, sm, session_id, shutdown_rx).await;
    }

    Ok(())
}
