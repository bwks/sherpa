//! Health, the server certificate and the API specification.

use axum::Json;
use axum::extract::State;
use axum::http::{StatusCode, header};
use axum::response::IntoResponse;
use serde_json::json;
use std::path::PathBuf;

use crate::daemon::state::AppState;

use crate::api::errors::ApiError;

use shared::api_spec;
use shared::konst::SHERPA_SERVER_CERT_PATH;

/// Health check endpoint
///
/// Returns server status and configuration information.
/// Always publicly accessible (no authentication required).
///
/// # Response (200 OK)
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
            "version": env!("CARGO_PKG_VERSION"),
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
        PathBuf::from(SHERPA_SERVER_CERT_PATH)
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

/// Serve the unified API specification
///
/// GET `/api/v1/spec`
/// Public endpoint - no authentication required
pub async fn api_spec_handler() -> Result<impl IntoResponse, ApiError> {
    static SPEC_JSON: std::sync::OnceLock<Result<serde_json::Value, String>> =
        std::sync::OnceLock::new();
    let result = SPEC_JSON.get_or_init(|| {
        let spec = api_spec::build_spec();
        serde_json::to_value(spec).map_err(|e| e.to_string())
    });
    match result {
        Ok(spec) => Ok(Json(spec.clone())),
        Err(e) => Err(ApiError::internal(format!(
            "Failed to serialize API spec: {e}"
        ))),
    }
}

/// Serve the `OpenAPI` 3.1 specification
///
/// GET `/api/v1/openapi.json`
/// Public endpoint - no authentication required
pub async fn openapi_handler() -> Result<impl IntoResponse, ApiError> {
    static OPENAPI_JSON: std::sync::OnceLock<Result<serde_json::Value, String>> =
        std::sync::OnceLock::new();
    let result = OPENAPI_JSON.get_or_init(|| {
        let doc = api_spec::build_openapi();
        Ok(doc)
    });
    match result {
        Ok(doc) => Ok(Json(doc.clone())),
        Err(e) => Err(ApiError::internal(format!(
            "Failed to build OpenAPI spec: {e}"
        ))),
    }
}

// Request/Response types
