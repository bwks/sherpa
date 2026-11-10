use askama::Template;
use askama_web::WebTemplate;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use shared::data::{
    BridgeInfo, DbUser, DestroyError, DeviceInfo, LabInfo, LabState, LabSummary, LinkInfo,
    NodeConfig, NodeKind,
};

use crate::api::handlers::{ImageSummary, UserSummary};

/// Main dashboard page template
#[derive(Template, WebTemplate)]
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

/// User labs list full page template
#[derive(Template, WebTemplate)]
#[template(path = "user/labs.html.jinja")]
pub struct LabsListTemplate {
    pub username: String,
    pub is_admin: bool,
    pub active_page: String,
}

/// Labs grid partial template - displays all labs
#[derive(Template, WebTemplate)]
#[template(path = "user/partials/labs-grid.html.jinja")]
pub struct LabsGridTemplate {
    pub labs: Vec<LabSummary>,
}

/// Empty state partial template - shown when no labs exist
#[derive(Template, WebTemplate)]
#[template(path = "user/partials/empty-state.html.jinja")]
pub struct EmptyStateTemplate {
    pub username: String,
}

/// Error partial template - displays error messages
#[derive(Template, WebTemplate)]
#[template(path = "user/partials/error.html.jinja")]
pub struct ErrorTemplate {
    pub message: String,
}

// ============================================================================
// Authentication Templates
// ============================================================================

/// Login page template
#[derive(Template, WebTemplate)]
#[template(path = "auth/login.html.jinja")]
pub struct LoginPageTemplate {
    pub error: String,
    pub message: String,
    /// Whether to link to the signup page; see `Config::allow_signup`.
    pub allow_signup: bool,
}

/// Login error partial template
#[derive(Template, WebTemplate)]
#[template(path = "auth/login-error.html.jinja")]
pub struct LoginErrorTemplate {
    pub message: String,
}

/// Signup page template
#[derive(Template, WebTemplate)]
#[template(path = "auth/signup.html.jinja")]
pub struct SignupPageTemplate {}

/// Signup error partial template
#[derive(Template, WebTemplate)]
#[template(path = "auth/signup-error.html.jinja")]
pub struct SignupErrorTemplate {
    pub message: String,
}

// ============================================================================
// Error Page Templates
// ============================================================================

/// 404 Not Found error page template
#[derive(Template)]
#[template(path = "error/404.html.jinja")]
pub struct Error404Template {
    pub username: String,
    pub is_admin: bool,
    pub active_page: String,
    pub message: String,
}

impl IntoResponse for Error404Template {
    fn into_response(self) -> Response {
        (StatusCode::NOT_FOUND, WebTemplate(self)).into_response()
    }
}

/// 403 Forbidden error page template
#[derive(Template)]
#[template(path = "error/403.html.jinja")]
pub struct Error403Template {
    pub username: String,
    pub is_admin: bool,
    pub active_page: String,
    pub message: String,
}

impl IntoResponse for Error403Template {
    fn into_response(self) -> Response {
        (StatusCode::FORBIDDEN, WebTemplate(self)).into_response()
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
        (StatusCode::FORBIDDEN, WebTemplate(self)).into_response()
    }
}

// ============================================================================
// Lab Detail Templates
// ============================================================================

/// Lab detail page template
#[derive(Template, WebTemplate)]
#[template(path = "user/lab-detail.html.jinja")]
pub struct LabDetailTemplate {
    pub username: String,
    pub is_admin: bool,
    pub active_page: String,
    pub lab_id: String,
    pub lab_info: LabInfo,
    pub lab_state: LabState,
    pub devices: Vec<DeviceInfo>,
    pub device_count: usize,
    pub inactive_devices: Vec<String>,
    pub inactive_device_count: usize,
    pub links: Vec<LinkInfo>,
    pub link_count: usize,
    pub bridges: Vec<BridgeInfo>,
    pub bridge_count: usize,
}

// ============================================================================
// Node Detail Template
// ============================================================================

/// Node detail page template
#[derive(Template, WebTemplate)]
#[template(path = "user/node-detail.html.jinja")]
pub struct NodeDetailTemplate {
    pub username: String,
    pub is_admin: bool,
    pub active_page: String,
    pub lab_id: String,
    pub lab_name: String,
    pub device: DeviceInfo,
    pub node_config: NodeConfig,
    pub mgmt_mac: Option<String>,
}

impl NodeDetailTemplate {
    /// The node's vCPUs. Sherpa does not limit containers.
    pub fn cpu_display(&self) -> String {
        resource_display(self.device.cpu_count.map(u16::from), &self.device.kind)
    }

    /// The node's memory in MB.
    pub fn memory_display(&self) -> String {
        resource_display(self.device.memory, &self.device.kind)
    }
}

fn resource_display(value: Option<u16>, kind: &NodeKind) -> String {
    match (value, kind) {
        (Some(value), _) => value.to_string(),
        (None, NodeKind::Container) => "Not limited".to_string(),
        (None, _) => "Unknown".to_string(),
    }
}

// ============================================================================
// Profile Management Templates
// ============================================================================

/// User profile page template
#[derive(Template, WebTemplate)]
#[template(path = "user/profile.html.jinja")]
pub struct ProfileTemplate {
    pub username: String,
    pub is_admin: bool,
    pub active_page: String,
    /// Rendered in place, and on its own for HTMX swaps.
    pub ssh_keys_list: SshKeysListTemplate,
}

/// SSH keys list partial template
#[derive(Template, WebTemplate)]
#[template(path = "user/partials/ssh-keys-list.html.jinja")]
pub struct SshKeysListTemplate {
    pub ssh_keys: Vec<String>,
}

/// Password update success message template
#[derive(Template, WebTemplate)]
#[template(path = "user/partials/password-success.html.jinja")]
pub struct PasswordSuccessTemplate {
    pub message: String,
}

/// Password update error message template
#[derive(Template, WebTemplate)]
#[template(path = "user/partials/password-error.html.jinja")]
pub struct PasswordErrorTemplate {
    pub message: String,
}

/// SSH key operation error message template
#[derive(Template, WebTemplate)]
#[template(path = "user/partials/ssh-key-error.html.jinja")]
pub struct SshKeyErrorTemplate {
    pub message: String,
}

// ============================================================================
// Admin User Management Templates
// ============================================================================

/// Admin users page template
#[derive(Template, WebTemplate)]
#[template(path = "admin/users.html.jinja")]
pub struct AdminUsersTemplate {
    pub username: String,
    pub is_admin: bool,
    pub active_page: String,
    pub users: Vec<UserSummary>,
}

/// Admin labs list page template
#[derive(Template, WebTemplate)]
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
    pub description: Option<String>,
    pub owner_username: String,
    pub node_count: usize,
}

/// Admin user edit page template
#[derive(Template, WebTemplate)]
#[template(path = "admin/user-edit.html.jinja")]
pub struct AdminUserEditTemplate {
    pub username: String,
    pub is_admin: bool,
    pub active_page: String,
    pub target_user: DbUser,
    pub is_self: bool,
    /// Rendered in place, and on its own for HTMX swaps.
    pub ssh_keys_list: AdminSshKeysListTemplate,
}

/// Admin SSH keys list partial template
#[derive(Template, WebTemplate)]
#[template(path = "admin/partials/ssh-keys-list.html.jinja")]
pub struct AdminSshKeysListTemplate {
    pub target_username: String,
    pub ssh_keys: Vec<String>,
    pub success_message: String,
    pub is_error: bool,
}

/// Admin password update success message template
#[derive(Template, WebTemplate)]
#[template(path = "admin/partials/password-success.html.jinja")]
pub struct AdminPasswordSuccessTemplate {
    pub target_username: String,
}

/// Admin password update error message template
#[derive(Template, WebTemplate)]
#[template(path = "admin/partials/password-error.html.jinja")]
pub struct AdminPasswordErrorTemplate {
    pub error_message: String,
}

// ============================================================================
// Admin Image Templates
// ============================================================================

/// Admin images list page template
#[derive(Template, WebTemplate)]
#[template(path = "admin/images.html.jinja")]
pub struct AdminImagesListTemplate {
    pub username: String,
    pub is_admin: bool,
    pub active_page: String,
    pub configs: Vec<ImageSummary>,
}

/// Admin image detail page template
#[derive(Template, WebTemplate)]
#[template(path = "admin/image-detail.html.jinja")]
pub struct AdminImageDetailTemplate {
    pub username: String,
    pub is_admin: bool,
    pub active_page: String,
    pub config: NodeConfig,
}

/// Admin image edit page template
#[derive(Template, WebTemplate)]
#[template(path = "admin/image-edit.html.jinja")]
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

/// Admin image versions list page template
#[derive(Template, WebTemplate)]
#[template(path = "admin/image-versions.html.jinja")]
pub struct AdminImageVersionsTemplate {
    pub username: String,
    pub is_admin: bool,
    pub active_page: String,
    pub model: String,
    pub kind: String,
    pub versions: Vec<NodeConfig>,
}

/// Admin image upload page template
#[derive(Template, WebTemplate)]
#[template(path = "admin/image-upload.html.jinja")]
pub struct AdminImageUploadTemplate {
    pub username: String,
    pub is_admin: bool,
    pub active_page: String,
    pub models: Vec<String>,
}

/// Error notification returned by admin image form requests.
#[derive(Template, WebTemplate)]
#[template(path = "admin/partials/notification-error.html.jinja", escape = "html")]
pub struct AdminNotificationErrorTemplate {
    pub message: String,
}

// ============================================================================
// Admin Tools Templates
// ============================================================================

/// Admin tools page template
#[derive(Template, WebTemplate)]
#[template(path = "admin/tools.html.jinja")]
pub struct AdminToolsTemplate {
    pub username: String,
    pub is_admin: bool,
    pub active_page: String,
}

// ============================================================================
// Lab Create Templates
// ============================================================================

/// Lab creation page template
#[derive(Template, WebTemplate)]
#[template(path = "user/lab-create.html.jinja")]
pub struct LabCreateTemplate {
    pub username: String,
    pub is_admin: bool,
    pub active_page: String,
    pub generated_name: String,
    pub models: Vec<String>,
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
#[derive(Template, WebTemplate)]
#[template(path = "user/partials/nodes-table.html.jinja")]
pub struct NodesTableFragment {
    pub lab_id: String,
    pub devices: Vec<DeviceInfo>,
    pub device_count: usize,
}

// ============================================================================
// Lab Destroy Fragment Templates
// ============================================================================

/// Destroy button fragment — the red "Destroy Lab" button
#[derive(Template, WebTemplate)]
#[template(path = "user/partials/destroy-button.html.jinja")]
pub struct LabDestroyButtonFragment {
    pub lab_id: String,
}

/// Destroy confirmation fragment — warning panel with confirm/cancel buttons
#[derive(Template, WebTemplate)]
#[template(path = "user/partials/destroy-confirm.html.jinja")]
pub struct LabDestroyConfirmFragment {
    pub lab_id: String,
    pub lab_name: String,
    pub device_count: usize,
}

// ============================================================================
// Job Page Template
// ============================================================================

/// Full-page job progress view — SSE-connected page for streaming operation progress
#[derive(Template, WebTemplate)]
#[template(path = "user/job.html.jinja")]
pub struct JobPageTemplate {
    pub username: String,
    pub is_admin: bool,
    pub active_page: String,
    pub job_id: String,
    pub lab_name: String,
    pub action: String,
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
    use std::fs;
    use std::path::{Path, PathBuf};

    use askama::Template;
    use axum::http::header::CONTENT_TYPE;
    use shared::data::{NodeKind, NodeModel, NodeState};

    use super::*;

    /// Every template file under `dir`.
    fn template_files(dir: &Path) -> Vec<PathBuf> {
        let mut files = Vec::new();
        for entry in fs::read_dir(dir).expect("read templates dir") {
            let path = entry.expect("dir entry").path();
            if path.is_dir() {
                files.extend(template_files(&path));
            } else if path.extension().is_some_and(|ext| ext == "jinja") {
                files.push(path);
            }
        }
        files
    }

    /// Inline event handlers (`onclick=` and friends) and `<script>` blocks without `src`.
    fn inline_javascript(html: &str) -> Vec<String> {
        let lower = html.to_ascii_lowercase();
        let mut found = Vec::new();
        for (start, _) in lower.match_indices(" on") {
            let name: String = lower[start + 1..]
                .chars()
                .take_while(char::is_ascii_alphabetic)
                .collect();
            if name.len() > 2 && lower[start + 1 + name.len()..].starts_with('=') {
                found.push(name);
            }
        }
        for (start, _) in lower.match_indices("<script") {
            let tag_end = lower[start..]
                .find('>')
                .map_or(lower.len(), |end| start + end);
            if !lower[start..tag_end].contains(" src=") {
                found.push("<script> block".to_owned());
            }
        }
        found
    }

    // ========================================================================
    // Sidebar initial tests
    // ========================================================================

    /// The sidebar avatar's text on the dashboard for `username`.
    fn sidebar_initial(username: &str) -> String {
        let html = DashboardTemplate {
            username: username.to_string(),
            is_admin: false,
            active_page: "dashboard".to_string(),
            lab_count: 0,
            image_count: 0,
            docker_ok: true,
            libvirt_ok: true,
        }
        .render()
        .expect("template should render");
        let marker = r#"<span class="text-xs font-semibold text-accent">"#;
        let start = html.find(marker).expect("sidebar avatar") + marker.len();
        let end = start + html[start..].find("</span>").expect("avatar end");
        html[start..end].to_string()
    }

    #[test]
    fn the_sidebar_shows_the_username_initial_in_capitals() {
        assert_eq!(sidebar_initial("bradmin"), "B");
        assert_eq!(sidebar_initial("Admin"), "A");
    }

    #[test]
    fn the_sidebar_shows_no_initial_for_an_empty_username() {
        assert_eq!(sidebar_initial(""), "");
    }

    // ========================================================================
    // Template render tests
    // ========================================================================

    #[test]
    fn an_error_page_keeps_its_status() {
        let response = Error404Template {
            username: "testuser".to_string(),
            is_admin: false,
            active_page: String::new(),
            message: "Lab not found".to_string(),
        }
        .into_response();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        assert!(
            response.headers()[CONTENT_TYPE]
                .to_str()
                .unwrap()
                .starts_with("text/html")
        );
    }

    #[test]
    fn the_profile_page_renders_its_ssh_key_list_in_place() {
        let html = ProfileTemplate {
            username: "testuser".to_string(),
            is_admin: false,
            active_page: "profile".to_string(),
            ssh_keys_list: SshKeysListTemplate {
                ssh_keys: vec!["ssh-ed25519 AAAAkey testuser".to_string()],
            },
        }
        .render()
        .expect("template should render");
        assert!(html.contains(r#"<div id="ssh-keys-list">"#));
        assert!(html.contains("ssh-ed25519 AAAAkey testuser"));
    }

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

    fn test_device() -> DeviceInfo {
        DeviceInfo {
            name: "dev01".to_string(),
            model: NodeModel::UbuntuLinux,
            kind: NodeKind::VirtualMachine,
            state: NodeState::Running,
            mgmt_ipv4: "172.31.0.11".to_string(),
            mgmt_ipv6: None,
            vnc_port: None,
            disks: vec![],
            cpu_count: None,
            memory: None,
        }
    }

    #[test]
    fn test_nodes_table_shows_node_name_with_lab_id() {
        let tpl = NodesTableFragment {
            lab_id: "a1b2c3d4".to_string(),
            devices: vec![test_device()],
            device_count: 1,
        };
        let html = tpl.render().expect("template should render");
        assert!(html.contains("dev01.a1b2c3d4"));
        assert!(html.contains(r#"href="/labs/a1b2c3d4/nodes/dev01""#));
    }

    #[test]
    fn test_node_detail_heading_shows_node_name_with_lab_id() {
        let tpl = NodeDetailTemplate {
            username: "testuser".to_string(),
            is_admin: false,
            active_page: "labs".to_string(),
            lab_id: "a1b2c3d4".to_string(),
            lab_name: "test-lab".to_string(),
            device: test_device(),
            node_config: NodeConfig::get_model(NodeModel::UbuntuLinux),
            mgmt_mac: None,
        };
        let html = tpl.render().expect("template should render");
        assert!(
            html.contains(r#"<h1 class="text-2xl font-semibold text-heading">dev01.a1b2c3d4</h1>"#)
        );
    }

    fn node_detail(device: DeviceInfo, model: NodeModel) -> String {
        NodeDetailTemplate {
            username: "testuser".to_string(),
            is_admin: false,
            active_page: "labs".to_string(),
            lab_id: "a1b2c3d4".to_string(),
            lab_name: "test-lab".to_string(),
            device,
            node_config: NodeConfig::get_model(model),
            mgmt_mac: None,
        }
        .render()
        .expect("template should render")
    }

    fn compute_value(html: &str, label: &str) -> String {
        let after = &html[html.find(label).expect(label)..];
        let start = after.find(r#"font-mono">"#).expect("value") + r#"font-mono">"#.len();
        after[start..start + after[start..].find('<').expect("end")]
            .trim()
            .to_string()
    }

    #[test]
    fn test_node_detail_shows_the_nodes_cpu_and_memory_not_the_image_defaults() {
        let device = DeviceInfo {
            cpu_count: Some(16),
            memory: Some(32768),
            ..test_device()
        };
        let html = node_detail(device, NodeModel::UbuntuLinux);
        assert_eq!(compute_value(&html, "CPU Count"), "16");
        assert_eq!(compute_value(&html, "Memory (MB)"), "32768");
    }

    #[test]
    fn test_node_detail_shows_containers_as_not_limited() {
        let device = DeviceInfo {
            model: NodeModel::MongoDb,
            kind: NodeKind::Container,
            ..test_device()
        };
        let html = node_detail(device, NodeModel::MongoDb);
        assert_eq!(compute_value(&html, "CPU Count"), "Not limited");
        assert_eq!(compute_value(&html, "Memory (MB)"), "Not limited");
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
                description: Some("Practice <BGP> routing".to_string()),
                status: LabState::Unknown,
                node_count: 2,
            }],
        };
        let html = tpl.render().expect("template should render");
        assert!(html.contains("test-lab"));
        assert!(html.contains("a10736e8"));
        // Shown under the name, escaped.
        assert!(html.contains("Practice &#60;BGP&#62; routing"), "{html}");
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
    fn test_lab_create_required_fields_sit_in_build_panel() {
        let tpl = LabCreateTemplate {
            username: "testuser".to_string(),
            is_admin: false,
            active_page: "labs".to_string(),
            generated_name: "happy-panda".to_string(),
            models: vec!["cisco_iosv".to_string()],
        };
        let html = tpl.render().expect("template should render");
        let build = html.find("id=\"panel-build\"").expect("build panel");
        let upload = html.find("id=\"panel-upload\"").expect("upload panel");
        let name = html.find("id=\"lab-name\"").expect("lab name input");
        assert!(
            build < name && name < upload,
            "lab name must be in the build panel"
        );
        // The upload panel must hold no required field, and the script must
        // disable the inactive panel's fields so they cannot block submit.
        let upload_end = html[upload..]
            .find("<!-- Submit -->")
            .expect("submit marker");
        assert!(!html[upload..upload + upload_end].contains("required"));
        let js = include_str!("../web/static/js/lab-create.js");
        assert!(js.contains("fields[j].disabled = tab !== activeTab"));
    }

    #[test]
    fn test_job_page_template_renders() {
        let tpl = JobPageTemplate {
            username: "testuser".to_string(),
            is_admin: false,
            active_page: "labs".to_string(),
            job_id: "abc-123".to_string(),
            lab_name: "test-lab".to_string(),
            action: "Creating".to_string(),
        };
        let html = tpl.render().expect("template should render");
        assert!(html.contains("abc-123"));
        assert!(html.contains("test-lab"));
        assert!(html.contains("Creating"));
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
                description: Some("Practice BGP routing".to_string()),
                owner_username: "alice".to_string(),
                node_count: 3,
            }],
        };
        let html = tpl.render().expect("template should render");
        assert!(html.contains("my-lab"));
        assert!(html.contains("Practice BGP routing"));
        assert!(html.contains("abc12345"));
        assert!(html.contains("alice"));
    }

    #[test]
    fn test_login_page_template_renders() {
        let tpl = LoginPageTemplate {
            error: String::new(),
            message: String::new(),
            allow_signup: false,
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
    fn test_admin_notification_error_template_escapes_message() {
        let tpl = AdminNotificationErrorTemplate {
            message: "Invalid <script>alert('unsafe')</script>".to_string(),
        };
        let html = tpl.render().expect("template should render");

        assert!(html.contains("Invalid &#60;script&#62;"));
        assert!(!html.contains("<script>"));
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
            active_page: String::new(),
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

    /// Inline JavaScript runs whatever a template interpolates into it, and HTML escaping does
    /// not stop that: the browser decodes `&#x27;` back to `'` before the handler runs. All
    /// behaviour lives in `web/static/js` and reads its arguments from `data-` attributes.
    #[test]
    fn templates_carry_no_inline_javascript() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("templates");
        let offenders: Vec<String> = template_files(&dir)
            .into_iter()
            .flat_map(|path| {
                let html = fs::read_to_string(&path).expect("read template");
                let name = path
                    .strip_prefix(&dir)
                    .unwrap_or(&path)
                    .display()
                    .to_string();
                inline_javascript(&html)
                    .into_iter()
                    .map(move |found| format!("{name}: {found}"))
            })
            .collect();
        assert!(offenders.is_empty(), "inline JavaScript: {offenders:#?}");
    }

    #[test]
    fn the_inline_javascript_check_finds_handlers_and_script_blocks() {
        assert_eq!(
            inline_javascript(r#"<button onclick="go()">x</button><script>run()</script>"#),
            vec!["onclick".to_owned(), "<script> block".to_owned()]
        );
        assert_eq!(
            inline_javascript(
                r#"<a href="/x" data-lab-id="on=1"><script src="/js/a.js"></script>"#
            ),
            [] as [std::string::String; 0]
        );
    }
}
