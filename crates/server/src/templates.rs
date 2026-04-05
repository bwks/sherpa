use askama::Template;
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse, Response};
use shared::data::{
    BridgeInfo, DbUser, DestroyError, DeviceInfo, LabInfo, LabSummary, LinkInfo, NodeConfig,
};

use crate::api::handlers::{ImageSummary, UserSummary};

mod filters {
    pub fn initial(s: &str, _: &dyn askama::Values) -> askama::Result<String> {
        Ok(s.chars()
            .next()
            .map(|c| c.to_uppercase().to_string())
            .unwrap_or_default())
    }
}
/// Main dashboard page template
#[derive(Template)]
#[template(path = "user/dashboard.html.jinja")]
pub struct DashboardTemplate {
    pub username: String,
    pub is_admin: bool,
    pub active_page: String,
    pub lab_count: usize,
    pub image_count: usize,
    pub docker_ok: bool,
    pub libvirt_ok: bool,
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

/// User labs list full page template
#[derive(Template)]
#[template(path = "user/labs.html.jinja")]
pub struct LabsListTemplate {
    pub username: String,
    pub is_admin: bool,
    pub active_page: String,
}

impl IntoResponse for LabsListTemplate {
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
#[template(path = "user/partials/labs-grid.html.jinja")]
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
#[template(path = "user/partials/empty-state.html.jinja")]
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
#[template(path = "user/partials/error.html.jinja")]
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
#[template(path = "auth/login.html.jinja")]
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
#[template(path = "auth/login-error.html.jinja")]
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
#[template(path = "auth/signup.html.jinja")]
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
#[template(path = "auth/signup-error.html.jinja")]
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
#[template(path = "error/404.html.jinja")]
#[allow(dead_code)]
pub struct Error404Template {
    pub username: String,
    pub is_admin: bool,
    pub active_page: String,
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
#[template(path = "error/403.html.jinja")]
#[allow(dead_code)]
pub struct Error403Template {
    pub username: String,
    pub is_admin: bool,
    pub active_page: String,
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
#[template(path = "error/403-admin.html.jinja")]
pub struct Admin403Template {
    pub username: String,
    pub is_admin: bool,
    pub active_page: String,
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
#[template(path = "user/lab-detail.html.jinja")]
pub struct LabDetailTemplate {
    pub username: String,
    pub is_admin: bool,
    pub active_page: String,
    pub lab_info: LabInfo,
    pub devices: Vec<DeviceInfo>,
    pub device_count: usize,
    pub inactive_devices: Vec<String>,
    pub inactive_device_count: usize,
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
#[template(path = "user/profile.html.jinja")]
pub struct ProfileTemplate {
    pub username: String,
    pub is_admin: bool,
    pub active_page: String,
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
#[template(path = "user/partials/ssh-keys-list.html.jinja")]
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
#[template(path = "user/partials/password-success.html.jinja")]
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
#[template(path = "user/partials/password-error.html.jinja")]
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
#[template(path = "user/partials/ssh-key-error.html.jinja")]
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

/// Admin users page template
#[derive(Template)]
#[template(path = "admin/users.html.jinja")]
pub struct AdminUsersTemplate {
    pub username: String,
    pub is_admin: bool,
    pub active_page: String,
    pub users: Vec<UserSummary>,
}

impl IntoResponse for AdminUsersTemplate {
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
#[template(path = "admin/labs.html.jinja")]
pub struct AdminLabsTemplate {
    pub username: String,
    pub is_admin: bool,
    pub active_page: String,
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
#[template(path = "admin/user-edit.html.jinja")]
pub struct AdminUserEditTemplate {
    pub username: String,
    pub is_admin: bool,
    pub active_page: String,
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
#[template(path = "admin/partials/ssh-keys-list.html.jinja")]
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
#[template(path = "admin/partials/password-success.html.jinja")]
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
#[template(path = "admin/partials/password-error.html.jinja")]
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
// Admin Image Templates
// ============================================================================

/// Admin images list page template
#[derive(Template)]
#[template(path = "admin/images.html.jinja")]
#[allow(dead_code)]
pub struct AdminImagesListTemplate {
    pub username: String,
    pub is_admin: bool,
    pub active_page: String,
    pub configs: Vec<ImageSummary>,
}

impl IntoResponse for AdminImagesListTemplate {
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

/// Admin image detail page template
#[derive(Template)]
#[template(path = "admin/image-detail.html.jinja")]
#[allow(dead_code)]
pub struct AdminImageDetailTemplate {
    pub username: String,
    pub is_admin: bool,
    pub active_page: String,
    pub config: NodeConfig,
}

impl IntoResponse for AdminImageDetailTemplate {
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

/// Admin image edit page template
#[derive(Template)]
#[template(path = "admin/image-edit.html.jinja")]
#[allow(dead_code)]
pub struct AdminImageEditTemplate {
    pub username: String,
    pub is_admin: bool,
    pub active_page: String,
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

impl IntoResponse for AdminImageEditTemplate {
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

/// Admin image versions list page template
#[derive(Template)]
#[template(path = "admin/image-versions.html.jinja")]
#[allow(dead_code)]
pub struct AdminImageVersionsTemplate {
    pub username: String,
    pub is_admin: bool,
    pub active_page: String,
    pub model: String,
    pub kind: String,
    pub versions: Vec<NodeConfig>,
}

impl IntoResponse for AdminImageVersionsTemplate {
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

/// Admin image upload page template
#[derive(Template)]
#[template(path = "admin/image-upload.html.jinja")]
#[allow(dead_code)]
pub struct AdminImageUploadTemplate {
    pub username: String,
    pub is_admin: bool,
    pub active_page: String,
    pub models: Vec<String>,
}

impl IntoResponse for AdminImageUploadTemplate {
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
// Admin Tools Templates
// ============================================================================

/// Admin tools page template
#[derive(Template)]
#[template(path = "admin/tools.html.jinja")]
pub struct AdminToolsTemplate {
    pub username: String,
    pub is_admin: bool,
    pub active_page: String,
}

impl IntoResponse for AdminToolsTemplate {
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
// Lab Create Templates
// ============================================================================

/// Lab creation page template
#[derive(Template)]
#[template(path = "user/lab-create.html.jinja")]
pub struct LabCreateTemplate {
    pub username: String,
    pub is_admin: bool,
    pub active_page: String,
    pub generated_name: String,
    pub models: Vec<String>,
}

impl IntoResponse for LabCreateTemplate {
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

/// Create progress fragment — SSE-connected container that streams creation progress
#[derive(Template)]
#[template(path = "user/partials/create-progress.html.jinja")]
pub struct LabCreateProgressFragment {
    pub lab_id: String,
    pub lab_name: String,
}

impl IntoResponse for LabCreateProgressFragment {
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

/// Create success summary fragment
#[derive(Template)]
#[template(path = "user/partials/create-summary-success.html.jinja")]
pub struct CreateSummarySuccessFragment {
    pub lab_id: String,
    pub lab_name: String,
    pub containers: usize,
    pub vms: usize,
    pub unikernels: usize,
    pub networks: usize,
    pub bridges: usize,
    pub interfaces: usize,
    pub total_time_secs: u64,
}

/// Create failure fragment
#[derive(Template)]
#[template(path = "user/partials/create-summary-failed.html.jinja")]
pub struct CreateSummaryFailedFragment {
    pub message: String,
}

// ============================================================================
// Lab Nodes Polling Fragment
// ============================================================================

/// Nodes table partial for HTMX polling
#[derive(Template)]
#[template(path = "user/partials/nodes-table.html.jinja")]
pub struct NodesTableFragment {
    pub devices: Vec<DeviceInfo>,
    pub device_count: usize,
}

impl IntoResponse for NodesTableFragment {
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
#[template(path = "user/partials/destroy-button.html.jinja")]
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
#[template(path = "user/partials/destroy-confirm.html.jinja")]
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
#[template(path = "user/partials/destroy-progress.html.jinja")]
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
#[template(path = "user/partials/destroy-progress-line.html.jinja")]
pub struct DestroyProgressLineFragment {
    pub emoji: String,
    pub message: String,
}

/// Destroy success summary fragment
#[derive(Template)]
#[template(path = "user/partials/destroy-summary-success.html.jinja")]
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
#[template(path = "user/partials/destroy-summary-errors.html.jinja")]
pub struct DestroySummaryErrorsFragment {
    pub lab_name: String,
    pub errors: Vec<DestroyError>,
}

/// Destroy failure fragment
#[derive(Template)]
#[template(path = "user/partials/destroy-summary-failed.html.jinja")]
pub struct DestroySummaryFailedFragment {
    pub message: String,
}

#[cfg(test)]
mod tests {
    use askama::{NO_VALUES, Template};

    use super::filters::initial;
    use super::*;

    // ========================================================================
    // Filter tests
    // ========================================================================

    #[test]
    fn test_initial_lowercase() {
        assert_eq!(initial("bradmin", NO_VALUES).unwrap(), "B");
    }

    #[test]
    fn test_initial_uppercase() {
        assert_eq!(initial("Admin", NO_VALUES).unwrap(), "A");
    }

    #[test]
    fn test_initial_empty() {
        assert_eq!(initial("", NO_VALUES).unwrap(), "");
    }

    // ========================================================================
    // Template render tests
    // ========================================================================

    #[test]
    fn test_dashboard_template_renders() {
        let tpl = DashboardTemplate {
            username: "testuser".to_string(),
            is_admin: false,
            active_page: "dashboard".to_string(),
            lab_count: 3,
            image_count: 12,
            docker_ok: true,
            libvirt_ok: true,
        };
        let html = tpl.render().expect("template should render");
        assert!(html.contains("testuser"));
    }

    #[test]
    fn test_labs_list_template_renders() {
        let tpl = LabsListTemplate {
            username: "testuser".to_string(),
            is_admin: false,
            active_page: "labs".to_string(),
        };
        let html = tpl.render().expect("template should render");
        assert!(html.contains("testuser"));
    }

    #[test]
    fn test_labs_grid_template_renders_empty() {
        let tpl = LabsGridTemplate { labs: vec![] };
        let html = tpl.render().expect("template should render");
        // Empty grid still renders the table structure
        assert!(html.contains("<table"));
        assert!(html.contains("Name"));
    }

    #[test]
    fn test_labs_grid_template_renders_with_labs() {
        let tpl = LabsGridTemplate {
            labs: vec![LabSummary {
                id: "a10736e8".to_string(),
                name: "test-lab".to_string(),
                status: shared::data::LabStatus::Unknown,
                node_count: 2,
            }],
        };
        let html = tpl.render().expect("template should render");
        assert!(html.contains("test-lab"));
        assert!(html.contains("a10736e8"));
    }

    #[test]
    fn test_lab_create_template_renders() {
        let tpl = LabCreateTemplate {
            username: "testuser".to_string(),
            is_admin: false,
            active_page: "labs".to_string(),
            generated_name: "happy-panda".to_string(),
            models: vec!["cisco_iosv".to_string(), "ubuntu_linux".to_string()],
        };
        let html = tpl.render().expect("template should render");
        assert!(html.contains("happy-panda"));
        assert!(html.contains("cisco_iosv"));
        assert!(html.contains("ubuntu_linux"));
    }

    #[test]
    fn test_lab_create_progress_fragment_renders() {
        let tpl = LabCreateProgressFragment {
            lab_id: "a10736e8".to_string(),
            lab_name: "test-lab".to_string(),
        };
        let html = tpl.render().expect("template should render");
        assert!(html.contains("a10736e8"));
    }

    #[test]
    fn test_create_summary_failed_fragment_renders() {
        let tpl = CreateSummaryFailedFragment {
            message: "something went wrong".to_string(),
        };
        let html = tpl.render().expect("template should render");
        assert!(html.contains("something went wrong"));
    }

    #[test]
    fn test_admin_tools_template_renders() {
        let tpl = AdminToolsTemplate {
            username: "admin".to_string(),
            is_admin: true,
            active_page: "admin_tools".to_string(),
        };
        let html = tpl.render().expect("template should render");
        assert!(html.contains("Tools"));
        assert!(html.contains("Lab Clean"));
        assert!(html.contains("lab-id"));
    }

    #[test]
    fn test_admin_tools_template_has_script_src() {
        let tpl = AdminToolsTemplate {
            username: "admin".to_string(),
            is_admin: true,
            active_page: "admin_tools".to_string(),
        };
        let html = tpl.render().expect("template should render");
        assert!(html.contains("admin-tools.js"));
    }

    #[test]
    fn test_admin_labs_template_renders_empty() {
        let tpl = AdminLabsTemplate {
            username: "admin".to_string(),
            is_admin: true,
            active_page: "admin_labs".to_string(),
            labs: vec![],
        };
        let html = tpl.render().expect("template should render");
        assert!(html.contains("No labs found"));
    }

    #[test]
    fn test_admin_labs_template_renders_with_labs() {
        let tpl = AdminLabsTemplate {
            username: "admin".to_string(),
            is_admin: true,
            active_page: "admin_labs".to_string(),
            labs: vec![AdminLabSummary {
                lab_id: "abc12345".to_string(),
                name: "my-lab".to_string(),
                owner_username: "alice".to_string(),
                node_count: 3,
            }],
        };
        let html = tpl.render().expect("template should render");
        assert!(html.contains("my-lab"));
        assert!(html.contains("abc12345"));
        assert!(html.contains("alice"));
    }

    #[test]
    fn test_login_page_template_renders() {
        let tpl = LoginPageTemplate {
            error: String::new(),
            message: String::new(),
        };
        let html = tpl.render().expect("template should render");
        assert!(html.contains("Login") || html.contains("login"));
    }

    #[test]
    fn test_login_error_template_renders() {
        let tpl = LoginErrorTemplate {
            message: "Invalid credentials".to_string(),
        };
        let html = tpl.render().expect("template should render");
        assert!(html.contains("Invalid credentials"));
    }

    #[test]
    fn test_signup_page_template_renders() {
        let tpl = SignupPageTemplate {};
        let html = tpl.render().expect("template should render");
        assert!(html.contains("Sign") || html.contains("sign"));
    }

    #[test]
    fn test_error_404_template_renders() {
        let tpl = Error404Template {
            username: "testuser".to_string(),
            is_admin: false,
            active_page: "".to_string(),
            message: "Page not found".to_string(),
        };
        let html = tpl.render().expect("template should render");
        assert!(html.contains("404") || html.contains("not found") || html.contains("Not Found"));
    }

    #[test]
    fn test_error_template_renders() {
        let tpl = ErrorTemplate {
            message: "something broke".to_string(),
        };
        let html = tpl.render().expect("template should render");
        assert!(html.contains("something broke"));
    }

    #[test]
    fn test_empty_state_template_renders() {
        let tpl = EmptyStateTemplate {
            username: "testuser".to_string(),
        };
        let html = tpl.render().expect("template should render");
        assert!(html.contains("testuser") || html.contains("no labs") || html.contains("No labs"));
    }

    // ========================================================================
    // Sidebar rendering — active page highlighting
    // ========================================================================

    #[test]
    fn test_sidebar_highlights_admin_tools() {
        let tpl = AdminToolsTemplate {
            username: "admin".to_string(),
            is_admin: true,
            active_page: "admin_tools".to_string(),
        };
        let html = tpl.render().expect("template should render");
        // The active link should have the accent class
        assert!(html.contains("/admin/tools"));
    }

    #[test]
    fn test_sidebar_shows_admin_section_for_admins() {
        let tpl = DashboardTemplate {
            username: "admin".to_string(),
            is_admin: true,
            active_page: "dashboard".to_string(),
            lab_count: 5,
            image_count: 10,
            docker_ok: true,
            libvirt_ok: true,
        };
        let html = tpl.render().expect("template should render");
        assert!(html.contains("/admin/users"));
        assert!(html.contains("/admin/labs"));
        assert!(html.contains("/admin/tools"));
        assert!(html.contains("/admin/images"));
    }

    #[test]
    fn test_sidebar_hides_admin_section_for_non_admins() {
        let tpl = DashboardTemplate {
            username: "user".to_string(),
            is_admin: false,
            active_page: "dashboard".to_string(),
            lab_count: 0,
            image_count: 0,
            docker_ok: true,
            libvirt_ok: true,
        };
        let html = tpl.render().expect("template should render");
        assert!(!html.contains("/admin/users"));
        assert!(!html.contains("/admin/labs"));
        assert!(!html.contains("/admin/tools"));
    }

    #[test]
    fn test_dashboard_shows_stats_cards() {
        let tpl = DashboardTemplate {
            username: "testuser".to_string(),
            is_admin: false,
            active_page: "dashboard".to_string(),
            lab_count: 7,
            image_count: 15,
            docker_ok: true,
            libvirt_ok: true,
        };
        let html = tpl.render().expect("template should render");
        assert!(html.contains("My Labs"));
        assert!(html.contains("Node Images"));
        assert!(html.contains(">7<"));
        assert!(html.contains(">15<"));
    }

    #[test]
    fn test_admin_users_page_renders() {
        let tpl = AdminUsersTemplate {
            username: "admin".to_string(),
            is_admin: true,
            active_page: "admin_users".to_string(),
            users: vec![],
        };
        let html = tpl.render().expect("template should render");
        assert!(html.contains("User Management"));
        assert!(html.contains("No users found"));
    }
}
