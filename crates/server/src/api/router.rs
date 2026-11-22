use axum::Router;
use axum::extract::Path;
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post};
use rust_embed::Embed;
use tower_http::trace::TraceLayer;

use crate::daemon::state::AppState;

use super::handlers::{
    add_ssh_key_handler, add_ssh_key_json, admin_dashboard_handler, admin_delete_ssh_key_handler,
    admin_delete_user_handler, admin_image_detail_handler, admin_image_edit_page_handler,
    admin_image_update_handler, admin_image_upload_handler, admin_image_upload_page_handler,
    admin_image_versions_handler, admin_images_list_handler, admin_labs_list_handler,
    admin_tools_clean_handler, admin_tools_handler, admin_tools_scan_handler,
    admin_update_user_password_handler, admin_user_edit_handler, api_spec_handler, cancel_lab_json,
    change_password_json, clean_lab_json, create_lab_json, create_user_json, dashboard_handler,
    delete_image_json, delete_lab_json, delete_ssh_key_handler, delete_user_json, down_lab_json,
    download_image_json, get_certificate_handler, get_lab, get_labs_html, get_labs_json,
    get_user_info_json, health_check, import_image_json, job_page_handler, job_stream_handler,
    lab_create_page_handler, lab_create_post_handler, lab_destroy_button_handler,
    lab_destroy_confirm_handler, lab_destroy_post_handler, lab_detail_handler,
    lab_download_handler, lab_nodes_handler, lab_start_handler, lab_stop_handler,
    labs_list_page_handler, list_images_json, list_users_json, login, login_form_handler,
    login_page_handler, logout_handler, logout_json, node_console_handler, node_detail_handler,
    node_redeploy_handler, node_start_handler, node_stop_handler, node_vnc_handler,
    openapi_handler, profile_handler, pull_image_json, redeploy_node_json, remove_ssh_key_json,
    resume_lab_json, scan_images_json, set_default_image_json, set_device_password_json,
    show_image_json, signup_form_handler, signup_page_handler, update_device_password_handler,
    update_impairment_json, update_password_handler, upload_image_multipart, validate_token_json,
};

#[derive(Embed)]
#[folder = "web/static"]
struct StaticAssets;

async fn api_docs_handler() -> Response {
    match StaticAssets::get("swagger/index.html") {
        Some(file) => (
            StatusCode::OK,
            [(header::CONTENT_TYPE, "text/html")],
            file.data,
        )
            .into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

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

/// Build the Axum router with all API routes.
///
/// It sends no CORS headers, so browsers refuse cross-origin calls. The web UI calls only its own
/// origin and the CLI is not a browser, so neither needs a grant. A layer that mirrored the
/// request origin with credentials would let any web page call the API with a signed-in user's
/// session cookie, which the REST routes accept.
pub fn build_router() -> Router<AppState> {
    Router::new()
        .merge(auth_page_routes())
        .merge(user_page_routes())
        .merge(admin_page_routes())
        .merge(public_api_routes())
        .merge(api_routes())
        // Apply middleware layers (outermost = first to process request)
        .layer(TraceLayer::new_for_http())
        // Serve embedded static files
        .route("/{*path}", get(embedded_asset_handler))
}

/// Public HTML routes: the authentication pages.
fn auth_page_routes() -> Router<AppState> {
    Router::new()
        .route("/login", get(login_page_handler))
        .route("/login", post(login_form_handler))
        .route("/signup", get(signup_page_handler))
        .route("/signup", post(signup_form_handler))
        .route("/logout", post(logout_handler))
}

/// Protected HTML routes, which require cookie authentication.
fn user_page_routes() -> Router<AppState> {
    Router::new()
        .route("/", get(dashboard_handler))
        .route("/labs", get(labs_list_page_handler))
        .route("/labs/grid", get(get_labs_html))
        .route(
            "/labs/create",
            get(lab_create_page_handler).post(lab_create_post_handler),
        )
        .route("/jobs/{job_id}", get(job_page_handler))
        .route("/jobs/{job_id}/stream", get(job_stream_handler))
        .route("/labs/{lab_id}", get(lab_detail_handler))
        .route("/labs/{lab_id}/nodes", get(lab_nodes_handler))
        .route("/labs/{lab_id}/nodes/{node_name}", get(node_detail_handler))
        .route(
            "/labs/{lab_id}/destroy/confirm",
            get(lab_destroy_confirm_handler),
        )
        .route(
            "/labs/{lab_id}/destroy/button",
            get(lab_destroy_button_handler),
        )
        .route("/labs/{lab_id}/destroy", post(lab_destroy_post_handler))
        .route("/labs/{lab_id}/download", get(lab_download_handler))
        .route("/labs/{lab_id}/stop", post(lab_stop_handler))
        .route("/labs/{lab_id}/start", post(lab_start_handler))
        .route(
            "/labs/{lab_id}/nodes/{node_name}/stop",
            post(node_stop_handler),
        )
        .route(
            "/labs/{lab_id}/nodes/{node_name}/start",
            post(node_start_handler),
        )
        .route(
            "/labs/{lab_id}/nodes/{node_name}/redeploy",
            post(node_redeploy_handler),
        )
        .route("/profile", get(profile_handler))
        .route("/profile/password", post(update_password_handler))
        .route(
            "/profile/device-password",
            post(update_device_password_handler),
        )
        .route("/profile/ssh-keys", post(add_ssh_key_handler))
        .route("/profile/ssh-keys/{index}", delete(delete_ssh_key_handler))
}

/// Admin HTML routes, which require admin privileges.
fn admin_page_routes() -> Router<AppState> {
    Router::new()
        .route("/admin/users", get(admin_dashboard_handler))
        .route("/admin/labs", get(admin_labs_list_handler))
        .route("/admin/users/{username}", get(admin_user_edit_handler))
        .route("/admin/users/{username}", delete(admin_delete_user_handler))
        .route(
            "/admin/users/{username}/password",
            post(admin_update_user_password_handler),
        )
        .route(
            "/admin/users/{username}/ssh-keys/{index}",
            delete(admin_delete_ssh_key_handler),
        )
        .route("/admin/tools", get(admin_tools_handler))
        .route(
            "/admin/tools/clean/{lab_id}",
            post(admin_tools_clean_handler),
        )
        .route("/admin/tools/scan", post(admin_tools_scan_handler))
        .route("/admin/images", get(admin_images_list_handler))
        // Upload route (must come before {model} catch-all)
        .route(
            "/admin/images/upload",
            get(admin_image_upload_page_handler).post(admin_image_upload_handler),
        )
        // Each image page names a model and one of its kinds. `versions` and `edit` are
        // matched before a `{version}` of the same place.
        .route(
            "/admin/images/{model}/{kind}/versions",
            get(admin_image_versions_handler),
        )
        .route(
            "/admin/images/{model}/{kind}/{version}",
            get(admin_image_detail_handler),
        )
        .route(
            "/admin/images/{model}/{kind}/{version}/edit",
            get(admin_image_edit_page_handler).post(admin_image_update_handler),
        )
        // Without a version, the model and kind's default version.
        .route(
            "/admin/images/{model}/{kind}",
            get(admin_image_detail_handler),
        )
        .route(
            "/admin/images/{model}/{kind}/edit",
            get(admin_image_edit_page_handler).post(admin_image_update_handler),
        )
}

/// Public API endpoints, which need no authentication.
fn public_api_routes() -> Router<AppState> {
    Router::new()
        .route("/health", get(health_check))
        .route("/cert", get(get_certificate_handler))
        .route("/api/v1/spec", get(api_spec_handler))
        .route("/api/v1/openapi.json", get(openapi_handler))
        .route("/api/docs", get(api_docs_handler))
        .route("/api/v1/auth/login", post(login)) // JSON login for CLI
        .route("/api/v1/auth/validate", post(validate_token_json))
        .route("/api/v1/auth/logout", post(logout_json))
}

/// Protected API endpoints, which require authentication.
fn api_routes() -> Router<AppState> {
    Router::new()
        .route("/api/v1/labs", get(get_labs_json).post(create_lab_json))
        .route("/api/v1/labs/{id}", get(get_lab))
        .route("/api/v1/labs/{id}", delete(delete_lab_json))
        .route("/api/v1/labs/{id}/down", post(down_lab_json))
        .route("/api/v1/labs/{id}/resume", post(resume_lab_json))
        .route("/api/v1/labs/{id}/cancel", post(cancel_lab_json))
        .route(
            "/api/v1/labs/{id}/nodes/{node_name}/redeploy",
            post(redeploy_node_json),
        )
        // A node's serial console, as a WebSocket
        .route(
            "/api/v1/labs/{id}/nodes/{node_name}/console",
            get(node_console_handler),
        )
        // A VM's VNC console, as a WebSocket carrying RFB
        .route(
            "/api/v1/labs/{id}/nodes/{node_name}/vnc",
            get(node_vnc_handler),
        )
        // Link API endpoints
        .route(
            "/api/v1/labs/{lab_id}/links/{link_index}/impairment",
            post(update_impairment_json),
        )
        // Image API endpoints
        .route("/api/v1/images", get(list_images_json))
        .route("/api/v1/images/import", post(import_image_json))
        .route("/api/v1/images/upload", post(upload_image_multipart))
        .route("/api/v1/images/pull", post(pull_image_json))
        .route("/api/v1/images/download", post(download_image_json))
        .route("/api/v1/images/{model}/{kind}", get(show_image_json))
        .route(
            "/api/v1/images/{model}/{kind}/{version}",
            delete(delete_image_json),
        )
        .route(
            "/api/v1/images/{model}/{kind}/{version}/default",
            post(set_default_image_json),
        )
        // Admin API — Tools
        .route("/api/v1/admin/tools/labs/clean/{id}", post(clean_lab_json))
        .route("/api/v1/admin/tools/images/scan", post(scan_images_json))
        // Admin API — Users
        .route(
            "/api/v1/admin/users",
            post(create_user_json).get(list_users_json),
        )
        .route(
            "/api/v1/admin/users/{username}",
            get(get_user_info_json).delete(delete_user_json),
        )
        .route(
            "/api/v1/admin/users/{username}/password",
            post(change_password_json),
        )
        .route(
            "/api/v1/admin/users/{username}/device-password",
            post(set_device_password_json),
        )
        .route(
            "/api/v1/admin/users/{username}/ssh-keys",
            post(add_ssh_key_json).delete(remove_ssh_key_json),
        )
}
