use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::{StatusCode, header};
use axum::response::IntoResponse;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::HashMap;
use std::path::PathBuf;

use crate::auth::jwt;
use crate::daemon::state::AppState;
use crate::services::{inspect, list_labs};
use crate::templates::{DashboardTemplate, EmptyStateTemplate, ErrorTemplate, LabsGridTemplate};

use super::errors::ApiError;
use super::extractors::AuthenticatedUser;

use shared::auth::password;
use shared::data::{
    InspectRequest, InspectResponse, ListLabsResponse, LoginRequest, LoginResponse,
};
use shared::konst::{
    JWT_TOKEN_EXPIRY_SECONDS, SHERPA_BASE_DIR, SHERPA_CERTS_DIR, SHERPA_SERVER_CERT_FILE,
};

/// Authenticate user and issue JWT token
///
/// # Request Body
/// ```json
/// {
///   "username": "alice",
///   "password": "SecurePass123!"
/// }
/// ```
///
/// # Response (200 OK)
/// ```json
/// {
///   "token": "eyJhbGc...",
///   "username": "alice",
///   "is_admin": false,
///   "expires_at": 1234567890
/// }
/// ```
///
/// # Errors
/// - `401 Unauthorized` - Invalid username or password
/// - `500 Internal Server Error` - Server error during authentication
pub async fn login(
    State(state): State<AppState>,
    Json(request): Json<LoginRequest>,
) -> Result<Json<LoginResponse>, ApiError> {
    // Get user from database (includes password_hash)
    let user = db::get_user_for_auth(&state.db, &request.username)
        .await
        .map_err(|_| {
            // Don't reveal whether user exists
            tracing::debug!("Login attempt for non-existent user: {}", request.username);
            ApiError::unauthorized("Invalid username or password")
        })?;

    // Verify password against stored hash
    let is_valid =
        password::verify_password(&request.password, &user.password_hash).map_err(|e| {
            tracing::error!("Password verification error: {:?}", e);
            ApiError::internal("Authentication error")
        })?;

    if !is_valid {
        tracing::debug!("Invalid password for user: {}", request.username);
        return Err(ApiError::unauthorized("Invalid username or password"));
    }

    // Create JWT token
    let token = jwt::create_token(
        &state.jwt_secret,
        &user.username,
        user.is_admin,
        JWT_TOKEN_EXPIRY_SECONDS,
    )
    .map_err(|e| {
        tracing::error!("Failed to create JWT token: {:?}", e);
        ApiError::internal("Failed to create authentication token")
    })?;

    let now = jiff::Timestamp::now().as_second();
    let expires_at = now + JWT_TOKEN_EXPIRY_SECONDS;

    tracing::info!("User '{}' logged in successfully", user.username);

    Ok(Json(LoginResponse {
        token,
        username: user.username,
        is_admin: user.is_admin,
        expires_at,
    }))
}

/// Get detailed information about a lab
///
/// Returns the current state of a lab including:
/// - Lab metadata (name, topology, description)
/// - Active devices with status, management IPs, and disk information
/// - Inactive devices (nodes that should exist but aren't running)
///
/// # Authentication
/// Requires valid JWT token in `Authorization: Bearer <token>` header.
///
/// # Authorization
/// Users can only inspect their own labs. Admins can inspect any lab.
///
/// # Path Parameters
/// - `id` - The lab ID to inspect
///
/// # Response (200 OK)
/// Returns `InspectResponse` with lab details.
///
/// # Errors
/// - `401 Unauthorized` - Missing or invalid token
/// - `403 Forbidden` - User doesn't own this lab
/// - `404 Not Found` - Lab doesn't exist
/// - `500 Internal Server Error` - Server-side error
///
/// # Example
/// ```bash
/// curl -H "Authorization: Bearer <token>" \
///      https://server:3030/api/v1/labs/my-lab
/// ```
pub async fn get_lab(
    State(state): State<AppState>,
    Path(lab_id): Path<String>,
    auth: AuthenticatedUser,
) -> Result<Json<InspectResponse>, ApiError> {
    tracing::debug!(
        "User '{}' requesting inspect for lab '{}'",
        auth.username,
        lab_id
    );

    let request = InspectRequest {
        lab_id: lab_id.clone(),
        username: auth.username.clone(),
    };

    // Call service layer
    let response = inspect::inspect_lab(request, &state).await.map_err(|e| {
        tracing::error!(
            "Failed to inspect lab '{}' for user '{}': {:?}",
            lab_id,
            auth.username,
            e
        );
        ApiError::from(e)
    })?;

    tracing::info!(
        "Successfully inspected lab '{}' for user '{}'",
        lab_id,
        auth.username
    );

    Ok(Json(response))
}

/// Health check endpoint
///
/// Returns server status and configuration information.
/// Always publicly accessible (no authentication required).
///
/// # Response (200 OK)
/// ```json
/// {
///   "status": "ok",
///   "service": "sherpad",
///   "tls": "enabled"
/// }
/// ```
pub async fn health_check(State(state): State<AppState>) -> impl IntoResponse {
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

/// Certificate download endpoint
///
/// Returns the server's TLS certificate in PEM format.
/// Always available over HTTP to allow clients to fetch certificate before trusting.
/// Publicly accessible (no authentication required).
///
/// # Response
/// - `200 OK` - Returns certificate file (application/x-pem-file)
/// - `404 Not Found` - Certificate file doesn't exist
/// - `500 Internal Server Error` - Failed to read certificate
/// - `503 Service Unavailable` - TLS is disabled on server
pub async fn get_certificate_handler(State(state): State<AppState>) -> impl IntoResponse {
    // Check if TLS is enabled
    if !state.config.tls.enabled {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            [(header::CONTENT_TYPE, "application/json")],
            Json(json!({
                "error": "TLS is disabled on this server",
                "message": "The server is not using TLS, no certificate is available"
            }))
            .to_string(),
        )
            .into_response();
    }

    // Determine certificate path
    let cert_path = if let Some(ref path) = state.config.tls.cert_path {
        PathBuf::from(path)
    } else {
        PathBuf::from(format!(
            "{}/{}/{}",
            SHERPA_BASE_DIR, SHERPA_CERTS_DIR, SHERPA_SERVER_CERT_FILE
        ))
    };

    // Check if certificate exists
    if !cert_path.exists() {
        tracing::error!("Certificate file not found at: {}", cert_path.display());
        return (
            StatusCode::NOT_FOUND,
            [(header::CONTENT_TYPE, "application/json")],
            Json(json!({
                "error": "Certificate not found",
                "message": "Server certificate file does not exist. This should not happen."
            }))
            .to_string(),
        )
            .into_response();
    }

    // Read certificate file
    match tokio::fs::read_to_string(&cert_path).await {
        Ok(cert_pem) => {
            tracing::info!("Serving certificate from: {}", cert_path.display());
            (
                StatusCode::OK,
                [
                    (header::CONTENT_TYPE, "application/x-pem-file"),
                    (
                        header::CONTENT_DISPOSITION,
                        "inline; filename=\"server.crt\"",
                    ),
                ],
                cert_pem,
            )
                .into_response()
        }
        Err(e) => {
            tracing::error!("Failed to read certificate file: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                [(header::CONTENT_TYPE, "application/json")],
                Json(json!({
                    "error": "Failed to read certificate",
                    "message": "An error occurred while reading the certificate file"
                }))
                .to_string(),
            )
                .into_response()
        }
    }
}

/// List all labs for a specific user (JSON API)
///
/// Query all labs owned by the specified user.
/// Returns JSON response for programmatic API access.
/// Publicly accessible (no authentication) for initial development.
///
/// # Query Parameters
/// - `username` (required) - The username to list labs for
///
/// # Response (200 OK)
/// Returns `ListLabsResponse` with lab summaries
///
/// # Errors
/// - `404 Not Found` - User doesn't exist
/// - `500 Internal Server Error` - Database error
///
/// # Example
/// ```bash
/// curl https://server:3030/api/v1/labs?username=bradmin
/// ```
pub async fn get_labs_json(
    State(state): State<AppState>,
    Query(params): Query<ListLabsQuery>,
) -> Result<Json<ListLabsResponse>, ApiError> {
    let username = params.username.trim();

    if username.is_empty() {
        return Err(ApiError::bad_request("Username parameter is required"));
    }

    tracing::debug!("Listing labs for user '{}'", username);

    // Call service layer
    let response = list_labs::list_labs(username, &state).await.map_err(|e| {
        tracing::error!("Failed to list labs for user '{}': {:?}", username, e);
        // Check if it's a user not found error
        if e.to_string().contains("User not found") {
            ApiError::not_found("User", format!("User '{}' not found", username))
        } else {
            ApiError::from(e)
        }
    })?;

    tracing::info!(
        "Successfully listed {} labs for user '{}'",
        response.total,
        username
    );

    Ok(Json(response))
}

/// Dashboard page handler (HTML)
///
/// Serves the main dashboard HTML page with HTMX support.
/// The page will use HTMX to dynamically load labs.
///
/// # Query Parameters
/// - `username` (optional) - The username to display, defaults to "bradmin"
///
/// # Response (200 OK)
/// Returns HTML page rendered from dashboard template
///
/// # Example
/// ```bash
/// curl https://server:3030/?username=bradmin
/// ```
pub async fn dashboard_handler(
    Query(params): Query<HashMap<String, String>>,
) -> Result<DashboardTemplate, ApiError> {
    let username = params
        .get("username")
        .map(|s| s.to_string())
        .unwrap_or_else(|| "bradmin".to_string());

    tracing::debug!("Serving dashboard for user '{}'", username);

    Ok(DashboardTemplate { username })
}

/// Labs grid HTML fragment handler
///
/// Returns an HTML fragment containing the labs grid for HTMX swapping.
/// This endpoint is called by HTMX on page load and periodically for auto-refresh.
///
/// # Query Parameters
/// - `username` (optional) - The username to list labs for, defaults to "bradmin"
///
/// # Response (200 OK)
/// Returns HTML fragment with labs grid, empty state, or error message
///
/// # Example
/// ```bash
/// curl https://server:3030/labs?username=bradmin
/// ```
pub async fn get_labs_html(
    State(state): State<AppState>,
    Query(params): Query<HashMap<String, String>>,
) -> impl IntoResponse {
    let username = params
        .get("username")
        .map(|s| s.to_string())
        .unwrap_or_else(|| "bradmin".to_string());

    tracing::debug!("Fetching labs HTML for user '{}'", username);

    // Call service layer
    match list_labs::list_labs(&username, &state).await {
        Ok(response) => {
            if response.labs.is_empty() {
                tracing::debug!("No labs found for user '{}'", username);
                EmptyStateTemplate { username }.into_response()
            } else {
                tracing::debug!(
                    "Returning {} labs for user '{}'",
                    response.labs.len(),
                    username
                );
                LabsGridTemplate {
                    labs: response.labs,
                }
                .into_response()
            }
        }
        Err(e) => {
            tracing::error!("Failed to load labs for user '{}': {:?}", username, e);
            let message = if e.to_string().contains("User not found") {
                format!("User '{}' not found", username)
            } else {
                format!("Failed to load labs: {}", e)
            };
            ErrorTemplate { message }.into_response()
        }
    }
}

// Stub handlers for future implementation

/// Handler for creating a lab (stub)
pub async fn lab_up(Json(payload): Json<LabId>) -> String {
    format!("Creating Lab {}", payload.id)
}

/// Handler for destroying a lab (stub)
pub async fn lab_destroy(Json(payload): Json<LabId>) -> String {
    format!("Destroying Lab {}", payload.id)
}

// Request/Response types

/// Query parameters for listing labs
#[derive(Deserialize)]
pub struct ListLabsQuery {
    pub username: String,
}

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
