pub mod handlers;
pub mod websocket;

use crate::daemon::state::AppState;
use axum::{extract::State, http::StatusCode, response::IntoResponse, routing::{get, post}, Json, Router};
use handlers::{lab_destroy, lab_inspect, lab_up};
use serde_json::json;

/// Build the Axum router with all API routes
pub fn build_router() -> Router<AppState> {
    Router::new()
        .route("/health", get(health_check))
        .route("/up", post(lab_up))
        .route("/destroy", post(lab_destroy))
        .route("/inspect/{id}", get(lab_inspect))
}

/// Health check endpoint
async fn health_check(State(state): State<AppState>) -> impl IntoResponse {
    let tls_status = if state.config.tls.enabled {
        "enabled"
    } else {
        "disabled"
    };

    (
        StatusCode::OK,
        Json(json!({
            "status": "ok",
            "service": "sherpad",
            "tls": tls_status,
        })),
    )
}
