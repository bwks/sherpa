use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post};
use axum::{Router, extract::Path};
use rust_embed::Embed;
use tower_http::cors::{AllowOrigin, CorsLayer};

use crate::daemon::state::AppState;

#[derive(Embed)]
#[folder = "web/static"]
struct StaticAssets;

async fn embedded_asset_handler(Path(path): Path<String>) -> Response {
    match StaticAssets::get(&path) {
        Some(file) => {
            let mime = match path.rsplit_once('.').map(|(_, ext)| ext) {
                Some("css") => "text/css",
                Some("js") => "application/javascript",
                Some("html") => "text/html",
                Some("svg") => "image/svg+xml",
                Some("png") => "image/png",
                Some("jpg" | "jpeg") => "image/jpeg",
                Some("ico") => "image/x-icon",
                Some("woff2") => "font/woff2",
                Some("woff") => "font/woff",
                Some("ttf") => "font/ttf",
                Some("json") => "application/json",
                _ => "application/octet-stream",
            };
            (StatusCode::OK, [(header::CONTENT_TYPE, mime)], file.data).into_response()
        }
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

use super::handlers::{
    add_ssh_key_handler, admin_add_ssh_key_handler, admin_dashboard_handler,
    admin_delete_ssh_key_handler, admin_delete_user_handler, admin_labs_list_handler,
    admin_node_image_detail_handler, admin_node_image_edit_page_handler,
    admin_node_image_update_handler, admin_node_image_versions_handler,
    admin_node_images_list_handler, admin_update_user_password_handler, admin_user_edit_handler,
    api_spec_handler, create_lab_json, dashboard_handler, delete_lab_json, delete_ssh_key_handler,
    down_lab_json, get_certificate_handler, get_lab, get_labs_html, get_labs_json, health_check,
    lab_destroy_button_handler, lab_destroy_confirm_handler, lab_destroy_post_handler,
    lab_destroy_stream_handler, lab_detail_handler, lab_nodes_handler, login, login_form_handler,
    login_page_handler, logout_handler, profile_handler, redeploy_node_json, resume_lab_json,
    signup_form_handler, signup_page_handler, update_password_handler,
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
        .route("/labs/{lab_id}/nodes", get(lab_nodes_handler))
        .route(
            "/labs/{lab_id}/destroy/confirm",
            get(lab_destroy_confirm_handler),
        )
        .route(
            "/labs/{lab_id}/destroy/button",
            get(lab_destroy_button_handler),
        )
        .route(
            "/labs/{lab_id}/destroy/stream",
            get(lab_destroy_stream_handler),
        )
        .route("/labs/{lab_id}/destroy", post(lab_destroy_post_handler))
        .route("/profile", get(profile_handler))
        .route("/profile/password", post(update_password_handler))
        .route("/profile/ssh-keys", post(add_ssh_key_handler))
        .route("/profile/ssh-keys/{index}", delete(delete_ssh_key_handler))
        // Admin routes (require admin privileges)
        .route("/admin", get(admin_dashboard_handler))
        .route("/admin/labs", get(admin_labs_list_handler))
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
        .route("/api/v1/spec", get(api_spec_handler))
        .route("/api/v1/auth/login", post(login)) // JSON login for CLI
        .route("/api/v1/labs", get(get_labs_json))
        // Protected API endpoints (authentication required)
        .route("/api/v1/labs", post(create_lab_json))
        .route("/api/v1/labs/{id}", get(get_lab))
        .route("/api/v1/labs/{id}", delete(delete_lab_json))
        .route("/api/v1/labs/{id}/down", post(down_lab_json))
        .route("/api/v1/labs/{id}/resume", post(resume_lab_json))
        .route(
            "/api/v1/labs/{id}/nodes/{node_name}/redeploy",
            post(redeploy_node_json),
        )
        // Apply CORS middleware to all routes
        .layer(cors)
        // Serve embedded static files
        .route("/{*path}", get(embedded_asset_handler))
}
