use askama::Template;
use askama_web::WebTemplate;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use shared::data::{
    BridgeInfo, DbUser, DestroyError, DeviceInfo, LabInfo, LabPhase, LabSummary, LinkInfo,
    NodeConfig, NodeKind, Ztp,
};

use crate::api::handlers::{ImageSummary, UserSummary};

mod actions;
mod badge;
mod kind;
mod login;
mod nav;

pub use actions::{LabActions, NodeActions};
pub use badge::{Badge, BadgeSize, BadgeTone};
pub use kind::{KindBadge, ModelChoice, kind_choices, kind_label};
pub use login::LoginError;
pub use nav::{Nav, NavLink, NavPage};

/// Main dashboard page template
#[derive(Template, WebTemplate)]
#[template(path = "user/dashboard.html.jinja")]
pub struct DashboardTemplate {
    pub username: String,
    pub is_admin: bool,
    pub nav: Nav,
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
    pub nav: Nav,
}

/// Labs grid partial template - displays all labs
#[derive(Template, WebTemplate)]
#[template(path = "user/partials/labs-grid.html.jinja")]
pub struct LabsGridTemplate {
    pub labs: Vec<LabRow>,
}

/// One lab in the labs grid.
pub struct LabRow {
    pub lab: LabSummary,
    /// The lab's phase until it is ready, then its state.
    pub status_badge: Badge,
    pub actions: LabActions,
}

impl From<LabSummary> for LabRow {
    fn from(lab: LabSummary) -> Self {
        Self {
            status_badge: Badge::for_lab(lab.phase, lab.status, BadgeSize::Small),
            actions: LabActions::for_lab(lab.phase, lab.status),
            lab,
        }
    }
}

/// One node in a lab's nodes table.
pub struct NodeRow {
    pub device: DeviceInfo,
    pub state_badge: Badge,
    pub actions: NodeActions,
}

impl From<DeviceInfo> for NodeRow {
    fn from(device: DeviceInfo) -> Self {
        Self {
            state_badge: Badge::for_node_state(device.state, BadgeSize::Small),
            actions: NodeActions::for_state(device.state),
            device,
        }
    }
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
    pub error: Option<LoginError>,
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
    pub nav: Nav,
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
    pub nav: Nav,
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
    pub nav: Nav,
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
    pub nav: Nav,
    pub lab_id: String,
    pub lab_info: LabInfo,
    /// The lab's phase until it is ready, then its state.
    pub state_badge: Badge,
    pub actions: LabActions,
    /// Set by the handler: only the lab's owner is offered the files, and a lab with no nodes,
    /// or an unknown state, has none to download.
    pub offers_download: bool,
    pub nodes: Vec<NodeRow>,
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
    pub nav: Nav,
    pub lab_id: String,
    pub lab_name: String,
    pub device: DeviceInfo,
    pub state_badge: Badge,
    pub actions: NodeActions,
    pub node_config: NodeConfig,
    /// The ZTP setting the node runs with: its manifest's, or else its image's.
    pub ztp: Ztp,
    pub mgmt_mac: Option<String>,
}

impl NodeDetailTemplate {
    /// The node's vCPUs, or a container's CPU quota; "Not limited" for a container without one.
    pub fn cpu_display(&self) -> String {
        resource_display(self.device.cpu_count.map(u16::from), self.device.kind)
    }

    /// The node's memory in MB.
    pub fn memory_display(&self) -> String {
        resource_display(self.device.memory, self.device.kind)
    }
}

fn resource_display(value: Option<u16>, kind: NodeKind) -> String {
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
    pub nav: Nav,
    /// Rendered in place, and on its own for HTMX swaps.
    pub ssh_keys_list: SshKeysListTemplate,
    /// Whether the user has a device password, rendered in place.
    pub device_password_status: DevicePasswordStatusTemplate,
}

/// Whether the user has a device password: `set` or `not set`. Rendered in the profile page, and
/// out of band after the password is set, so the page shows the new state.
#[derive(Template, WebTemplate)]
#[template(path = "user/partials/device-password-status.html.jinja")]
pub struct DevicePasswordStatusTemplate {
    pub set: bool,
    pub oob: bool,
}

/// The device password was set: a message, and the status swapped in out of band.
#[derive(Template, WebTemplate)]
#[template(path = "user/partials/device-password-success.html.jinja")]
pub struct DevicePasswordSuccessTemplate {
    pub status: DevicePasswordStatusTemplate,
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
    pub nav: Nav,
    pub users: Vec<UserSummary>,
}

/// Admin labs list page template
#[derive(Template, WebTemplate)]
#[template(path = "admin/labs.html.jinja")]
pub struct AdminLabsTemplate {
    pub username: String,
    pub is_admin: bool,
    pub nav: Nav,
    pub labs: Vec<AdminLabSummary>,
}

/// Summary data for a lab displayed in the admin labs list
pub struct AdminLabSummary {
    pub lab_id: String,
    pub name: String,
    pub description: Option<String>,
    pub owner_username: String,
    pub node_count: usize,
    /// Which operation owns the lab.
    pub phase_badge: Badge,
    /// Only a ready lab can be started or stopped; any other can only be destroyed or cleaned.
    pub offers_start_stop: bool,
}

impl AdminLabSummary {
    /// The summary of a lab in `phase`.
    pub fn new(
        lab_id: String,
        name: String,
        description: Option<String>,
        owner_username: String,
        node_count: usize,
        phase: LabPhase,
    ) -> Self {
        Self {
            lab_id,
            name,
            description,
            owner_username,
            node_count,
            phase_badge: Badge::for_phase(phase, BadgeSize::Small),
            offers_start_stop: phase.is_ready(),
        }
    }
}

/// Admin user edit page template
#[derive(Template, WebTemplate)]
#[template(path = "admin/user-edit.html.jinja")]
pub struct AdminUserEditTemplate {
    pub username: String,
    pub is_admin: bool,
    pub nav: Nav,
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
    pub nav: Nav,
    pub configs: Vec<ImageSummary>,
}

/// Admin image detail page template
#[derive(Template, WebTemplate)]
#[template(path = "admin/image-detail.html.jinja")]
pub struct AdminImageDetailTemplate {
    pub username: String,
    pub is_admin: bool,
    pub nav: Nav,
    pub kind_badge: KindBadge,
    pub config: NodeConfig,
}

/// One option of a form's list, built by the handler: the value posted, the text shown, and
/// whether it is the current one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FormChoice {
    pub value: String,
    pub label: String,
    pub selected: bool,
}

/// Admin image edit page template
#[derive(Template, WebTemplate)]
#[template(path = "admin/image-edit.html.jinja")]
pub struct AdminImageEditTemplate {
    pub username: String,
    pub is_admin: bool,
    pub nav: Nav,
    pub config: NodeConfig,
    pub kind_label: &'static str,
    /// A container image's repo, blank when it has none; `None` for other kinds.
    pub repo: Option<String>,
    /// The `vm` group's fields, for a virtual machine image.
    pub vm: Option<VmEditChoices>,
    /// The CPU, machine and interface device fields of a virtual machine or unikernel image.
    pub device: Option<DeviceEditChoices>,
    /// The ZTP settings the image's model takes, the image's own selected; none for a
    /// unikernel, which has no ZTP setting.
    pub ztp_choices: Vec<FormChoice>,
}

/// The edit form's fields only a virtual machine image has.
pub struct VmEditChoices {
    pub os_variant_choices: Vec<FormChoice>,
    pub bios_choices: Vec<FormChoice>,
    pub hdd_bus_choices: Vec<FormChoice>,
    pub cdrom_bus_choices: Vec<FormChoice>,
    /// The CD-ROM image's file name, blank when it has none.
    pub cdrom: String,
    pub vmx: bool,
}

/// The edit form's CPU, machine and interface device fields, which virtual machine and
/// unikernel images have.
pub struct DeviceEditChoices {
    pub cpu_architecture_choices: Vec<FormChoice>,
    pub cpu_model_choices: Vec<FormChoice>,
    pub machine_type_choices: Vec<FormChoice>,
    pub interface_type_choices: Vec<FormChoice>,
    pub interface_mtu: u16,
}

/// Admin image versions list page template
#[derive(Template, WebTemplate)]
#[template(path = "admin/image-versions.html.jinja")]
pub struct AdminImageVersionsTemplate {
    pub username: String,
    pub is_admin: bool,
    pub nav: Nav,
    pub model: String,
    pub kind: NodeKind,
    pub kind_label: &'static str,
    pub versions: Vec<NodeConfig>,
}

/// Admin image upload page template
#[derive(Template, WebTemplate)]
#[template(path = "admin/image-upload.html.jinja")]
pub struct AdminImageUploadTemplate {
    pub username: String,
    pub is_admin: bool,
    pub nav: Nav,
    pub models: Vec<ModelChoice>,
    /// Every kind; the page's script offers those of the chosen model.
    pub kinds: Vec<FormChoice>,
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
    pub nav: Nav,
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
    pub nav: Nav,
    pub generated_name: String,
    pub models: Vec<ModelChoice>,
    /// Every kind; the page's script offers those of each node's model.
    pub kinds: Vec<FormChoice>,
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
    pub nodes: Vec<NodeRow>,
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
    pub nav: Nav,
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
    use shared::data::{LabState, ModelKind, NodeKind, NodeModel, NodeState};

    use super::*;
    use crate::api::handlers::lab_offers_download;

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
            nav: NavPage::Dashboard.into(),
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
            nav: NavPage::Other.into(),
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
    fn a_set_device_password_shows_set_and_swaps_the_status_out_of_band() {
        let html = DevicePasswordSuccessTemplate {
            status: DevicePasswordStatusTemplate {
                set: true,
                oob: true,
            },
        }
        .render()
        .expect("template should render");
        assert!(html.contains("Device password set"));
        assert!(html.contains(r#"<span id="device-password-status" hx-swap-oob="true""#));
        assert!(html.contains(">set</span>"));
    }

    fn test_device() -> DeviceInfo {
        DeviceInfo {
            name: "dev01".to_string(),
            model: NodeModel::UbuntuLinux,
            kind: NodeKind::VirtualMachine,
            state: NodeState::Running,
            mgmt_ipv4: "172.31.0.11".to_string(),
            mgmt_ipv6: None,
            disks: vec![],
            cpu_count: None,
            memory: None,
        }
    }

    #[test]
    fn test_nodes_table_shows_node_name_with_lab_id() {
        let tpl = NodesTableFragment {
            lab_id: "a1b2c3d4".to_string(),
            nodes: vec![NodeRow::from(test_device())],
            device_count: 1,
        };
        let html = tpl.render().expect("template should render");
        assert!(html.contains("dev01.a1b2c3d4"));
        assert!(html.contains(r#"href="/labs/a1b2c3d4/nodes/dev01""#));
    }

    fn node_detail(device: DeviceInfo, model: NodeModel) -> String {
        NodeDetailTemplate {
            username: "testuser".to_string(),
            is_admin: false,
            nav: NavPage::Labs.into(),
            lab_id: "a1b2c3d4".to_string(),
            lab_name: "test-lab".to_string(),
            state_badge: Badge::for_node_state(device.state, BadgeSize::Large),
            actions: NodeActions::for_state(device.state),
            device,
            node_config: ModelKind::default_for(model).node_config(),
            ztp: ModelKind::default_for(model).node_config().ztp(),
            mgmt_mac: None,
        }
        .render()
        .expect("template should render")
    }

    #[test]
    fn node_state_020_the_web_ui_offers_a_node_only_the_actions_its_state_allows() {
        // M1.1 and M4.2: a node being built offers nothing, and a missing one only redeploy.
        let cases = [
            (NodeState::Running, ["stop", "redeploy"].as_slice()),
            (NodeState::Starting, &["stop", "redeploy"]),
            (NodeState::Stopped, &["start", "redeploy"]),
            (NodeState::Failed, &["start", "redeploy"]),
            (NodeState::Missing, &["redeploy"]),
            (NodeState::Building, &[]),
            (NodeState::Unknown, &["start", "redeploy"]),
        ];
        for (state, offered) in cases {
            let device = DeviceInfo {
                state,
                ..test_device()
            };
            let table = NodesTableFragment {
                lab_id: "a1b2c3d4".to_string(),
                nodes: vec![NodeRow::from(device.clone())],
                device_count: 1,
            }
            .render()
            .expect("template should render");
            let detail = node_detail(device, NodeModel::UbuntuLinux);
            for html in [table, detail] {
                for action in ["start", "stop", "redeploy"] {
                    let attribute = format!(r#"data-node-action="{action}""#);
                    assert_eq!(
                        html.contains(&attribute),
                        offered.contains(&action),
                        "{action} for a {state} node"
                    );
                }
            }
        }
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
    fn test_labs_grid_template_renders_with_labs() {
        let tpl = LabsGridTemplate {
            labs: vec![LabRow::from(LabSummary {
                id: "a10736e8".to_string(),
                name: "test-lab".to_string(),
                description: Some("Practice <BGP> routing".to_string()),
                status: LabState::Unknown,
                node_count: 2,
                phase: LabPhase::Ready,
            })],
        };
        let html = tpl.render().expect("template should render");
        assert!(html.contains("test-lab"));
        assert!(html.contains("a10736e8"));
        // Shown under the name, escaped.
        assert!(html.contains("Practice &#60;BGP&#62; routing"), "{html}");
    }

    #[test]
    fn a_lab_that_is_not_ready_shows_its_phase_and_offers_no_start_or_stop() {
        // M7.3: a lab whose destroy failed used to show `stopped` and offer Start.
        for (phase, badge) in [
            (
                LabPhase::Creating,
                r#"<span class="badge-warning">Creating</span>"#,
            ),
            (
                LabPhase::Destroying,
                r#"<span class="badge-warning">Destroying</span>"#,
            ),
            (
                LabPhase::DestroyFailed,
                r#"<span class="badge-danger">Destroy failed</span>"#,
            ),
            (
                LabPhase::Interrupted,
                r#"<span class="badge-danger">Interrupted</span>"#,
            ),
            (
                LabPhase::Unknown,
                r#"<span class="badge-danger">Unknown</span>"#,
            ),
        ] {
            let tpl = LabsGridTemplate {
                labs: vec![LabRow::from(LabSummary {
                    id: "a10736e8".to_string(),
                    name: "test-lab".to_string(),
                    description: None,
                    status: LabState::Stopped,
                    node_count: 2,
                    phase,
                })],
            };
            let html = tpl.render().expect("template should render");
            assert!(!html.contains("data-lab-action"), "{phase:?}: {html}");
            assert!(html.contains(badge), "{phase:?}: {html}");
            assert!(!html.contains(">Stopped<"), "{phase:?}: {html}");
        }
    }

    #[test]
    fn the_lab_create_page_gives_each_model_its_kinds_and_default_and_offers_every_kind() {
        let html = LabCreateTemplate {
            username: "testuser".to_string(),
            is_admin: false,
            nav: NavPage::Labs.into(),
            generated_name: "happy-panda".to_string(),
            models: vec![NodeModel::AristaEos.into(), NodeModel::Generic.into()],
            kinds: kind_choices(),
        }
        .render()
        .expect("template should render");
        assert!(
            html.contains(
                r#"<option value="arista_eos" data-kinds="virtual_machine,container" data-default-kind="virtual_machine">"#
            ),
            "{html}"
        );
        assert!(
            html.contains(
                r#"<option value="generic" data-kinds="virtual_machine,container,unikernel" data-default-kind="">"#
            ),
            "{html}"
        );
        assert!(html.contains(r#"data-field="node-kind""#), "{html}");
        for (value, label) in [
            ("virtual_machine", "Virtual Machine"),
            ("container", "Container"),
            ("unikernel", "Unikernel"),
        ] {
            assert!(
                html.contains(&format!(r#"<option value="{value}">{label}</option>"#)),
                "{value}: {html}"
            );
        }
    }

    #[test]
    fn image_import_019_the_upload_page_offers_each_model_its_kinds_with_its_default_chosen() {
        let html = AdminImageUploadTemplate {
            username: "admin".to_string(),
            is_admin: true,
            nav: NavPage::AdminImages.into(),
            models: vec![NodeModel::AristaEos.into(), NodeModel::Generic.into()],
            kinds: kind_choices(),
        }
        .render()
        .expect("template should render");
        assert!(
            html.contains(
                r#"<li data-value="arista_eos" data-kinds="virtual_machine,container" data-default-kind="virtual_machine""#
            ),
            "{html}"
        );
        assert!(
            html.contains(
                r#"<li data-value="generic" data-kinds="virtual_machine,container,unikernel" data-default-kind="""#
            ),
            "{html}"
        );
        assert!(
            html.contains(r#"<select id="kind" name="kind" required"#),
            "{html}"
        );
        assert!(
            html.contains(r#"<option value="container">Container</option>"#),
            "{html}"
        );
    }

    #[test]
    fn test_lab_create_required_fields_sit_in_build_panel() {
        let tpl = LabCreateTemplate {
            username: "testuser".to_string(),
            is_admin: false,
            nav: NavPage::Labs.into(),
            generated_name: "happy-panda".to_string(),
            models: vec![NodeModel::CiscoIos.into()],
            kinds: kind_choices(),
        };
        let html = tpl.render().expect("template should render");
        let build = html.find(r#"id="panel-build""#).expect("build panel");
        let upload = html.find(r#"id="panel-upload""#).expect("upload panel");
        let name = html.find(r#"id="lab-name""#).expect("lab name input");
        assert!(
            build < name && name < upload,
            "lab name must be in the build panel"
        );
        // The upload panel must hold no required field, so it cannot block submit.
        let upload_end = html[upload..]
            .find("<!-- Submit -->")
            .expect("submit marker");
        assert!(!html[upload..upload + upload_end].contains("required"));
    }

    #[test]
    fn the_admin_list_shows_a_stranded_labs_phase_and_offers_no_start_or_stop() {
        // M6.2, M6.3 and M7.3: admins find the labs left behind, to clean them.
        let tpl = AdminLabsTemplate {
            username: "admin".to_string(),
            is_admin: true,
            nav: NavPage::AdminLabs.into(),
            labs: vec![AdminLabSummary::new(
                "abc12345".to_string(),
                "my-lab".to_string(),
                None,
                "alice".to_string(),
                3,
                LabPhase::Interrupted,
            )],
        };
        let html = tpl.render().expect("template should render");
        assert!(
            html.contains(r#"<span class="badge-danger">Interrupted</span>"#),
            "{html}"
        );
        assert!(!html.contains("data-lab-action"), "{html}");
    }

    fn lab_detail(lab_state: LabState, lab_phase: LabPhase) -> String {
        let lab_info: LabInfo = "id='abcd1234'\nname='test-lab'\nuser='alice'\n\
            ipv4_network='192.0.2.0/24'\nipv4_gateway='192.0.2.1'\n\
            ipv4_router='192.0.2.2'"
            .parse()
            .expect("lab info");
        LabDetailTemplate {
            username: "alice".to_string(),
            is_admin: false,
            nav: NavPage::Labs.into(),
            lab_id: "abcd1234".to_string(),
            lab_info,
            state_badge: Badge::for_lab(lab_phase, lab_state, BadgeSize::Large),
            actions: LabActions::for_lab(lab_phase, lab_state),
            offers_download: lab_offers_download(true, lab_state),
            nodes: vec![],
            device_count: 0,
            inactive_devices: vec![],
            inactive_device_count: 0,
            links: vec![],
            link_count: 0,
            bridges: vec![],
            bridge_count: 0,
        }
        .render()
        .expect("template should render")
    }

    #[test]
    fn the_lab_detail_header_shows_a_large_phase_badge_while_the_lab_is_not_ready() {
        for (phase, badge) in [
            (
                LabPhase::Creating,
                r#"<span class="badge-warning-lg">Creating</span>"#,
            ),
            (
                LabPhase::Destroying,
                r#"<span class="badge-warning-lg">Destroying</span>"#,
            ),
            (
                LabPhase::DestroyFailed,
                r#"<span class="badge-danger-lg">Destroy failed</span>"#,
            ),
            (
                LabPhase::Interrupted,
                r#"<span class="badge-danger-lg">Interrupted</span>"#,
            ),
            (
                LabPhase::Unknown,
                r#"<span class="badge-danger-lg">Unknown</span>"#,
            ),
        ] {
            let html = lab_detail(LabState::Stopped, phase);
            assert!(html.contains(badge), "{phase:?}: {html}");
            assert!(!html.contains(">Stopped<"), "{phase:?}: {html}");
        }
    }

    #[test]
    fn the_lab_detail_offers_no_start_or_stop_while_the_lab_is_not_ready() {
        for phase in [
            LabPhase::Creating,
            LabPhase::Destroying,
            LabPhase::DestroyFailed,
            LabPhase::Interrupted,
            LabPhase::Unknown,
        ] {
            for state in [LabState::Running, LabState::Partial, LabState::Stopped] {
                let html = lab_detail(state, phase);
                assert!(
                    !html.contains("data-lab-detail-action"),
                    "{phase:?}: {html}"
                );
                assert!(html.contains("Destroy Lab"), "{phase:?}: {html}");
            }
        }
    }

    #[test]
    fn the_lab_detail_offers_the_download_only_for_a_lab_with_known_nodes() {
        let download = r#"href="/labs/abcd1234/download""#;
        for state in [LabState::Unknown, LabState::Empty] {
            let html = lab_detail(state, LabPhase::Ready);
            assert!(!html.contains(download), "{state:?}: {html}");
        }
        let html = lab_detail(LabState::Running, LabPhase::Ready);
        assert!(html.contains(download), "{html}");
    }

    #[test]
    fn the_lab_detail_header_shows_the_lab_state_once_the_lab_is_ready() {
        let html = lab_detail(LabState::Running, LabPhase::Ready);
        assert!(html.contains(">Running<"), "{html}");
        assert!(!html.contains(">Ready<"), "{html}");
    }

    /// The labels of the image detail page's machine type, and of its firmware and disks.
    fn image_detail(config: NodeConfig) -> String {
        AdminImageDetailTemplate {
            username: "admin".to_string(),
            is_admin: true,
            nav: NavPage::AdminImages.into(),
            kind_badge: KindBadge::full(config.kind()),
            config,
        }
        .render()
        .expect("template should render")
    }

    #[test]
    fn image_settings_011_the_image_page_shows_each_kind_only_the_settings_it_takes() {
        const VM: &[NodeKind] = &[NodeKind::VirtualMachine];
        const VM_AND_UNIKERNEL: &[NodeKind] = &[NodeKind::VirtualMachine, NodeKind::Unikernel];
        const ALL: &[NodeKind] = &[
            NodeKind::VirtualMachine,
            NodeKind::Container,
            NodeKind::Unikernel,
        ];
        for model in [
            NodeModel::CiscoCat8000,
            NodeModel::UbuntuLinux,
            NodeModel::NanosUnikernel,
            NodeModel::NokiaSrlinux,
            NodeModel::MongoDb,
        ] {
            let config = ModelKind::default_for(model).node_config();
            let kind = config.kind();
            let html = image_detail(config);
            for (label, kinds) in [
                ("OS Variant:", VM),
                ("BIOS:", VM),
                ("HDD Bus:", VM),
                (">CD-ROM Bus:</span>", VM),
                ("Storage Configuration", VM),
                ("Repository:", &[NodeKind::Container]),
                ("Boot Mode:", &[NodeKind::Unikernel]),
                (">Machine Type:</span>", VM_AND_UNIKERNEL),
                ("Interface MTU:", VM_AND_UNIKERNEL),
                ("ZTP:", &[NodeKind::VirtualMachine, NodeKind::Container]),
                ("Data Interfaces:", ALL),
            ] {
                assert_eq!(
                    html.contains(label),
                    kinds.contains(&kind),
                    "{model} {label}"
                );
            }
        }
    }

    #[test]
    fn the_node_page_shows_each_kind_the_machine_and_disk_settings_it_uses() {
        let firmware_and_disks = [
            ">BIOS</span>",
            ">HDD Bus</span>",
            ">CDROM</span>",
            ">CDROM Bus</span>",
        ];
        for (model, kind, machine_type, has_firmware_and_disks) in [
            (NodeModel::UbuntuLinux, NodeKind::VirtualMachine, true, true),
            (NodeModel::NanosUnikernel, NodeKind::Unikernel, true, false),
            (NodeModel::NokiaSrlinux, NodeKind::Container, false, false),
        ] {
            let device = DeviceInfo {
                model,
                kind,
                ..test_device()
            };
            let html = node_detail(device, model);
            assert_eq!(
                html.contains(">Machine Type</span>"),
                machine_type,
                "{model}"
            );
            for label in firmware_and_disks {
                assert_eq!(
                    html.contains(label),
                    has_firmware_and_disks,
                    "{model} {label}"
                );
            }
            // The node's own disks are listed for every kind.
            assert!(html.contains(">Disks</span>"), "{model}");
        }
    }

    #[test]
    fn the_login_page_shows_the_notice_for_its_error_code() {
        let notice = r#"<div class="bg-alert-error"#;
        for (code, text) in [
            ("session_required", "Please sign in to continue"),
            (
                "session_expired",
                "Your session has expired. Please sign in again.",
            ),
            ("logout_success", "You have been logged out successfully"),
            ("<b>odd</b>", "&#60;b&#62;odd&#60;/b&#62;"),
        ] {
            let html = LoginPageTemplate {
                error: LoginError::from_code(code),
                message: String::new(),
                allow_signup: false,
            }
            .render()
            .expect("template should render");
            assert!(html.contains(notice), "{code}: {html}");
            assert!(html.contains(text), "{code}: {html}");
        }
        let html = LoginPageTemplate {
            error: LoginError::from_code(""),
            message: String::new(),
            allow_signup: false,
        }
        .render()
        .expect("template should render");
        assert!(!html.contains(notice), "{html}");
    }

    #[test]
    fn the_sidebar_highlights_the_current_pages_link_alone() {
        let active = "transition-colors bg-accent/15 text-accent";
        for (page, href) in [
            (NavPage::Dashboard, Some("/")),
            (NavPage::Labs, Some("/labs")),
            (NavPage::AdminImages, Some("/admin/images")),
            (NavPage::AdminLabs, Some("/admin/labs")),
            (NavPage::AdminUsers, Some("/admin/users")),
            (NavPage::AdminTools, Some("/admin/tools")),
            (NavPage::Other, None),
        ] {
            let html = DashboardTemplate {
                username: "admin".to_string(),
                is_admin: true,
                nav: page.into(),
                lab_count: 0,
                image_count: 0,
                docker_ok: true,
                libvirt_ok: true,
            }
            .render()
            .expect("template should render");
            let highlighted: Vec<&str> = html
                .match_indices(active)
                .map(|(at, _)| {
                    let link = &html[html[..at].rfind(r#"<a href=""#).expect("link") + 9..];
                    &link[..link.find('"').expect("href end")]
                })
                .collect();
            assert_eq!(
                highlighted,
                href.into_iter().collect::<Vec<_>>(),
                "{page:?}"
            );
        }
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

    // ========================================================================
    // Sidebar rendering — active page highlighting
    // ========================================================================

    #[test]
    fn test_sidebar_shows_admin_section_for_admins() {
        let tpl = DashboardTemplate {
            username: "admin".to_string(),
            is_admin: true,
            nav: NavPage::Dashboard.into(),
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
            nav: NavPage::Dashboard.into(),
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

    /// Template tags that decide from an enum's text: a `to_string()` call, or a comparison
    /// with a non-empty string literal.
    fn text_decisions(html: &str) -> Vec<String> {
        html.split("{%")
            .skip(1)
            .filter_map(|tag| tag.split_once("%}").map(|(tag, _)| tag.trim()))
            .filter(|tag| {
                tag.contains("to_string()")
                    || [r#"== ""#, r#"!= ""#].iter().any(|op| {
                        tag.match_indices(op)
                            .any(|(at, _)| !tag[at + op.len()..].starts_with('"'))
                    })
            })
            .map(str::to_owned)
            .collect()
    }

    #[test]
    fn templates_decide_nothing_from_an_enums_text() {
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
                text_decisions(&html)
                    .into_iter()
                    .map(move |found| format!("{name}: {found}"))
            })
            .collect();
        assert!(offenders.is_empty(), "text decisions: {offenders:#?}");
    }

    #[test]
    fn the_text_decision_check_finds_to_string_and_literal_comparisons() {
        assert_eq!(
            text_decisions(
                r#"{% if state.to_string() == "running" %}{% elif kind == "vm" %}{% if ip != "" %}"#
            ),
            vec![
                r#"if state.to_string() == "running""#.to_owned(),
                r#"elif kind == "vm""#.to_owned()
            ]
        );
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
