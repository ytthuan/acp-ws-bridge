//! HTTP/REST API endpoints (health checks, session listing, etc.).

use std::path::PathBuf;
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
use crate::stats_cache::StatsCache;

/// Detected Copilot CLI information (populated at startup).
#[derive(Clone, Debug, serde::Serialize)]
pub struct CopilotInfo {
    pub version: Option<String>,
    pub path: String,
    pub mode: String,
}

/// Shared state for API handlers.
#[derive(Clone)]
struct ApiState {
    session_manager: SessionManager,
    start_time: Arc<Instant>,
    stats_cache: Arc<StatsCache>,
    copilot_info: CopilotInfo,
    copilot_dir: PathBuf,
}

/// GET /health — Health check
async fn health(State(state): State<ApiState>) -> Json<serde_json::Value> {
    let uptime = state.start_time.elapsed().as_secs();
    Json(serde_json::json!({
        "status": "ok",
        "version": env!("CARGO_PKG_VERSION"),
        "copilot_cli_version": state.copilot_info.version,
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
async fn delete_session(State(state): State<ApiState>, Path(id): Path<String>) -> StatusCode {
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

// -- History endpoints (read-only, from the configured Copilot data directory) --

/// GET /api/history/sessions — list all historical sessions
async fn list_history_sessions(State(state): State<ApiState>) -> impl IntoResponse {
    match history::list_sessions_from(&state.copilot_dir) {
        Ok(sessions) => Json(sessions).into_response(),
        Err(e) => {
            tracing::error!("Failed to load history sessions: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "history database unavailable",
            )
                .into_response()
        }
    }
}

/// GET /api/history/sessions/:id — get session turns
async fn get_history_session(
    State(state): State<ApiState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    match history::get_session_turns_from(&state.copilot_dir, &id) {
        Ok(turns) => Json(turns).into_response(),
        Err(e) => {
            tracing::error!("Failed to load history session {}: {}", id, e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "history database unavailable",
            )
                .into_response()
        }
    }
}

/// GET /api/history/sessions/:id/turns — get session turns (explicit sub-route)
async fn get_history_session_turns(
    State(state): State<ApiState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    match history::get_session_turns_from(&state.copilot_dir, &id) {
        Ok(turns) => Json(turns).into_response(),
        Err(e) => {
            tracing::error!("Failed to load history session turns {}: {}", id, e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "history database unavailable",
            )
                .into_response()
        }
    }
}

/// GET /api/history/stats — aggregate stats
async fn get_history_stats(State(state): State<ApiState>) -> impl IntoResponse {
    match history::get_history_stats_from(&state.copilot_dir) {
        Ok(stats) => Json(stats).into_response(),
        Err(e) => {
            tracing::error!("Failed to load history stats: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "history database unavailable",
            )
                .into_response()
        }
    }
}

/// GET /api/copilot/usage — aggregate Copilot CLI usage statistics
async fn get_copilot_usage(State(state): State<ApiState>) -> impl IntoResponse {
    let cache = state.stats_cache.clone();
    match tokio::task::spawn_blocking(move || history::get_copilot_usage(&cache)).await {
        Ok(Ok(stats)) => Json(stats).into_response(),
        Ok(Err(e)) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

/// GET /api/copilot/info — Copilot CLI metadata and capabilities
async fn get_copilot_info(State(state): State<ApiState>) -> Json<serde_json::Value> {
    let info = &state.copilot_info;
    let version_str = info.version.as_deref().unwrap_or("unknown");

    // Determine GA status and features based on detected version
    let is_ga = is_copilot_ga(version_str);
    let features = detect_features(version_str);

    Json(serde_json::json!({
        "version": info.version,
        "path": info.path,
        "mode": info.mode,
        "ga": is_ga,
        "features": features
    }))
}

/// Returns true if the detected Copilot CLI version is GA (>= 0.0.418 or >= 1.0.0).
fn is_copilot_ga(version: &str) -> bool {
    // Strip leading 'v' or any prefix text (e.g. "GitHub Copilot CLI v1.0.2")
    let v = version
        .rsplit(' ')
        .next()
        .unwrap_or(version)
        .trim_start_matches('v');

    let parts: Vec<&str> = v.split('.').collect();
    if parts.len() < 3 {
        return false;
    }
    let major = parts[0].parse::<u32>().unwrap_or(0);
    let minor = parts[1].parse::<u32>().unwrap_or(0);
    let patch = parts[2].parse::<u32>().unwrap_or(0);

    if major >= 1 {
        return true;
    }
    // Pre-1.0: GA was v0.0.418
    major == 0 && minor == 0 && patch >= 418
}

/// Detect available features based on CLI version.
fn detect_features(version: &str) -> Vec<&'static str> {
    let v = version
        .rsplit(' ')
        .next()
        .unwrap_or(version)
        .trim_start_matches('v');

    let parts: Vec<&str> = v.split('.').collect();
    let (major, minor, patch) = if parts.len() >= 3 {
        (
            parts[0].parse::<u32>().unwrap_or(0),
            parts[1].parse::<u32>().unwrap_or(0),
            parts[2].parse::<u32>().unwrap_or(0),
        )
    } else {
        return vec![];
    };

    let at_least =
        |maj: u32, min: u32, pat: u32| -> bool { (major, minor, patch) >= (maj, min, pat) };

    let mut features = Vec::new();
    if at_least(0, 0, 418) {
        features.push("ga");
    }
    if at_least(0, 0, 421) {
        features.push("reasoning_effort");
        features.push("mcp_elicitations");
    }
    if at_least(0, 0, 422) {
        features.push("exit_plan_mode");
        features.push("session_metrics");
        features.push("output_format_json");
    }
    if at_least(1, 0, 0) {
        features.push("v1_stable");
    }
    features
}

/// Build the axum Router for the REST API.
pub fn api_router(
    session_manager: SessionManager,
    stats_cache: Arc<StatsCache>,
    copilot_info: CopilotInfo,
    copilot_dir: PathBuf,
) -> Router {
    let state = ApiState {
        session_manager,
        start_time: Arc::new(Instant::now()),
        stats_cache,
        copilot_info,
        copilot_dir,
    };

    Router::new()
        .route("/health", get(health))
        .route("/api/sessions", get(list_sessions))
        .route("/api/sessions/:id", get(get_session).delete(delete_session))
        .route("/api/sessions/:id/commands", get(get_session_commands))
        .route("/api/stats", get(get_stats))
        .route("/api/history/sessions", get(list_history_sessions))
        .route("/api/history/sessions/:id", get(get_history_session))
        .route(
            "/api/history/sessions/:id/turns",
            get(get_history_session_turns),
        )
        .route("/api/history/stats", get(get_history_stats))
        .route("/api/copilot/usage", get(get_copilot_usage))
        .route("/api/copilot/info", get(get_copilot_info))
        .layer(
            CorsLayer::new()
                .allow_origin(tower_http::cors::Any)
                .allow_methods([
                    axum::http::Method::GET,
                    axum::http::Method::DELETE,
                    axum::http::Method::OPTIONS,
                ])
                .allow_headers(tower_http::cors::Any),
        )
        .with_state(state)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    fn test_copilot_info() -> CopilotInfo {
        CopilotInfo {
            version: Some("1.0.2".to_string()),
            path: "copilot".to_string(),
            mode: "stdio".to_string(),
        }
    }

    fn test_app() -> Router {
        api_router(
            SessionManager::new(),
            Arc::new(StatsCache::new()),
            test_copilot_info(),
            PathBuf::from("/tmp/copilot-data"),
        )
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
        assert_eq!(json["copilot_cli_version"], "1.0.2");
        assert!(json["uptime_secs"].is_number());
    }

    #[tokio::test]
    async fn test_copilot_info_endpoint() {
        let app = test_app();
        let req = Request::builder()
            .uri("/api/copilot/info")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let json = body_json(resp.into_body()).await;
        assert_eq!(json["version"], "1.0.2");
        assert_eq!(json["path"], "copilot");
        assert_eq!(json["mode"], "stdio");
        assert_eq!(json["ga"], true);
        let features = json["features"].as_array().unwrap();
        assert!(features.iter().any(|f| f == "v1_stable"));
        assert!(features.iter().any(|f| f == "reasoning_effort"));
    }

    #[test]
    fn test_is_copilot_ga() {
        assert!(is_copilot_ga("1.0.2"));
        assert!(is_copilot_ga("0.0.418"));
        assert!(is_copilot_ga("0.0.422"));
        assert!(!is_copilot_ga("0.0.417"));
        assert!(!is_copilot_ga("0.0.100"));
        assert!(is_copilot_ga("GitHub Copilot CLI v1.0.2"));
    }

    #[test]
    fn test_detect_features() {
        let f = detect_features("1.0.2");
        assert!(f.contains(&"ga"));
        assert!(f.contains(&"reasoning_effort"));
        assert!(f.contains(&"exit_plan_mode"));
        assert!(f.contains(&"v1_stable"));

        let f2 = detect_features("0.0.420");
        assert!(f2.contains(&"ga"));
        assert!(!f2.contains(&"reasoning_effort"));
        assert!(!f2.contains(&"v1_stable"));
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

        let app = api_router(
            sm,
            std::sync::Arc::new(crate::stats_cache::StatsCache::new()),
            test_copilot_info(),
            PathBuf::from("/tmp/copilot-data"),
        );
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

        let app = api_router(
            sm,
            std::sync::Arc::new(crate::stats_cache::StatsCache::new()),
            test_copilot_info(),
            PathBuf::from("/tmp/copilot-data"),
        );
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

        let app = api_router(
            sm,
            std::sync::Arc::new(crate::stats_cache::StatsCache::new()),
            test_copilot_info(),
            PathBuf::from("/tmp/copilot-data"),
        );
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
        sm.update_status(&s1.id, crate::session::SessionStatus::Active)
            .await;
        sm.update_status(&s2.id, crate::session::SessionStatus::Idle)
            .await;
        sm.increment_prompts(&s1.id).await;
        sm.increment_messages(&s1.id).await;
        sm.increment_messages(&s1.id).await;

        let app = api_router(
            sm,
            std::sync::Arc::new(crate::stats_cache::StatsCache::new()),
            test_copilot_info(),
            PathBuf::from("/tmp/copilot-data"),
        );
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
        let app = api_router(
            sm,
            std::sync::Arc::new(crate::stats_cache::StatsCache::new()),
            test_copilot_info(),
            PathBuf::from("/tmp/copilot-data"),
        );
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

        let app = api_router(
            sm,
            std::sync::Arc::new(crate::stats_cache::StatsCache::new()),
            test_copilot_info(),
            PathBuf::from("/tmp/copilot-data"),
        );
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
