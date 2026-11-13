//! The web UI's lab pages, lab and node actions, jobs and lab creation.

use super::labs_api::run_detached;
use anyhow::Context;
use axum::extract::{Path, State};
use axum::http::{StatusCode, header};
use axum::response::{Html, IntoResponse, Response, sse};
use axum::{Form, Json};
use serde_json::json;
use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use strum::IntoEnumIterator;

use crate::api::sse::{destroy_progress_stream, up_progress_stream};
use crate::auth::policy::{require_lab_access, require_lab_owner, require_lab_starter};
use crate::auth::session::SessionLookup;
use crate::daemon::state::AppState;
use crate::daemon::state::{Job, JobType};
use crate::services::error::ServiceError;
use crate::services::progress::ProgressSender;
use crate::services::{destroy, down, download, inspect, list_labs, redeploy, resume, up};
use crate::templates::{
    Badge, BadgeSize, DashboardTemplate, EmptyStateTemplate, Error403Template, Error404Template,
    ErrorTemplate, JobPageTemplate, LabActions, LabCreateTemplate, LabDestroyButtonFragment,
    LabDestroyConfirmFragment, LabDetailTemplate, LabRow, LabsGridTemplate, LabsListTemplate,
    NavPage, NodeActions, NodeDetailTemplate, NodeRow, NodesTableFragment,
};

use crate::api::errors::ApiError;
use crate::api::extractors::AuthenticatedUserFromCookie;

use shared::data::{LabNodeActionResponse, LabState, NodeModel, RedeployRequest, UpRequest, Ztp};
use shared::konst::{
    LAB_FILE_NAME, SHERPA_KNOWN_HOSTS_FILE, SHERPA_LABS_PATH, SHERPA_SSH_CONFIG_FILE,
    SHERPA_SSH_PRIVATE_KEY_FILE,
};
use shared::util::{generate_lab_name, get_id_for_user, load_saved_manifest};
use shared::validators::effective_ztp;
use topology::Manifest;

/// Dashboard page handler (HTML)
///
/// Serves the main dashboard HTML page with HTMX support.
/// The page will use HTMX to dynamically load labs.
///
/// Requires authentication via cookie.
///
/// # Response (200 OK)
/// Returns HTML page rendered from dashboard template
///
pub async fn dashboard_handler(
    State(state): State<AppState>,
    auth: AuthenticatedUserFromCookie,
) -> Result<DashboardTemplate, ApiError> {
    tracing::debug!("Serving dashboard for user '{}'", auth.username);

    // Get user's lab count
    let lab_count = match db::get_user(&state.db, &auth.username).await {
        Ok(user) => {
            if let Some(user_id) = user.id {
                db::count_labs_by_user(&state.db, user_id)
                    .await
                    .unwrap_or(0)
            } else {
                0
            }
        }
        Err(_) => 0,
    };

    let image_count = db::count_node_images(&state.db).await.unwrap_or(0);

    // Check system status
    let docker_ok = state.docker.ping().await.is_ok();
    let libvirt_ok = state.qemu.connect().is_ok();

    Ok(DashboardTemplate {
        username: auth.username.clone(),
        is_admin: auth.is_admin,
        nav: NavPage::Dashboard.into(),
        lab_count,
        image_count,
        docker_ok,
        libvirt_ok,
    })
}

/// Labs list full page handler
///
/// GET `/labs/list`
pub async fn labs_list_page_handler(auth: AuthenticatedUserFromCookie) -> impl IntoResponse {
    LabsListTemplate {
        username: auth.username,
        is_admin: auth.is_admin,
        nav: NavPage::Labs.into(),
    }
}

/// Labs grid HTML fragment handler
///
/// Returns an HTML fragment containing the labs grid for HTMX swapping.
/// This endpoint is called by HTMX on page load and periodically for auto-refresh.
///
/// Requires authentication via cookie.
///
/// # Response (200 OK)
/// Returns HTML fragment with labs grid, empty state, or error message
///
pub async fn get_labs_html(
    State(state): State<AppState>,
    auth: AuthenticatedUserFromCookie,
) -> impl IntoResponse {
    tracing::debug!("Fetching labs HTML for user '{}'", auth.username);

    // Call service layer with authenticated username
    match list_labs::list_labs(&auth.context(), None, &state).await {
        Ok(response) => {
            if response.labs.is_empty() {
                tracing::debug!("No labs found for user '{}'", auth.username);
                EmptyStateTemplate {
                    username: auth.username,
                }
                .into_response()
            } else {
                tracing::debug!(
                    "Returning {} labs for user '{}'",
                    response.labs.len(),
                    auth.username
                );
                LabsGridTemplate {
                    labs: response.labs.into_iter().map(LabRow::from).collect(),
                }
                .into_response()
            }
        }
        Err(e) => {
            tracing::error!("Failed to load labs for user '{}': {:?}", auth.username, e);
            let message = if e.to_string().contains("User not found") {
                format!("User '{}' not found", auth.username)
            } else {
                format!("Failed to load labs: {e}")
            };
            ErrorTemplate { message }.into_response()
        }
    }
}

/// Stop all nodes in a lab (web UI handler, cookie auth)
///
/// POST `/labs/{lab_id}/stop`
#[tracing::instrument(skip(state), fields(%lab_id))]
pub async fn lab_stop_handler(
    auth: AuthenticatedUserFromCookie,
    State(state): State<AppState>,
    Path(lab_id): Path<String>,
) -> Result<Json<LabNodeActionResponse>, ApiError> {
    let auth = auth.into_context();
    let response =
        run_detached(async move { down::shutdown_lab_nodes(&auth, &lab_id, None, &state).await })
            .await
            .map_err(ApiError::from)?;

    Ok(Json(response))
}

/// Start all nodes in a lab (web UI handler, cookie auth)
///
/// POST `/labs/{lab_id}/start`
#[tracing::instrument(skip(state), fields(%lab_id))]
pub async fn lab_start_handler(
    auth: AuthenticatedUserFromCookie,
    State(state): State<AppState>,
    Path(lab_id): Path<String>,
) -> Result<Json<LabNodeActionResponse>, ApiError> {
    let auth = auth.into_context();
    let response = run_detached(async move {
        resume::start_lab_nodes(&auth, &lab_id, None, None, &state, None).await
    })
    .await
    .map_err(ApiError::from)?;

    Ok(Json(response))
}

/// Stop a single node in a lab (web UI handler, cookie auth)
///
/// POST `/labs/{lab_id}/nodes/{node_name}/stop`
#[tracing::instrument(skip(state), fields(%lab_id, %node_name))]
pub async fn node_stop_handler(
    auth: AuthenticatedUserFromCookie,
    State(state): State<AppState>,
    Path((lab_id, node_name)): Path<(String, String)>,
) -> Result<Json<LabNodeActionResponse>, ApiError> {
    let auth = auth.into_context();
    let response = run_detached(async move {
        down::shutdown_lab_nodes(&auth, &lab_id, Some(&node_name), &state).await
    })
    .await
    .map_err(ApiError::from)?;

    Ok(Json(response))
}

/// Start a single node in a lab (web UI handler, cookie auth)
///
/// POST `/labs/{lab_id}/nodes/{node_name}/start`
#[tracing::instrument(skip(state), fields(%lab_id, %node_name))]
pub async fn node_start_handler(
    auth: AuthenticatedUserFromCookie,
    State(state): State<AppState>,
    Path((lab_id, node_name)): Path<(String, String)>,
) -> Result<Json<LabNodeActionResponse>, ApiError> {
    let auth = auth.into_context();
    let response = run_detached(async move {
        resume::start_lab_nodes(&auth, &lab_id, Some(&node_name), None, &state, None).await
    })
    .await
    .map_err(ApiError::from)?;

    Ok(Json(response))
}

/// Redeploy a single node in a lab (web UI handler, cookie auth)
///
/// POST `/labs/{lab_id}/nodes/{node_name}/redeploy`
#[tracing::instrument(skip(state), fields(%lab_id, %node_name))]
pub async fn node_redeploy_handler(
    auth: AuthenticatedUserFromCookie,
    State(state): State<AppState>,
    Path((lab_id, node_name)): Path<(String, String)>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let auth = auth.into_context();
    // Checked before loading the saved manifest so a non-owner gets 403, not a load error.
    require_lab_owner(&auth, &state.db, &lab_id).await?;

    // New labs persist TOML; older JSON manifests remain readable.
    let lab_dir = PathBuf::from(SHERPA_LABS_PATH).join(&lab_id);
    let manifest = load_saved_manifest(&lab_dir).map_err(|error| {
        tracing::error!(%error, "Failed to load saved lab manifest");
        ApiError::internal("Unable to load saved lab manifest")
    })?;

    // The web UI cannot send secret files, so the node keeps its current ones.
    let request = RedeployRequest {
        lab_id,
        node_name: node_name.clone(),
        manifest,
        secrets: None,
    };

    let progress = ProgressSender::new(tokio::sync::mpsc::unbounded_channel().0);

    let result =
        run_detached(
            async move { redeploy::redeploy_node(&auth, request, &state, progress).await },
        )
        .await
        .map_err(|e| {
            tracing::error!("Redeploy failed for node '{}': {:?}", node_name, e);
            ApiError::internal(format!("Redeploy failed: {e}"))
        })?;

    Ok(Json(json!({
        "success": result.success,
        "message": result.message,
        "links": result.links,
    })))
}

/// Lab detail page handler
///
/// Displays detailed information about a specific lab including:
/// - Lab metadata (name, ID, network configuration)
/// - List of devices (active and inactive) with their details
///
/// Requires authentication via cookie.
///
/// # Path Parameters
/// - `lab_id`: Lab ID (UUID format)
///
/// # Response (200 OK)
/// Returns HTML page with lab details
///
/// # Errors
/// - `403 Forbidden` - User doesn't have permission to view this lab
/// - `404 Not Found` - Lab doesn't exist
/// - `500 Internal Server Error` - Server error while fetching lab details
///
pub async fn lab_detail_handler(
    Path(lab_id): Path<String>,
    auth: AuthenticatedUserFromCookie,
    State(state): State<AppState>,
) -> impl IntoResponse {
    tracing::debug!(
        "Fetching lab details for lab '{}' by user '{}'",
        lab_id,
        auth.username
    );

    // Call inspect service
    match inspect::inspect_lab(&auth.context(), &lab_id, &state).await {
        Ok(response) => {
            tracing::info!(
                "Successfully loaded lab '{}' details for user '{}'",
                lab_id,
                auth.username
            );
            let device_count = response.devices.len();
            let inactive_device_count = response.inactive_devices.len();
            let link_count = response.links.len();
            let bridge_count = response.bridges.len();
            LabDetailTemplate {
                username: auth.username.clone(),
                is_admin: auth.is_admin,
                nav: NavPage::Labs.into(),
                lab_id: lab_id.clone(),
                lab_info: response.lab_info,
                state_badge: Badge::for_lab(
                    response.lab_phase,
                    response.lab_state,
                    BadgeSize::Large,
                ),
                actions: LabActions::for_lab(response.lab_phase, response.lab_state),
                offers_download: lab_offers_download(response.lab_state),
                nodes: response.devices.into_iter().map(NodeRow::from).collect(),
                device_count,
                inactive_devices: response.inactive_devices,
                inactive_device_count,
                links: response.links,
                link_count,
                bridges: response.bridges,
                bridge_count,
            }
            .into_response()
        }
        Err(e) => lab_error_page(
            auth.username,
            auth.is_admin,
            &lab_id,
            &e,
            "An error occurred loading the lab.",
        ),
    }
}

/// Whether the lab detail page offers the lab files for download.
///
/// A lab with no nodes, or one whose state cannot be determined, has nothing to download.
pub(crate) fn lab_offers_download(lab_state: LabState) -> bool {
    match lab_state {
        LabState::Running
        | LabState::Stopped
        | LabState::Partial
        | LabState::Starting
        | LabState::Failed => true,
        LabState::Empty | LabState::Unknown => false,
    }
}

/// Render the web error page for a failed lab service call.
///
/// Authorization failures render 403 or 404 pages; other failures are logged.
fn lab_error_page(
    username: String,
    is_admin: bool,
    lab_id: &str,
    error: &anyhow::Error,
    failure_message: &str,
) -> Response {
    match ServiceError::find(error) {
        Some(ServiceError::Denied(_)) => Error403Template {
            username,
            is_admin,
            nav: NavPage::Other.into(),
            message: "You don't have permission to view this lab.".to_string(),
        }
        .into_response(),
        Some(ServiceError::NotFound(_)) => {
            tracing::debug!("Lab '{}' not found for user '{}'", lab_id, username);
            Error404Template {
                username,
                is_admin,
                nav: NavPage::Other.into(),
                message: "Lab not found.".to_string(),
            }
            .into_response()
        }
        Some(
            ServiceError::Conflict(_)
            | ServiceError::Invalid(_)
            | ServiceError::InsufficientMemory { .. },
        )
        | None => {
            tracing::error!(
                "Failed to load lab '{}' for user '{}': {:?}",
                lab_id,
                username,
                error
            );
            Error404Template {
                username,
                is_admin,
                nav: NavPage::Other.into(),
                message: failure_message.to_string(),
            }
            .into_response()
        }
    }
}

/// Render the HTMX error fragment for a failed lab service call.
///
/// Authorization failures keep their 403 or 404 status; other failures render the message.
fn lab_error_fragment(error: &anyhow::Error, failure_message: &str) -> Response {
    let message = format!("{failure_message}: {error}");
    match ServiceError::find(error) {
        Some(ServiceError::Denied(_)) => {
            (StatusCode::FORBIDDEN, ErrorTemplate { message }).into_response()
        }
        Some(ServiceError::NotFound(_)) => {
            (StatusCode::NOT_FOUND, ErrorTemplate { message }).into_response()
        }
        // The refusal names the memory that fell short; the outer context would not.
        Some(refusal @ ServiceError::InsufficientMemory { .. }) => {
            tracing::warn!("{failure_message}: {refusal}");
            ErrorTemplate {
                message: format!("{failure_message}: {refusal}"),
            }
            .into_response()
        }
        Some(ServiceError::Conflict(_) | ServiceError::Invalid(_)) | None => {
            tracing::error!("{failure_message}: {:?}", error);
            ErrorTemplate { message }.into_response()
        }
    }
}

// ============================================================================
// Node Detail Handler
// ============================================================================

/// Node detail page (web UI handler, cookie auth)
///
/// GET `/labs/{lab_id}/nodes/{node_name}`
#[tracing::instrument(skip(state), fields(%lab_id, %node_name))]
pub async fn node_detail_handler(
    Path((lab_id, node_name)): Path<(String, String)>,
    auth: AuthenticatedUserFromCookie,
    State(state): State<AppState>,
) -> impl IntoResponse {
    // Inspect lab to get runtime device info (VNC port, disks, state)
    let inspect_response = match inspect::inspect_lab(&auth.context(), &lab_id, &state).await {
        Ok(r) => r,
        Err(e) => {
            return lab_error_page(
                auth.username,
                auth.is_admin,
                &lab_id,
                &e,
                "An error occurred loading lab data.",
            );
        }
    };

    // Find the specific device
    let device = match inspect_response
        .devices
        .iter()
        .find(|d| d.name == node_name)
    {
        Some(d) => d.clone(),
        None => {
            return Error404Template {
                username: auth.username,
                is_admin: auth.is_admin,
                nav: NavPage::Other.into(),
                message: format!("Node '{node_name}' not found in lab."),
            }
            .into_response();
        }
    };

    // Fetch DbNode for mgmt_mac and image reference
    let db_lab = match db::get_lab(&state.db, &lab_id).await {
        Ok(l) => l,
        Err(e) => {
            tracing::error!("Failed to get lab from DB: {:?}", e);
            return Error404Template {
                username: auth.username,
                is_admin: auth.is_admin,
                nav: NavPage::Other.into(),
                message: "Lab not found.".to_string(),
            }
            .into_response();
        }
    };

    let Some(lab_record_id) = db_lab.id else {
        return ErrorTemplate {
            message: "Lab missing record ID.".to_string(),
        }
        .into_response();
    };

    let db_node = match db::get_node_by_name_and_lab(&state.db, &node_name, lab_record_id).await {
        Ok(n) => n,
        Err(e) => {
            tracing::error!("Failed to get node from DB: {:?}", e);
            return Error404Template {
                username: auth.username,
                is_admin: auth.is_admin,
                nav: NavPage::Other.into(),
                message: format!("Node '{node_name}' not found."),
            }
            .into_response();
        }
    };

    // Fetch node image config
    let node_config = match db::find_node_image_by_id(&state.db, db_node.image.clone()).await {
        Ok(Some(config)) => config,
        Ok(None) => {
            tracing::error!("Node image not found for node '{}'", node_name);
            return ErrorTemplate {
                message: "Node image configuration not found.".to_string(),
            }
            .into_response();
        }
        Err(e) => {
            tracing::error!("Failed to fetch node image: {:?}", e);
            return ErrorTemplate {
                message: "Failed to load node image configuration.".to_string(),
            }
            .into_response();
        }
    };

    let ztp = effective_ztp(saved_node_ztp(&lab_id, &node_name), &node_config);

    NodeDetailTemplate {
        username: auth.username,
        is_admin: auth.is_admin,
        nav: NavPage::Labs.into(),
        lab_id: lab_id.clone(),
        lab_name: inspect_response.lab_info.name.clone(),
        state_badge: Badge::for_node_state(device.state, BadgeSize::Large),
        actions: NodeActions::for_state(device.state),
        device,
        ztp,
        node_config,
        mgmt_mac: db_node.mgmt_mac,
    }
    .into_response()
}

/// The ZTP setting the lab's saved manifest gives the node, if it gives one. A manifest that
/// cannot be read gives none, so the page shows the image's setting.
fn saved_node_ztp(lab_id: &str, node_name: &str) -> Option<Ztp> {
    let lab_dir = PathBuf::from(SHERPA_LABS_PATH).join(lab_id);
    match load_saved_manifest(&lab_dir).and_then(|manifest| manifest_node_ztp(manifest, node_name))
    {
        Ok(ztp) => ztp,
        Err(error) => {
            tracing::warn!(%error, %lab_id, "Failed to read the saved manifest for the node's ZTP");
            None
        }
    }
}

/// The `ztp.method` a manifest sets on its node `node_name`.
fn manifest_node_ztp(manifest: serde_json::Value, node_name: &str) -> anyhow::Result<Option<Ztp>> {
    let manifest: Manifest =
        serde_json::from_value(manifest).context("Failed to deserialize the saved manifest")?;
    Ok(manifest
        .nodes
        .iter()
        .find(|node| node.name == node_name)
        .and_then(|node| node.ztp.method))
}

// ============================================================================
// Admin User Management Handlers
// ============================================================================

/// Handler to return the nodes table fragment for HTMX polling
///
/// GET `/labs/{lab_id}/nodes`
pub async fn lab_nodes_handler(
    Path(lab_id): Path<String>,
    auth: AuthenticatedUserFromCookie,
    State(state): State<AppState>,
) -> impl IntoResponse {
    match inspect::inspect_lab(&auth.into_context(), &lab_id, &state).await {
        Ok(response) => {
            let device_count = response.devices.len();
            NodesTableFragment {
                lab_id,
                nodes: response.devices.into_iter().map(NodeRow::from).collect(),
                device_count,
            }
            .into_response()
        }
        Err(e) => lab_error_fragment(&e, "Failed to load nodes"),
    }
}

/// Download lab files as a zip archive
///
/// GET `/labs/{lab_id}/download`
///
/// Returns a zip containing lab-info.toml and `sherpa_ssh_config`, plus `sherpa_known_hosts` when
/// node host keys were recorded and `sherpa_ssh_key`, the lab's own key, when the lab's SSH config
/// uses it. These files allow CLI tools (sherpa ssh, sherpa console) to work with the lab. Uses
/// the same download service as the RPC API.
#[tracing::instrument(skip(state), fields(%lab_id))]
pub async fn lab_download_handler(
    Path(lab_id): Path<String>,
    auth: AuthenticatedUserFromCookie,
    State(state): State<AppState>,
) -> Result<Response, ApiError> {
    let auth = auth.into_context();
    let db_lab = require_lab_access(&auth, &state.db, &lab_id).await?;
    let files = download::download_lab_files(&auth, &lab_id, &state)
        .await
        .map_err(ApiError::from)?;

    let mut entries = vec![
        (LAB_FILE_NAME, files.lab_info.to_string()),
        (SHERPA_SSH_CONFIG_FILE, files.ssh_config),
    ];
    if !files.known_hosts.is_empty() {
        entries.push((SHERPA_KNOWN_HOSTS_FILE, files.known_hosts));
    }
    if !files.ssh_private_key.is_empty() {
        entries.push((SHERPA_SSH_PRIVATE_KEY_FILE, files.ssh_private_key));
    }

    let mut buf = std::io::Cursor::new(Vec::new());
    {
        let mut zip = zip::ZipWriter::new(&mut buf);
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);

        for (name, content) in entries {
            zip.start_file(name, options)
                .map_err(|e| ApiError::internal(format!("Failed to create zip: {e}")))?;
            std::io::Write::write_all(&mut zip, content.as_bytes())
                .map_err(|e| ApiError::internal(format!("Failed to write zip: {e}")))?;
        }

        zip.finish()
            .map_err(|e| ApiError::internal(format!("Failed to finalize zip: {e}")))?;
    }

    let zip_bytes = buf.into_inner();
    let filename = format!("{}-{}.zip", db_lab.name, lab_id);

    Ok((
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "application/zip".to_string()),
            (
                header::CONTENT_DISPOSITION,
                format!(r#"attachment; filename="{filename}""#),
            ),
        ],
        zip_bytes,
    )
        .into_response())
}

/// Handler to return the destroy confirmation panel
///
/// GET `/labs/{lab_id}/destroy/confirm`
pub async fn lab_destroy_confirm_handler(
    Path(lab_id): Path<String>,
    auth: AuthenticatedUserFromCookie,
    State(state): State<AppState>,
) -> impl IntoResponse {
    match inspect::inspect_lab(&auth.into_context(), &lab_id, &state).await {
        Ok(response) => LabDestroyConfirmFragment {
            lab_id,
            lab_name: response.lab_info.name,
            device_count: response.devices.len(),
        }
        .into_response(),
        Err(e) => lab_error_fragment(&e, "Failed to load lab details"),
    }
}

/// Handler to return the original destroy button (used by Cancel)
///
/// GET `/labs/{lab_id}/destroy/button`
pub async fn lab_destroy_button_handler(
    Path(lab_id): Path<String>,
    _auth: AuthenticatedUserFromCookie,
) -> impl IntoResponse {
    LabDestroyButtonFragment { lab_id }
}

/// Handler to initiate destroy — creates a job and redirects to the jobs page
///
/// POST `/labs/{lab_id}/destroy`
pub async fn lab_destroy_post_handler(
    Path(lab_id): Path<String>,
    auth: AuthenticatedUserFromCookie,
    State(state): State<AppState>,
) -> Result<Response, ApiError> {
    // Checked before the job is queued; the destroy service checks again when it runs.
    let auth = auth.into_context();
    let db_lab = require_lab_access(&auth, &state.db, &lab_id).await?;

    let job_id = uuid::Uuid::new_v4().to_string();
    state.pending_jobs.insert(
        job_id.clone(),
        Job {
            lab_name: db_lab.name,
            job_type: JobType::Destroy { lab_id },
            requester: auth,
        },
    );

    let redirect_url = format!("/jobs/{job_id}");
    let mut response = Html("").into_response();
    response.headers_mut().insert(
        header::HeaderName::from_static("hx-redirect"),
        header::HeaderValue::from_str(&redirect_url)
            .map_err(|e| ApiError::internal(format!("Failed to create redirect header: {e}")))?,
    );

    Ok(response)
}

// ============================================================================
// Job Handlers (HTML)
// ============================================================================

/// Handler to render the job progress page
///
/// GET `/jobs/{job_id}`
pub async fn job_page_handler(
    Path(job_id): Path<String>,
    auth: AuthenticatedUserFromCookie,
    State(state): State<AppState>,
) -> impl IntoResponse {
    // Peek at the job to get display info (don't remove it — stream handler consumes it)
    match state.pending_jobs.get(&job_id) {
        Some(job) if !job.can_view(&auth.context()) => {
            tracing::warn!(
                event = "auth.access_denied",
                username = %auth.username,
                %job_id,
                "User attempted to view another user's job"
            );
            Error403Template {
                username: auth.username,
                is_admin: auth.is_admin,
                nav: NavPage::Other.into(),
                message: "You don't have permission to view this job.".to_string(),
            }
            .into_response()
        }
        Some(job) => {
            let action = match &job.job_type {
                JobType::Create { .. } => "Creating".to_string(),
                JobType::Destroy { .. } => "Destroying".to_string(),
            };
            JobPageTemplate {
                username: auth.username,
                is_admin: auth.is_admin,
                nav: NavPage::Labs.into(),
                job_id,
                lab_name: job.lab_name.clone(),
                action,
            }
            .into_response()
        }
        None => Error404Template {
            username: auth.username,
            is_admin: auth.is_admin,
            nav: NavPage::Other.into(),
            message: "Job not found or already completed".to_string(),
        }
        .into_response(),
    }
}

/// Handler to stream job progress via SSE
///
/// GET `/jobs/{job_id}/stream`
pub async fn job_stream_handler(
    Path(job_id): Path<String>,
    auth: AuthenticatedUserFromCookie,
    State(state): State<AppState>,
) -> impl IntoResponse {
    // Refuse other users before consuming the job, so a refusal leaves it for its creator.
    // The map guard is dropped at the end of this statement, before the job is removed.
    let allowed = match state.pending_jobs.get(&job_id) {
        Some(job) => job.can_view(&auth.context()),
        // A missing job is reported by the consume step below.
        None => true,
    };
    if !allowed {
        tracing::warn!(
            event = "auth.access_denied",
            username = %auth.username,
            %job_id,
            "User attempted to stream another user's job"
        );
        return ApiError::forbidden("You don't have permission to view this job").into_response();
    }

    // Consume the pending job
    let Some((_, job)) = state.pending_jobs.remove(&job_id) else {
        return failed_job_stream(anyhow::anyhow!("Job not found or already consumed"));
    };

    // The job runs as the user who created it, as they are now: a requester whose session has
    // ended since queuing it is refused, and one whose rights changed runs with the new rights.
    let requester = match job.resolve_requester(&state.db).await {
        Ok(SessionLookup::Active(requester)) => requester,
        Ok(SessionLookup::Rejected(rejection)) => {
            tracing::warn!(
                event = "auth.job_refused",
                requester = %job.requester.username,
                %job_id,
                %rejection,
                "Refused to start a job whose requester's session cannot be used"
            );
            return failed_job_stream(anyhow::anyhow!(
                "The job was not started: {rejection} since it was queued. Log in and start it \
                 again"
            ));
        }
        Err(error) => {
            tracing::error!(%job_id, error = ?error, "Failed to resolve the job's requester");
            return failed_job_stream(error.context("The job was not started"));
        }
    };

    let (progress_tx, progress_rx) = tokio::sync::mpsc::unbounded_channel();
    let progress = ProgressSender::new(progress_tx);

    match job.job_type {
        JobType::Destroy { lab_id } => {
            let (result_tx, result_rx) = tokio::sync::oneshot::channel();
            tokio::spawn(async move {
                let result = destroy::destroy_lab(&requester, &lab_id, &state, progress).await;
                let _ = result_tx.send(result);
            });
            sse::Sse::new(destroy_progress_stream(progress_rx, result_rx)).into_response()
        }
        JobType::Create { request } => {
            let (result_tx, result_rx) = tokio::sync::oneshot::channel();
            tokio::spawn(async move {
                let result = up::up_lab(&requester, request, &state, progress).await;
                let _ = result_tx.send(result);
            });
            sse::Sse::new(up_progress_stream(progress_rx, result_rx)).into_response()
        }
    }
}

/// An SSE stream that reports `error` as a job's result, for a job that never started.
fn failed_job_stream(error: anyhow::Error) -> Response {
    let (tx, rx) = tokio::sync::oneshot::channel();
    let _ = tx.send(Err(error));
    sse::Sse::new(destroy_progress_stream(
        tokio::sync::mpsc::unbounded_channel().1,
        rx,
    ))
    .into_response()
}

// ============================================================================
// Lab Create Handlers (HTML)
// ============================================================================

/// Handler to show the create lab page
///
/// GET `/labs/create`
pub async fn lab_create_page_handler(auth: AuthenticatedUserFromCookie) -> impl IntoResponse {
    let generated_name = generate_lab_name().unwrap_or_else(|_| "my-lab".to_string());
    let models: Vec<String> = NodeModel::iter().map(|m| m.to_string()).collect();

    LabCreateTemplate {
        username: auth.username,
        is_admin: auth.is_admin,
        nav: NavPage::Labs.into(),
        generated_name,
        models,
    }
}

/// Handler to validate manifest and initiate lab creation
///
/// POST `/labs/create`
pub async fn lab_create_post_handler(
    auth: AuthenticatedUserFromCookie,
    State(state): State<AppState>,
    Form(form): Form<HashMap<String, String>>,
) -> impl IntoResponse {
    let auth = auth.into_context();
    if let Err(refusal) = require_lab_starter(&auth) {
        return ErrorTemplate {
            message: refusal.to_string(),
        }
        .into_response();
    }

    let manifest_toml = match form.get("manifest") {
        Some(m) if !m.trim().is_empty() => m.trim().to_string(),
        _ => {
            return ErrorTemplate {
                message: "Manifest is required".to_string(),
            }
            .into_response();
        }
    };

    // Parse TOML to extract lab name and validate structure
    let manifest_value: serde_json::Value = match toml::from_str::<toml::Value>(&manifest_toml) {
        Ok(toml_val) => match serde_json::to_value(toml_val) {
            Ok(json_val) => json_val,
            Err(e) => {
                return ErrorTemplate {
                    message: format!("Failed to convert manifest: {e}"),
                }
                .into_response();
            }
        },
        Err(e) => {
            return ErrorTemplate {
                message: format!("Invalid TOML: {e}"),
            }
            .into_response();
        }
    };

    let lab_name = match manifest_value.get("name").and_then(|v| v.as_str()) {
        Some(name) => name.to_string(),
        None => {
            return ErrorTemplate {
                message: "Manifest must include a 'name' field".to_string(),
            }
            .into_response();
        }
    };

    let lab_id = get_id_for_user(&auth.username, &lab_name);

    let request = UpRequest {
        lab_id,
        manifest: manifest_value,
        tailscale_auth_key: None,
        node_secrets: BTreeMap::new(),
        client_host: None,
        client_folder: None,
    };

    let job_id = uuid::Uuid::new_v4().to_string();
    state.pending_jobs.insert(
        job_id.clone(),
        Job {
            lab_name,
            job_type: JobType::Create { request },
            requester: auth,
        },
    );

    let redirect_url = format!("/jobs/{job_id}");
    let mut response = Html("").into_response();
    if let Ok(val) = header::HeaderValue::from_str(&redirect_url) {
        response
            .headers_mut()
            .insert(header::HeaderName::from_static("hx-redirect"), val);
    }

    response
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_lab_detail_offers_the_download_only_for_a_lab_with_known_nodes() {
        for (state, expected) in [
            (LabState::Running, true),
            (LabState::Stopped, true),
            (LabState::Partial, true),
            (LabState::Starting, true),
            (LabState::Failed, true),
            (LabState::Empty, false),
            (LabState::Unknown, false),
        ] {
            assert_eq!(lab_offers_download(state), expected, "{state:?}");
        }
        assert_eq!(
            LabState::iter().count(),
            7,
            "a new LabState needs a case here"
        );
    }

    #[test]
    fn the_saved_manifest_gives_a_node_its_ztp_only_when_it_sets_one() {
        let manifest = json!({
            "name": "lab",
            "nodes": [
                { "name": "frr01", "model": "frr_linux" },
                { "name": "frr02", "model": "frr_linux", "ztp": { "method": "disabled" } },
            ],
        });
        assert_eq!(manifest_node_ztp(manifest.clone(), "frr01").unwrap(), None);
        assert_eq!(
            manifest_node_ztp(manifest.clone(), "frr02").unwrap(),
            Some(Ztp::Disabled)
        );
        assert_eq!(manifest_node_ztp(manifest, "absent").unwrap(), None);
    }
}
