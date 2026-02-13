use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};

use crate::daemon::state::AppState;
use crate::services::inspect;

// Handler for creating a lab
pub async fn lab_up(Json(payload): Json<LabId>) -> String {
    format!("Creating Lab {}", payload.id)
}

// Handler for destroying a lab
pub async fn lab_destroy(Json(payload): Json<LabId>) -> String {
    format!("Destroying Lab {}", payload.id)
}

// Handler for inspecting a lab
pub async fn lab_inspect(State(state): State<AppState>, Path(lab_id): Path<String>) -> Response {
    // Call the inspect service
    match inspect::inspect_lab(&lab_id, &state).await {
        Ok(response) => {
            // Return JSON response
            (StatusCode::OK, Json(response)).into_response()
        }
        Err(e) => {
            // Return error response
            let error_msg = format!("Failed to inspect lab '{}': {:?}", lab_id, e);
            tracing::error!("{}", error_msg);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse { error: error_msg }),
            )
                .into_response()
        }
    }
}

// Request/Response types
#[derive(Deserialize)]
#[allow(dead_code)]
pub struct CreateUser {
    pub username: String,
}

#[derive(Deserialize)]
pub struct LabId {
    pub id: String,
}

#[derive(Serialize)]
#[allow(dead_code)]
pub struct User {
    pub id: u64,
    pub username: String,
}

#[derive(Serialize)]
pub struct ErrorResponse {
    pub error: String,
}
