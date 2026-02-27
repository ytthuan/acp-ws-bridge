//! HTTP/REST API endpoints (health checks, session listing, etc.).

use std::sync::Arc;
use std::time::Instant;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::get;
use axum::{Json, Router};
use tower_http::cors::CorsLayer;

use axum::response::IntoResponse;

use crate::history;
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

/// GET /api/sessions/:id/commands — Get cached available_commands for a session
async fn get_session_commands(
    State(state): State<ApiState>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    // Verify session exists
    if state.session_manager.get_session(&id).await.is_none() {
        return Err(StatusCode::NOT_FOUND);
    }
    let commands = state
        .session_manager
        .get_available_commands(&id)
        .await
        .unwrap_or(serde_json::json!([]));
    Ok(Json(commands))
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

// -- History endpoints (read-only, from ~/.copilot/session-store.db) --

/// GET /api/history/sessions — list all historical sessions
async fn list_history_sessions() -> impl IntoResponse {
    match history::list_sessions() {
        Ok(sessions) => Json(sessions).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

/// GET /api/history/sessions/:id — get session turns
async fn get_history_session(Path(id): Path<String>) -> impl IntoResponse {
    match history::get_session_turns(&id) {
        Ok(turns) => Json(turns).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

/// GET /api/history/sessions/:id/turns — get session turns (explicit sub-route)
async fn get_history_session_turns(Path(id): Path<String>) -> impl IntoResponse {
    match history::get_session_turns(&id) {
        Ok(turns) => Json(turns).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

/// GET /api/history/stats — aggregate stats
async fn get_history_stats() -> impl IntoResponse {
    match history::get_history_stats() {
        Ok(stats) => Json(stats).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
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
            "/api/sessions/:id",
            get(get_session).delete(delete_session),
        )
        .route("/api/sessions/:id/commands", get(get_session_commands))
        .route("/api/stats", get(get_stats))
        .route("/api/history/sessions", get(list_history_sessions))
        .route("/api/history/sessions/:id", get(get_history_session))
        .route("/api/history/sessions/:id/turns", get(get_history_session_turns))
        .route("/api/history/stats", get(get_history_stats))
        .layer(CorsLayer::permissive())
        .with_state(state)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    fn test_app() -> Router {
        api_router(SessionManager::new())
    }

    async fn body_json(body: Body) -> serde_json::Value {
        let bytes = body.collect().await.unwrap().to_bytes();
        serde_json::from_slice(&bytes).unwrap()
    }

    #[tokio::test]
    async fn test_health_endpoint() {
        let app = test_app();
        let req = Request::builder()
            .uri("/health")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let json = body_json(resp.into_body()).await;
        assert_eq!(json["status"], "ok");
        assert!(json["version"].is_string());
        assert!(json["uptime_secs"].is_number());
    }

    #[tokio::test]
    async fn test_list_sessions_empty() {
        let app = test_app();
        let req = Request::builder()
            .uri("/api/sessions")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let json = body_json(resp.into_body()).await;
        assert_eq!(json, serde_json::json!([]));
    }

    #[tokio::test]
    async fn test_get_session_not_found() {
        let app = test_app();
        let req = Request::builder()
            .uri("/api/sessions/nonexistent")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_delete_session_not_found() {
        let app = test_app();
        let req = Request::builder()
            .method("DELETE")
            .uri("/api/sessions/nonexistent")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_get_stats_empty() {
        let app = test_app();
        let req = Request::builder()
            .uri("/api/stats")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let json = body_json(resp.into_body()).await;
        assert_eq!(json["total_sessions"], 0);
        assert_eq!(json["active_sessions"], 0);
        assert_eq!(json["idle_sessions"], 0);
        assert_eq!(json["total_prompts"], 0);
        assert_eq!(json["total_messages"], 0);
    }

    #[tokio::test]
    async fn test_list_sessions_with_data() {
        let sm = SessionManager::new();
        sm.create_session().await;
        sm.create_session().await;

        let app = api_router(sm);
        let req = Request::builder()
            .uri("/api/sessions")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let json = body_json(resp.into_body()).await;
        assert_eq!(json.as_array().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn test_get_session_exists() {
        let sm = SessionManager::new();
        let info = sm.create_session().await;

        let app = api_router(sm);
        let req = Request::builder()
            .uri(format!("/api/sessions/{}", info.id))
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let json = body_json(resp.into_body()).await;
        assert_eq!(json["id"], info.id);
    }

    #[tokio::test]
    async fn test_delete_session_exists() {
        let sm = SessionManager::new();
        let info = sm.create_session().await;

        let app = api_router(sm);
        let req = Request::builder()
            .method("DELETE")
            .uri(format!("/api/sessions/{}", info.id))
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NO_CONTENT);
    }

    #[tokio::test]
    async fn test_stats_with_sessions() {
        let sm = SessionManager::new();
        let s1 = sm.create_session().await;
        let s2 = sm.create_session().await;
        sm.update_status(&s1.id, crate::session::SessionStatus::Active).await;
        sm.update_status(&s2.id, crate::session::SessionStatus::Idle).await;
        sm.increment_prompts(&s1.id).await;
        sm.increment_messages(&s1.id).await;
        sm.increment_messages(&s1.id).await;

        let app = api_router(sm);
        let req = Request::builder()
            .uri("/api/stats")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let json = body_json(resp.into_body()).await;
        assert_eq!(json["total_sessions"], 2);
        assert_eq!(json["active_sessions"], 1);
        assert_eq!(json["idle_sessions"], 1);
        assert_eq!(json["total_prompts"], 1);
        assert_eq!(json["total_messages"], 2);
    }

    #[tokio::test]
    async fn test_get_session_commands_not_found() {
        let app = test_app();
        let req = Request::builder()
            .uri("/api/sessions/nonexistent/commands")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_get_session_commands_empty() {
        let sm = SessionManager::new();
        let info = sm.create_session().await;
        let app = api_router(sm);
        let req = Request::builder()
            .uri(format!("/api/sessions/{}/commands", info.id))
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let json = body_json(resp.into_body()).await;
        assert_eq!(json, serde_json::json!([]));
    }

    #[tokio::test]
    async fn test_get_session_commands_with_data() {
        let sm = SessionManager::new();
        let info = sm.create_session().await;
        let cmds = serde_json::json!([{"name": "explain"}, {"name": "fix"}]);
        sm.set_available_commands(&info.id, cmds.clone()).await;

        let app = api_router(sm);
        let req = Request::builder()
            .uri(format!("/api/sessions/{}/commands", info.id))
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let json = body_json(resp.into_body()).await;
        assert_eq!(json, cmds);
    }
}
