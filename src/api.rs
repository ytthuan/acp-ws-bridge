//! HTTP/REST API endpoints (health checks, session listing, etc.).

use std::sync::Arc;
use std::time::Instant;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::get;
use axum::{Json, Router};
use tower_http::cors::CorsLayer;

use crate::session::{SessionInfo, SessionManager, SessionStats};

/// Shared state for API handlers.
#[derive(Clone)]
struct ApiState {
    session_manager: SessionManager,
    start_time: Arc<Instant>,
}

/// GET /health — Health check
async fn health(State(state): State<ApiState>) -> Json<serde_json::Value> {
    let uptime = state.start_time.elapsed().as_secs();
    Json(serde_json::json!({
        "status": "ok",
        "version": env!("CARGO_PKG_VERSION"),
        "uptime_secs": uptime
    }))
}

/// GET /api/sessions — List all sessions
async fn list_sessions(State(state): State<ApiState>) -> Json<Vec<SessionInfo>> {
    Json(state.session_manager.list_sessions().await)
}

/// GET /api/sessions/:id — Get session details
async fn get_session(
    State(state): State<ApiState>,
    Path(id): Path<String>,
) -> Result<Json<SessionInfo>, StatusCode> {
    state
        .session_manager
        .get_session(&id)
        .await
        .map(Json)
        .ok_or(StatusCode::NOT_FOUND)
}

/// DELETE /api/sessions/:id — Delete a session
async fn delete_session(
    State(state): State<ApiState>,
    Path(id): Path<String>,
) -> StatusCode {
    if state.session_manager.delete_session(&id).await {
        StatusCode::NO_CONTENT
    } else {
        StatusCode::NOT_FOUND
    }
}

/// GET /api/stats — Aggregate statistics
async fn get_stats(State(state): State<ApiState>) -> Json<SessionStats> {
    Json(state.session_manager.get_stats().await)
}

/// Build the axum Router for the REST API.
pub fn api_router(session_manager: SessionManager) -> Router {
    let state = ApiState {
        session_manager,
        start_time: Arc::new(Instant::now()),
    };

    Router::new()
        .route("/health", get(health))
        .route("/api/sessions", get(list_sessions))
        .route(
            "/api/sessions/{id}",
            get(get_session).delete(delete_session),
        )
        .route("/api/stats", get(get_stats))
        .layer(CorsLayer::permissive())
        .with_state(state)
}
