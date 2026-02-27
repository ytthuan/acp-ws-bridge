//! Bridge logic connecting ACP streams to WebSocket clients.

use tokio::net::TcpListener;
use tokio_rustls::TlsAcceptor;
use tokio_tungstenite::accept_async;

use crate::acp;
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
                // Create a session for this connection
                let session_info = sm.create_session().await;
                let session_id = session_info.id.clone();
                tracing::info!("Created session {} for {}", session_id, peer_addr);

                if let Err(e) =
                    handle_connection(stream, peer_addr, &copilot_host, copilot_port, tls_acceptor, sm.clone(), &session_id)
                        .await
                {
                    tracing::error!("Connection error for {} (session {}): {}", peer_addr, session_id, e);
                    sm.update_status(&session_id, SessionStatus::Error).await;
                }

                sm.update_status(&session_id, SessionStatus::Disconnected).await;
                tracing::info!("Connection closed for {} (session {})", peer_addr, session_id);
            });
        }
    }
}

async fn handle_connection(
    stream: tokio::net::TcpStream,
    peer_addr: std::net::SocketAddr,
    copilot_host: &str,
    copilot_port: u16,
    tls_acceptor: Option<TlsAcceptor>,
    sm: SessionManager,
    session_id: &str,
) -> anyhow::Result<()> {
    // Connect to Copilot CLI
    let (tcp_reader, tcp_writer) = acp::connect(copilot_host, copilot_port).await?;
    sm.update_status(session_id, SessionStatus::Active).await;

    if let Some(acceptor) = tls_acceptor {
        // TLS path: wrap TCP stream then upgrade to WebSocket
        let tls_stream = acceptor.accept(stream).await?;
        let ws_stream = accept_async(tls_stream).await?;
        tracing::info!("WSS handshake complete for {}", peer_addr);
        ws::relay(ws_stream, tcp_reader, tcp_writer, sm, session_id).await;
    } else {
        // Plain WebSocket path
        let ws_stream = accept_async(stream).await?;
        tracing::info!("WebSocket handshake complete for {}", peer_addr);
        ws::relay(ws_stream, tcp_reader, tcp_writer, sm, session_id).await;
    }

    Ok(())
}
