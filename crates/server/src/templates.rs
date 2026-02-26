use askama::Template;
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse, Response};
use shared::data::{
    BridgeInfo, DbUser, DestroyError, DeviceInfo, LabInfo, LabSummary, LinkInfo, NodeConfig,
};

use crate::api::handlers::{NodeImageSummary, UserSummary};
/// Main dashboard page template
#[derive(Template)]
#[template(path = "dashboard.html.jinja")]
pub struct DashboardTemplate {
    pub username: String,
    pub is_admin: bool,
}

impl IntoResponse for DashboardTemplate {
    fn into_response(self) -> Response {
        match self.render() {
            Ok(html) => Html(html).into_response(),
            Err(err) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to render template: {}", err),
            )
                .into_response(),
        }
    }
}

/// Labs grid partial template - displays all labs
#[derive(Template)]
#[template(path = "partials/labs-grid.html.jinja")]
pub struct LabsGridTemplate {
    pub labs: Vec<LabSummary>,
}

impl IntoResponse for LabsGridTemplate {
    fn into_response(self) -> Response {
        match self.render() {
            Ok(html) => Html(html).into_response(),
            Err(err) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to render template: {}", err),
            )
                .into_response(),
        }
    }
}

/// Empty state partial template - shown when no labs exist
#[derive(Template)]
#[template(path = "partials/empty-state.html.jinja")]
pub struct EmptyStateTemplate {
    pub username: String,
}

impl IntoResponse for EmptyStateTemplate {
    fn into_response(self) -> Response {
        match self.render() {
            Ok(html) => Html(html).into_response(),
            Err(err) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to render template: {}", err),
            )
                .into_response(),
        }
    }
}

/// Error partial template - displays error messages
#[derive(Template)]
#[template(path = "partials/error.html.jinja")]
pub struct ErrorTemplate {
    pub message: String,
}

impl IntoResponse for ErrorTemplate {
    fn into_response(self) -> Response {
        match self.render() {
            Ok(html) => Html(html).into_response(),
            Err(err) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to render template: {}", err),
            )
                .into_response(),
        }
    }
}

// ============================================================================
// Authentication Templates
// ============================================================================

/// Login page template
#[derive(Template)]
#[template(path = "login.html.jinja")]
pub struct LoginPageTemplate {
    pub error: String,
    pub message: String,
}

impl IntoResponse for LoginPageTemplate {
    fn into_response(self) -> Response {
        match self.render() {
            Ok(html) => Html(html).into_response(),
            Err(err) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to render template: {}", err),
            )
                .into_response(),
        }
    }
}

/// Login error partial template
#[derive(Template)]
#[template(path = "partials/login-error.html.jinja")]
pub struct LoginErrorTemplate {
    pub message: String,
}

impl IntoResponse for LoginErrorTemplate {
    fn into_response(self) -> Response {
        match self.render() {
            Ok(html) => Html(html).into_response(),
            Err(err) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to render template: {}", err),
            )
                .into_response(),
        }
    }
}

/// Signup page template
#[derive(Template)]
#[template(path = "signup.html.jinja")]
pub struct SignupPageTemplate {}

impl IntoResponse for SignupPageTemplate {
    fn into_response(self) -> Response {
        match self.render() {
            Ok(html) => Html(html).into_response(),
            Err(err) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to render template: {}", err),
            )
                .into_response(),
        }
    }
}

/// Signup error partial template
#[derive(Template)]
#[template(path = "partials/signup-error.html.jinja")]
pub struct SignupErrorTemplate {
    pub message: String,
}

impl IntoResponse for SignupErrorTemplate {
    fn into_response(self) -> Response {
        match self.render() {
            Ok(html) => Html(html).into_response(),
            Err(err) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to render template: {}", err),
            )
                .into_response(),
        }
    }
}

// ============================================================================
// Error Page Templates
// ============================================================================

/// 404 Not Found error page template
#[derive(Template)]
#[template(path = "error-404.html.jinja")]
#[allow(dead_code)]
pub struct Error404Template {
    pub username: String,
    pub message: String,
}

impl IntoResponse for Error404Template {
    fn into_response(self) -> Response {
        match self.render() {
            Ok(html) => (StatusCode::NOT_FOUND, Html(html)).into_response(),
            Err(err) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to render template: {}", err),
            )
                .into_response(),
        }
    }
}

/// 403 Forbidden error page template
#[derive(Template)]
#[template(path = "error-403.html.jinja")]
#[allow(dead_code)]
pub struct Error403Template {
    pub username: String,
    pub message: String,
}

impl IntoResponse for Error403Template {
    fn into_response(self) -> Response {
        match self.render() {
            Ok(html) => (StatusCode::FORBIDDEN, Html(html)).into_response(),
            Err(err) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to render template: {}", err),
            )
                .into_response(),
        }
    }
}

/// 403 Forbidden error page for admin access denied
#[derive(Template)]
#[template(path = "error-403-admin.html.jinja")]
pub struct Admin403Template {
    pub username: String,
}

impl IntoResponse for Admin403Template {
    fn into_response(self) -> Response {
        match self.render() {
            Ok(html) => (StatusCode::FORBIDDEN, Html(html)).into_response(),
            Err(err) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to render template: {}", err),
            )
                .into_response(),
        }
    }
}

// ============================================================================
// Lab Detail Templates
// ============================================================================

/// Lab detail page template
#[derive(Template)]
#[template(path = "lab-detail.html.jinja")]
pub struct LabDetailTemplate {
    pub username: String,
    pub is_admin: bool,
    pub lab_info: LabInfo,
    pub devices: Vec<DeviceInfo>,
    pub device_count: usize,
    pub links: Vec<LinkInfo>,
    pub link_count: usize,
    pub bridges: Vec<BridgeInfo>,
    pub bridge_count: usize,
}

impl IntoResponse for LabDetailTemplate {
    fn into_response(self) -> Response {
        match self.render() {
            Ok(html) => Html(html).into_response(),
            Err(err) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to render template: {}", err),
            )
                .into_response(),
        }
    }
}

// ============================================================================
// Profile Management Templates
// ============================================================================

/// User profile page template
#[derive(Template)]
#[template(path = "profile.html.jinja")]
pub struct ProfileTemplate {
    pub username: String,
    pub is_admin: bool,
    pub ssh_keys_html: String,
}

impl IntoResponse for ProfileTemplate {
    fn into_response(self) -> Response {
        match self.render() {
            Ok(html) => Html(html).into_response(),
            Err(err) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to render template: {}", err),
            )
                .into_response(),
        }
    }
}

/// SSH keys list partial template
#[derive(Template)]
#[template(path = "partials/ssh-keys-list.html.jinja")]
pub struct SshKeysListTemplate {
    pub ssh_keys: Vec<String>,
}

impl IntoResponse for SshKeysListTemplate {
    fn into_response(self) -> Response {
        match self.render() {
            Ok(html) => Html(html).into_response(),
            Err(err) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to render template: {}", err),
            )
                .into_response(),
        }
    }
}

/// Password update success message template
#[derive(Template)]
#[template(path = "partials/password-success.html.jinja")]
pub struct PasswordSuccessTemplate {
    pub message: String,
}

impl IntoResponse for PasswordSuccessTemplate {
    fn into_response(self) -> Response {
        match self.render() {
            Ok(html) => Html(html).into_response(),
            Err(err) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to render template: {}", err),
            )
                .into_response(),
        }
    }
}

/// Password update error message template
#[derive(Template)]
#[template(path = "partials/password-error.html.jinja")]
pub struct PasswordErrorTemplate {
    pub message: String,
}

impl IntoResponse for PasswordErrorTemplate {
    fn into_response(self) -> Response {
        match self.render() {
            Ok(html) => Html(html).into_response(),
            Err(err) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to render template: {}", err),
            )
                .into_response(),
        }
    }
}

/// SSH key operation error message template
#[derive(Template)]
#[template(path = "partials/ssh-key-error.html.jinja")]
pub struct SshKeyErrorTemplate {
    pub message: String,
}

impl IntoResponse for SshKeyErrorTemplate {
    fn into_response(self) -> Response {
        match self.render() {
            Ok(html) => Html(html).into_response(),
            Err(err) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to render template: {}", err),
            )
                .into_response(),
        }
    }
}

// ============================================================================
// Admin User Management Templates
// ============================================================================

/// Admin dashboard page template
#[derive(Template)]
#[template(path = "admin-dashboard.html.jinja")]
pub struct AdminDashboardTemplate {
    pub username: String,
    pub users: Vec<UserSummary>,
}

impl IntoResponse for AdminDashboardTemplate {
    fn into_response(self) -> Response {
        match self.render() {
            Ok(html) => Html(html).into_response(),
            Err(err) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to render template: {}", err),
            )
                .into_response(),
        }
    }
}

/// Admin labs list page template
#[derive(Template)]
#[template(path = "admin-labs.html.jinja")]
pub struct AdminLabsTemplate {
    pub username: String,
    pub labs: Vec<AdminLabSummary>,
}

/// Summary data for a lab displayed in the admin labs list
pub struct AdminLabSummary {
    pub lab_id: String,
    pub name: String,
    pub owner_username: String,
    pub node_count: usize,
}

impl IntoResponse for AdminLabsTemplate {
    fn into_response(self) -> Response {
        match self.render() {
            Ok(html) => Html(html).into_response(),
            Err(err) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to render template: {}", err),
            )
                .into_response(),
        }
    }
}

/// Admin user edit page template
#[derive(Template)]
#[template(path = "admin-user-edit.html.jinja")]
pub struct AdminUserEditTemplate {
    pub admin_username: String,
    pub target_user: DbUser,
    pub is_self: bool,
    pub ssh_keys_html: String,
}

impl IntoResponse for AdminUserEditTemplate {
    fn into_response(self) -> Response {
        match self.render() {
            Ok(html) => Html(html).into_response(),
            Err(err) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to render template: {}", err),
            )
                .into_response(),
        }
    }
}

/// Admin SSH keys list partial template
#[derive(Template)]
#[template(path = "partials/admin-ssh-keys-list.html.jinja")]
pub struct AdminSshKeysListTemplate {
    pub target_username: String,
    pub ssh_keys: Vec<String>,
    pub success_message: String,
    pub is_error: bool,
}

impl IntoResponse for AdminSshKeysListTemplate {
    fn into_response(self) -> Response {
        match self.render() {
            Ok(html) => Html(html).into_response(),
            Err(err) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to render template: {}", err),
            )
                .into_response(),
        }
    }
}

/// Admin password update success message template
#[derive(Template)]
#[template(path = "partials/admin-password-success.html.jinja")]
pub struct AdminPasswordSuccessTemplate {
    pub target_username: String,
}

impl IntoResponse for AdminPasswordSuccessTemplate {
    fn into_response(self) -> Response {
        match self.render() {
            Ok(html) => Html(html).into_response(),
            Err(err) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to render template: {}", err),
            )
                .into_response(),
        }
    }
}

/// Admin password update error message template
#[derive(Template)]
#[template(path = "partials/admin-password-error.html.jinja")]
pub struct AdminPasswordErrorTemplate {
    pub error_message: String,
}

impl IntoResponse for AdminPasswordErrorTemplate {
    fn into_response(self) -> Response {
        match self.render() {
            Ok(html) => Html(html).into_response(),
            Err(err) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to render template: {}", err),
            )
                .into_response(),
        }
    }
}

// ============================================================================
// Admin Node Image Templates
// ============================================================================

/// Admin node images list page template
#[derive(Template)]
#[template(path = "admin-node-images.html.jinja")]
#[allow(dead_code)]
pub struct AdminNodeImagesListTemplate {
    pub username: String,
    pub is_admin: bool,
    pub configs: Vec<NodeImageSummary>,
}

impl IntoResponse for AdminNodeImagesListTemplate {
    fn into_response(self) -> Response {
        match self.render() {
            Ok(html) => Html(html).into_response(),
            Err(err) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to render template: {}", err),
            )
                .into_response(),
        }
    }
}

/// Admin node image detail page template
#[derive(Template)]
#[template(path = "admin-node-image-detail.html.jinja")]
#[allow(dead_code)]
pub struct AdminNodeImageDetailTemplate {
    pub username: String,
    pub is_admin: bool,
    pub config: NodeConfig,
}

impl IntoResponse for AdminNodeImageDetailTemplate {
    fn into_response(self) -> Response {
        match self.render() {
            Ok(html) => Html(html).into_response(),
            Err(err) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to render template: {}", err),
            )
                .into_response(),
        }
    }
}

/// Admin node image edit page template
#[derive(Template)]
#[template(path = "admin-node-image-edit.html.jinja")]
#[allow(dead_code)]
pub struct AdminNodeImageEditTemplate {
    pub username: String,
    pub is_admin: bool,
    pub config: NodeConfig,
    pub os_variants: Vec<String>,
    pub bios_types: Vec<String>,
    pub cpu_architectures: Vec<String>,
    pub cpu_models: Vec<String>,
    pub machine_types: Vec<String>,
    pub disk_buses: Vec<String>,
    pub ztp_methods: Vec<String>,
    pub interface_types: Vec<String>,
}

impl IntoResponse for AdminNodeImageEditTemplate {
    fn into_response(self) -> Response {
        match self.render() {
            Ok(html) => Html(html).into_response(),
            Err(err) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to render template: {}", err),
            )
                .into_response(),
        }
    }
}

/// Admin node image versions list page template
#[derive(Template)]
#[template(path = "admin-node-image-versions.html.jinja")]
#[allow(dead_code)]
pub struct AdminNodeImageVersionsTemplate {
    pub username: String,
    pub is_admin: bool,
    pub model: String,
    pub kind: String,
    pub versions: Vec<NodeConfig>,
}

impl IntoResponse for AdminNodeImageVersionsTemplate {
    fn into_response(self) -> Response {
        match self.render() {
            Ok(html) => Html(html).into_response(),
            Err(err) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to render template: {}", err),
            )
                .into_response(),
        }
    }
}

// ============================================================================
// Lab Destroy Fragment Templates
// ============================================================================

/// Destroy button fragment — the red "Destroy Lab" button
#[derive(Template)]
#[template(path = "partials/destroy-button.html.jinja")]
pub struct LabDestroyButtonFragment {
    pub lab_id: String,
}

impl IntoResponse for LabDestroyButtonFragment {
    fn into_response(self) -> Response {
        match self.render() {
            Ok(html) => Html(html).into_response(),
            Err(err) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to render template: {}", err),
            )
                .into_response(),
        }
    }
}

/// Destroy confirmation fragment — warning panel with confirm/cancel buttons
#[derive(Template)]
#[template(path = "partials/destroy-confirm.html.jinja")]
pub struct LabDestroyConfirmFragment {
    pub lab_id: String,
    pub lab_name: String,
    pub device_count: usize,
}

impl IntoResponse for LabDestroyConfirmFragment {
    fn into_response(self) -> Response {
        match self.render() {
            Ok(html) => Html(html).into_response(),
            Err(err) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to render template: {}", err),
            )
                .into_response(),
        }
    }
}

/// Destroy progress fragment — SSE-connected container that streams progress
#[derive(Template)]
#[template(path = "partials/destroy-progress.html.jinja")]
pub struct LabDestroyProgressFragment {
    pub lab_id: String,
}

impl IntoResponse for LabDestroyProgressFragment {
    fn into_response(self) -> Response {
        match self.render() {
            Ok(html) => Html(html).into_response(),
            Err(err) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to render template: {}", err),
            )
                .into_response(),
        }
    }
}

// ============================================================================
// SSE Destroy Stream Fragment Templates
// ============================================================================

/// Single progress line fragment for SSE streaming
#[derive(Template)]
#[template(path = "partials/destroy-progress-line.html.jinja")]
pub struct DestroyProgressLineFragment {
    pub emoji: String,
    pub message: String,
}

/// Destroy success summary fragment
#[derive(Template)]
#[template(path = "partials/destroy-summary-success.html.jinja")]
pub struct DestroySummarySuccessFragment {
    pub lab_name: String,
    pub containers: usize,
    pub vms: usize,
    pub disks: usize,
    pub networks: usize,
    pub interfaces: usize,
}

/// Destroy summary with errors fragment
#[derive(Template)]
#[template(path = "partials/destroy-summary-errors.html.jinja")]
pub struct DestroySummaryErrorsFragment {
    pub lab_name: String,
    pub errors: Vec<DestroyError>,
}

/// Destroy failure fragment
#[derive(Template)]
#[template(path = "partials/destroy-summary-failed.html.jinja")]
pub struct DestroySummaryFailedFragment {
    pub message: String,
}
