use axum::extract::Path;
use axum::Json;
use serde::{Deserialize, Serialize};

// Handler for creating a lab
pub async fn lab_up(Json(payload): Json<LabId>) -> String {
    format!("Creating Lab {}", payload.id)
}

// Handler for destroying a lab
pub async fn lab_destroy(Json(payload): Json<LabId>) -> String {
    format!("Destroying Lab {}", payload.id)
}

// Handler for inspecting a lab
pub async fn lab_inspect(id: Path<String>) -> String {
    format!("Inspecting Lab {}", id.0)
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
