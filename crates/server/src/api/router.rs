use crate::daemon::state::AppState;
use axum::{
    Router,
    routing::{delete, get, post},
};
use tower_http::cors::{AllowOrigin, CorsLayer};
use tower_http::services::ServeDir;

use super::handlers::{
    add_ssh_key_handler, admin_add_ssh_key_handler, admin_dashboard_handler,
    admin_delete_ssh_key_handler, admin_delete_user_handler, admin_node_image_detail_handler,
    admin_node_image_edit_page_handler, admin_node_image_update_handler,
    admin_node_image_versions_handler, admin_node_images_list_handler,
    admin_update_user_password_handler, admin_user_edit_handler, dashboard_handler,
    delete_ssh_key_handler, get_certificate_handler, get_lab, get_labs_html, get_labs_json,
    health_check, lab_destroy, lab_detail_handler, lab_up, login, login_form_handler,
    login_page_handler, logout_handler, profile_handler, signup_form_handler, signup_page_handler,
    update_password_handler,
};

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
        // Public HTML routes (authentication pages)
        .route("/login", get(login_page_handler))
        .route("/login", post(login_form_handler))
        .route("/signup", get(signup_page_handler))
        .route("/signup", post(signup_form_handler))
        .route("/logout", post(logout_handler))
        // Protected HTML routes (require cookie authentication)
        .route("/", get(dashboard_handler))
        .route("/labs", get(get_labs_html))
        .route("/labs/{lab_id}", get(lab_detail_handler))
        .route("/profile", get(profile_handler))
        .route("/profile/password", post(update_password_handler))
        .route("/profile/ssh-keys", post(add_ssh_key_handler))
        .route("/profile/ssh-keys/{index}", delete(delete_ssh_key_handler))
        // Admin routes (require admin privileges)
        .route("/admin", get(admin_dashboard_handler))
        .route("/admin/users/{username}", get(admin_user_edit_handler))
        .route("/admin/users/{username}", delete(admin_delete_user_handler))
        .route(
            "/admin/users/{username}/password",
            post(admin_update_user_password_handler),
        )
        .route(
            "/admin/users/{username}/ssh-keys",
            post(admin_add_ssh_key_handler),
        )
        .route(
            "/admin/users/{username}/ssh-keys/{index}",
            delete(admin_delete_ssh_key_handler),
        )
        .route("/admin/node-images", get(admin_node_images_list_handler))
        // Versions list route (most specific, must come first)
        .route(
            "/admin/node-images/{model}/versions",
            get(admin_node_image_versions_handler),
        )
        // Version-specific routes (must come before non-version routes for proper matching)
        .route(
            "/admin/node-images/{model}/{version}",
            get(admin_node_image_detail_handler),
        )
        .route(
            "/admin/node-images/{model}/{version}/edit",
            get(admin_node_image_edit_page_handler).post(admin_node_image_update_handler),
        )
        // Non-version routes (fallback to default version)
        .route(
            "/admin/node-images/{model}",
            get(admin_node_image_detail_handler),
        )
        .route(
            "/admin/node-images/{model}/edit",
            get(admin_node_image_edit_page_handler).post(admin_node_image_update_handler),
        )
        // Public API endpoints (no authentication required)
        .route("/health", get(health_check))
        .route("/cert", get(get_certificate_handler))
        .route("/api/v1/auth/login", post(login)) // JSON login for CLI
        .route("/api/v1/labs", get(get_labs_json))
        // Protected API endpoints (authentication required)
        .route("/api/v1/labs/{id}", get(get_lab))
        // Stub routes (future implementation)
        .route("/up", post(lab_up))
        .route("/destroy", post(lab_destroy))
        // Apply CORS middleware to all routes
        .layer(cors)
        // Serve static files as fallback (catches all unmatched routes)
        .fallback_service(
            ServeDir::new("crates/server/web/static").append_index_html_on_directories(true),
        )
}
