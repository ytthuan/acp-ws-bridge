//! Bridge logic connecting ACP streams to WebSocket clients.

use std::convert::Infallible;
use std::future::Future;
use std::net::SocketAddr;
use std::pin::Pin;

use axum::extract::connect_info::ConnectInfo;
use axum::extract::ws::WebSocketUpgrade;
use axum::extract::State;
use axum::response::IntoResponse;
use axum::routing::get;
use axum::Router;

use crate::config::Config;
use crate::session::{SessionManager, SessionStatus};
use crate::ws;

/// Core bridge that accepts WebSocket connections and relays to Copilot CLI.
pub struct Bridge {
    config: Config,
    session_manager: SessionManager,
    tls_acceptor: Option<tokio_native_tls::TlsAcceptor>,
}

/// Shared state for WebSocket upgrade handlers.
#[derive(Clone)]
struct WsState {
    session_manager: SessionManager,
    copilot_host: String,
    copilot_port: u16,
    copilot_mode: String,
    copilot_path: String,
    acp_command: Option<String>,
    copilot_args: Vec<String>,
}

impl Bridge {
    pub fn new(
        config: Config,
        session_manager: SessionManager,
        tls_acceptor: Option<tokio_native_tls::TlsAcceptor>,
    ) -> Self {
        Self {
            config,
            session_manager,
            tls_acceptor,
        }
    }

    /// Run the WebSocket-only server.
    pub async fn run(&self) -> anyhow::Result<()> {
        let addr = format!("{}:{}", self.config.listen_addr, self.config.ws_port);

        let ws_state = WsState {
            session_manager: self.session_manager.clone(),
            copilot_host: self.config.copilot_host.clone(),
            copilot_port: self.config.copilot_port,
            copilot_mode: self.config.effective_copilot_mode().to_string(),
            copilot_path: self.config.copilot_path.clone(),
            acp_command: self.config.acp_command.clone(),
            copilot_args: self.config.copilot_args.clone(),
        };

        // WebSocket-only router (REST API runs on a separate port)
        let app = Router::new()
            .route("/ws", get(ws_upgrade_handler))
            .route("/", get(ws_upgrade_handler))
            .with_state(ws_state);

        let listener = tokio::net::TcpListener::bind(&addr).await?;

        match &self.tls_acceptor {
            Some(acceptor) => {
                tracing::info!("TLS enabled for WebSocket server");
                tracing::info!("WebSocket listening on wss://{}", addr);
                serve_with_tls(listener, app, acceptor.clone()).await
            }
            None => {
                if self.config.tls_cert.is_some() || self.config.tls_key.is_some() {
                    anyhow::bail!("Both --tls-cert and --tls-key must be provided for TLS");
                }
                tracing::info!("WebSocket listening on ws://{}", addr);
                axum::serve(
                    listener,
                    app.into_make_service_with_connect_info::<SocketAddr>(),
                )
                .await?;
                Ok(())
            }
        }
    }
}

/// WebSocket upgrade handler for both `/ws` and `/` routes.
async fn ws_upgrade_handler(
    ws: WebSocketUpgrade,
    ConnectInfo(peer_addr): ConnectInfo<SocketAddr>,
    State(state): State<WsState>,
) -> impl IntoResponse {
    tracing::info!("WebSocket upgrade request from {}", peer_addr);
    ws.on_upgrade(move |socket| handle_ws_connection(socket, peer_addr, state))
}

/// Handle an upgraded WebSocket connection.
async fn handle_ws_connection(
    socket: axum::extract::ws::WebSocket,
    peer_addr: SocketAddr,
    state: WsState,
) {
    let sm = state.session_manager.clone();
    let handle = sm.register(peer_addr).await;
    let session_id = handle.id.clone();
    let shutdown_rx = handle.shutdown_rx.clone();
    tracing::info!("Registered session {} for {}", session_id, peer_addr);

    sm.update_status(&session_id, SessionStatus::Active).await;

    if state.copilot_mode == "stdio" {
        ws::relay_stdio(
            socket,
            &state.copilot_path,
            state.acp_command.as_deref(),
            &state.copilot_args,
            sm.clone(),
            &session_id,
            shutdown_rx,
        )
        .await;
    } else {
        ws::relay_lazy(
            socket,
            &state.copilot_host,
            state.copilot_port,
            sm.clone(),
            &session_id,
            shutdown_rx,
        )
        .await;
    }

    handle.touch().await;
    sm.unregister(&session_id).await;
    tracing::info!(
        "Connection closed for {} (session {})",
        peer_addr,
        session_id
    );
}

/// Hyper service adapter that wraps the axum Router for TLS connections,
/// injecting ConnectInfo and converting between hyper and axum body types.
#[derive(Clone)]
pub(crate) struct TlsService {
    router: Router,
    peer_addr: SocketAddr,
}

impl hyper::service::Service<hyper::Request<hyper::body::Incoming>> for TlsService {
    type Response = axum::http::Response<axum::body::Body>;
    type Error = Infallible;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn call(&self, req: hyper::Request<hyper::body::Incoming>) -> Self::Future {
        let mut router = self.router.clone();
        let peer_addr = self.peer_addr;
        Box::pin(async move {
            let (mut parts, body) = req.into_parts();
            parts.extensions.insert(ConnectInfo(peer_addr));
            let req = axum::http::Request::from_parts(parts, axum::body::Body::new(body));
            // Router is always ready; safe to call without poll_ready
            tower::Service::call(&mut router, req).await
        })
    }
}

/// Serve the axum app over TLS using hyper-util for each connection.
pub(crate) async fn serve_with_tls(
    listener: tokio::net::TcpListener,
    app: Router,
    tls_acceptor: tokio_native_tls::TlsAcceptor,
) -> anyhow::Result<()> {
    use hyper_util::rt::{TokioExecutor, TokioIo};
    use hyper_util::server::conn::auto;

    loop {
        let (tcp_stream, peer_addr) = match listener.accept().await {
            Ok(conn) => conn,
            Err(e) => {
                tracing::error!("Accept error: {}", e);
                continue;
            }
        };
        let tls_acceptor = tls_acceptor.clone();
        let app = app.clone();

        tokio::spawn(async move {
            let tls_stream = match tls_acceptor.accept(tcp_stream).await {
                Ok(s) => s,
                Err(e) => {
                    tracing::error!("TLS handshake failed for {}: {}", peer_addr, e);
                    return;
                }
            };
            tracing::debug!("TLS handshake complete for {}", peer_addr);

            let io = TokioIo::new(tls_stream);
            let service = TlsService {
                router: app,
                peer_addr,
            };

            let builder = auto::Builder::new(TokioExecutor::new());
            if let Err(e) = builder.serve_connection_with_upgrades(io, service).await {
                tracing::error!("TLS connection error for {}: {}", peer_addr, e);
            }
        });
    }
}
