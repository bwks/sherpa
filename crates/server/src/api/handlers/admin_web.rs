//! The web UI's admin pages: dashboard, labs, tools and users.

use super::labs_api::run_detached;
use axum::extract::{Path, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::{Form, Json};
use serde::Deserialize;

use crate::daemon::state::AppState;
use crate::services::error::ServiceError;
use crate::services::{clean, import, user_delete, user_password};
use crate::templates::{
    AdminNotificationErrorTemplate, AdminPasswordErrorTemplate, AdminPasswordSuccessTemplate,
    AdminSshKeysListTemplate, AdminToolsTemplate, AdminUserEditTemplate, AdminUsersTemplate,
};

use crate::api::errors::ApiError;
use crate::api::extractors::AdminUser;

use shared::data::ScanImagesRequest;
use shared::konst::ADMIN_USER_DELETE_ERROR_TARGET;
use shared::util::normalize_username;
use shared::validators::validate_ssh_key;

/// Helper struct for displaying user information in admin users page
#[derive(Debug, Clone)]
pub struct UserSummary {
    pub username: String,
    pub is_admin: bool,
    pub ssh_key_count: usize,
    pub lab_count: usize,
    pub created_at_formatted: String,
}

/// Admin users page - lists all users
///
/// GET /admin/users
pub async fn admin_dashboard_handler(
    State(state): State<AppState>,
    admin: AdminUser,
) -> Result<Response, ApiError> {
    tracing::info!("Admin '{}' accessing admin users page", admin.username);

    // Get all users from database
    let users = db::list_users(&state.db).await.map_err(|e| {
        tracing::error!("Failed to list users: {:?}", e);
        ApiError::internal("Failed to load users")
    })?;

    // Transform users into display format
    let mut user_summaries: Vec<UserSummary> = Vec::new();

    for user in users {
        // Format created_at date using jiff
        // In UTC, such as "Nov 7, 2026".
        let created_at_formatted = user.created_at.strftime("%b %-d, %Y").to_string();

        // Count labs owned by this user
        let lab_count = if let Some(ref user_id) = user.id {
            db::count_labs_by_user(&state.db, user_id.clone())
                .await
                .unwrap_or(0)
        } else {
            0
        };

        user_summaries.push(UserSummary {
            username: user.username,
            is_admin: user.is_admin,
            ssh_key_count: user.ssh_keys.len(),
            lab_count,
            created_at_formatted,
        });
    }

    // Sort users alphabetically by username
    user_summaries.sort_by(|a, b| a.username.cmp(&b.username));

    tracing::debug!("Loaded {} users for admin users page", user_summaries.len());

    Ok(AdminUsersTemplate {
        username: admin.username,
        is_admin: true,
        active_page: "admin_users".to_string(),
        users: user_summaries,
    }
    .into_response())
}

/// Admin labs list - shows all labs across all users
///
/// GET /admin/labs
pub async fn admin_labs_list_handler(
    State(state): State<AppState>,
    admin: AdminUser,
) -> Result<Response, ApiError> {
    tracing::info!("Admin '{}' accessing admin labs list", admin.username);

    let labs = db::list_labs(&state.db).await.map_err(|e| {
        tracing::error!("Failed to list labs: {:?}", e);
        ApiError::internal("Failed to load labs")
    })?;

    let mut lab_summaries: Vec<crate::templates::AdminLabSummary> = Vec::new();

    for lab in labs {
        let owner_username = match db::find_user_by_id(&state.db, lab.user.clone()).await {
            Ok(Some(user)) => user.username,
            _ => "unknown".to_string(),
        };

        let node_count = if let Some(ref lab_id) = lab.id {
            db::count_nodes_by_lab(&state.db, lab_id.clone())
                .await
                .unwrap_or(0)
        } else {
            0
        };

        lab_summaries.push(crate::templates::AdminLabSummary {
            lab_id: lab.lab_id,
            name: lab.name,
            owner_username,
            node_count,
        });
    }

    lab_summaries.sort_by(|a, b| a.name.cmp(&b.name));

    tracing::debug!("Loaded {} labs for admin labs list", lab_summaries.len());

    Ok(crate::templates::AdminLabsTemplate {
        username: admin.username,
        is_admin: true,
        active_page: "admin_labs".to_string(),
        labs: lab_summaries,
    }
    .into_response())
}

/// Admin tools page
///
/// GET /admin/tools
pub async fn admin_tools_handler(admin: AdminUser) -> impl IntoResponse {
    AdminToolsTemplate {
        username: admin.username,
        is_admin: true,
        active_page: "admin_tools".to_string(),
    }
}

/// Admin lab clean - force-clean all resources for a lab
///
/// POST /`admin/tools/clean/{lab_id`}
#[tracing::instrument(skip(state), fields(%lab_id))]
pub async fn admin_tools_clean_handler(
    admin: AdminUser,
    State(state): State<AppState>,
    Path(lab_id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    tracing::info!(
        admin = %admin.username,
        lab_id = %lab_id,
        "Admin triggering lab clean from tools page"
    );

    let admin = admin.into_context();
    let response = run_detached(async move { clean::clean_lab(&admin, &lab_id, &state).await })
        .await
        .map_err(ApiError::from)?;

    Ok(Json(response))
}

/// Admin image scan - scan filesystem and Docker for discoverable images
///
/// POST /admin/tools/scan
#[tracing::instrument(skip(state))]
pub async fn admin_tools_scan_handler(
    admin: AdminUser,
    State(state): State<AppState>,
    Form(form): Form<ScanForm>,
) -> Result<impl IntoResponse, ApiError> {
    tracing::info!(
        admin = %admin.username,
        dry_run = %form.dry_run(),
        "Admin triggering image scan from tools page"
    );

    let request = ScanImagesRequest {
        kind: None,
        dry_run: form.dry_run(),
    };

    let response = import::scan_images(request, &state)
        .await
        .map_err(ApiError::from)?;

    Ok(Json(response))
}

/// Form data for image scan
#[derive(Debug, Deserialize)]
pub struct ScanForm {
    pub dry_run: Option<String>,
}

impl ScanForm {
    pub fn dry_run(&self) -> bool {
        self.dry_run.as_deref() == Some("true")
    }
}

/// Admin user edit page - shows user details and allows editing
pub async fn admin_user_edit_handler(
    State(state): State<AppState>,
    Path(target_username): Path<String>,
    admin: AdminUser,
) -> Result<Response, ApiError> {
    tracing::info!(
        "Admin '{}' accessing edit page for user '{}'",
        admin.username,
        target_username
    );

    // Get target user from database
    let target_user = db::get_user(&state.db, &target_username)
        .await
        .map_err(|e| {
            tracing::warn!("User '{}' not found: {:?}", target_username, e);
            ApiError::not_found("User", format!("User '{target_username}' not found"))
        })?;

    // Check if admin is editing themselves
    let is_self = admin.username == normalize_username(&target_username);

    if is_self {
        tracing::debug!("Admin is editing their own account");
    }

    let ssh_keys_list = AdminSshKeysListTemplate {
        target_username: target_username.clone(),
        ssh_keys: target_user.ssh_keys.clone(),
        success_message: String::new(),
        is_error: false,
    };

    Ok(AdminUserEditTemplate {
        username: admin.username,
        is_admin: true,
        active_page: "admin_users".to_string(),
        target_user,
        is_self,
        ssh_keys_list,
    }
    .into_response())
}

/// Form for updating user password (admin action)
#[derive(Deserialize)]
pub struct AdminUpdatePasswordForm {
    pub new_password: String,
    pub confirm_new_password: String,
}

/// Admin update user password handler
pub async fn admin_update_user_password_handler(
    State(state): State<AppState>,
    Path(target_username): Path<String>,
    admin: AdminUser,
    Form(form): Form<AdminUpdatePasswordForm>,
) -> Result<Response, ApiError> {
    tracing::info!(
        "Admin '{}' attempting to update password for user '{}'",
        admin.username,
        target_username
    );

    // Prevent admin from updating their own password via admin dashboard
    if admin.username == normalize_username(&target_username) {
        tracing::warn!(
            "Admin '{}' attempted to update their own password via admin dashboard",
            admin.username
        );
        return Ok(AdminPasswordErrorTemplate {
            error_message: "You cannot update your own password here. Please use the Profile page."
                .to_string(),
        }
        .into_response());
    }

    // Validate passwords match
    if form.new_password != form.confirm_new_password {
        return Ok(AdminPasswordErrorTemplate {
            error_message: "Passwords do not match".to_string(),
        }
        .into_response());
    }

    if let Err(error) = user_password::change_password(
        &admin.into_context(),
        &target_username,
        None,
        &form.new_password,
        &state.db,
    )
    .await
    {
        if let Some(refusal @ ServiceError::Invalid(_)) = ServiceError::find(&error) {
            return Ok(AdminPasswordErrorTemplate {
                error_message: refusal.to_string(),
            }
            .into_response());
        }
        tracing::error!(target = %target_username, "Failed to update password: {error:?}");
        return Err(ApiError::from(error));
    }

    Ok(AdminPasswordSuccessTemplate { target_username }.into_response())
}

/// Form for adding SSH key (admin action)
#[derive(Deserialize)]
pub struct AdminAddSshKeyForm {
    pub ssh_key: String,
}

/// Admin add SSH key handler
pub async fn admin_add_ssh_key_handler(
    State(state): State<AppState>,
    Path(target_username): Path<String>,
    admin: AdminUser,
    Form(form): Form<AdminAddSshKeyForm>,
) -> Result<Response, ApiError> {
    tracing::info!(
        "Admin '{}' attempting to add SSH key for user '{}'",
        admin.username,
        target_username
    );

    let ssh_key = form.ssh_key.trim();

    // Validate SSH key format
    if let Err(e) = validate_ssh_key(ssh_key) {
        tracing::debug!("Invalid SSH key format: {:?}", e);
        return Ok(AdminSshKeysListTemplate {
            target_username: target_username.clone(),
            ssh_keys: vec![],
            success_message: format!("Error: {e}"),
            is_error: true,
        }
        .into_response());
    }

    // Get user from database
    let mut user = db::get_user(&state.db, &target_username)
        .await
        .map_err(|e| {
            tracing::error!("Failed to get user '{}': {:?}", target_username, e);
            ApiError::internal("Failed to add SSH key")
        })?;

    // Check for duplicate key
    if user.ssh_keys.contains(&ssh_key.to_string()) {
        tracing::debug!("SSH key already exists for user '{}'", target_username);
        return Ok(AdminSshKeysListTemplate {
            target_username: target_username.clone(),
            ssh_keys: user.ssh_keys,
            success_message: "Error: This SSH key already exists".to_string(),
            is_error: true,
        }
        .into_response());
    }

    // Add SSH key
    user.ssh_keys.push(ssh_key.to_string());

    // Save to database
    db::update_user(&state.db, user.clone())
        .await
        .map_err(|e| {
            tracing::error!(
                "Failed to update user '{}' in database: {:?}",
                target_username,
                e
            );
            ApiError::internal("Failed to add SSH key")
        })?;

    tracing::info!(
        "Admin '{}' successfully added SSH key for user '{}'",
        admin.username,
        target_username
    );

    Ok(AdminSshKeysListTemplate {
        target_username,
        ssh_keys: user.ssh_keys,
        success_message: "SSH key added successfully".to_string(),
        is_error: false,
    }
    .into_response())
}

/// Admin delete SSH key handler
pub async fn admin_delete_ssh_key_handler(
    State(state): State<AppState>,
    Path((target_username, key_index)): Path<(String, usize)>,
    admin: AdminUser,
) -> Result<Response, ApiError> {
    tracing::info!(
        "Admin '{}' attempting to delete SSH key {} for user '{}'",
        admin.username,
        key_index,
        target_username
    );

    // Get user from database
    let mut user = db::get_user(&state.db, &target_username)
        .await
        .map_err(|e| {
            tracing::error!("Failed to get user '{}': {:?}", target_username, e);
            ApiError::internal("Failed to delete SSH key")
        })?;

    // Validate index is in range
    if key_index >= user.ssh_keys.len() {
        tracing::warn!(
            "Invalid SSH key index {} for user '{}' (only {} keys)",
            key_index,
            target_username,
            user.ssh_keys.len()
        );
        return Ok(AdminSshKeysListTemplate {
            target_username: target_username.clone(),
            ssh_keys: user.ssh_keys,
            success_message: "Error: Invalid SSH key index".to_string(),
            is_error: true,
        }
        .into_response());
    }

    // Remove key at index
    user.ssh_keys.remove(key_index);

    // Save to database
    db::update_user(&state.db, user.clone())
        .await
        .map_err(|e| {
            tracing::error!(
                "Failed to update user '{}' in database: {:?}",
                target_username,
                e
            );
            ApiError::internal("Failed to delete SSH key")
        })?;

    tracing::info!(
        "Admin '{}' successfully deleted SSH key {} for user '{}'",
        admin.username,
        key_index,
        target_username
    );

    Ok(AdminSshKeysListTemplate {
        target_username,
        ssh_keys: user.ssh_keys,
        success_message: String::new(),
        is_error: false,
    }
    .into_response())
}

/// Admin delete user handler
///
/// DELETE /admin/users/{username}
///
/// On success the response is empty and HTMX removes the user's table row. A refused delete
/// (self-delete, last admin, or a user who owns labs) is shown in the page's
/// `#user-delete-error` notification instead, using `HX-Retarget`.
#[tracing::instrument(skip(state, admin), fields(admin = %admin.username, %target_username))]
pub async fn admin_delete_user_handler(
    State(state): State<AppState>,
    Path(target_username): Path<String>,
    admin: AdminUser,
) -> Result<Response, ApiError> {
    match user_delete::delete_user(&admin.into_context(), &target_username, &state).await {
        // Return empty response - HTMX will remove the table row
        Ok(_) => Ok(StatusCode::OK.into_response()),
        Err(e) => {
            if let Some(refusal) = ServiceError::find(&e) {
                Ok((
                    [
                        (
                            header::HeaderName::from_static("hx-retarget"),
                            header::HeaderValue::from_static(ADMIN_USER_DELETE_ERROR_TARGET),
                        ),
                        (
                            header::HeaderName::from_static("hx-reswap"),
                            header::HeaderValue::from_static("innerHTML"),
                        ),
                    ],
                    AdminNotificationErrorTemplate {
                        message: refusal.to_string(),
                    },
                )
                    .into_response())
            } else {
                tracing::error!("Failed to delete user '{}': {:?}", target_username, e);
                Err(ApiError::internal("Failed to delete user"))
            }
        }
    }
}
