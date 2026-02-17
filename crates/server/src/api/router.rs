use crate::daemon::state::AppState;
use axum::{
    Router,
    routing::{get, post},
};
use tower_http::cors::{AllowOrigin, CorsLayer};

use super::handlers::{get_certificate_handler, get_lab, health_check, lab_destroy, lab_up, login};

/// Build the Axum router with all API routes
pub fn build_router() -> Router<AppState> {
    use axum::http::Method;

    // Configure CORS
    // Note: When allow_credentials is true, we cannot use Any - we mirror the request origin instead
    // For production, replace mirror_request() with explicit allowed origins
    let cors = CorsLayer::new()
        .allow_origin(AllowOrigin::mirror_request()) // Mirror requesting origin (permissive)
        .allow_methods(vec![
            Method::GET,
            Method::POST,
            Method::PUT,
            Method::DELETE,
            Method::OPTIONS,
        ])
        .allow_headers(vec![
            axum::http::header::AUTHORIZATION,
            axum::http::header::CONTENT_TYPE,
            axum::http::header::ACCEPT,
        ])
        .allow_credentials(true); // Allow credentials (cookies, auth headers)

    Router::new()
        // Public endpoints (no authentication required)
        .route("/health", get(health_check))
        .route("/cert", get(get_certificate_handler))
        .route("/api/v1/auth/login", post(login))
        // Protected API endpoints (authentication required)
        .route("/api/v1/labs/{id}", get(get_lab))
        // Stub routes (future implementation)
        .route("/up", post(lab_up))
        .route("/destroy", post(lab_destroy))
        // Apply CORS middleware to all routes
        .layer(cors)
}
