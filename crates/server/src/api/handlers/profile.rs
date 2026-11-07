//! The signed-in user's profile: password and SSH keys.

use askama::Template;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Deserialize;

use crate::daemon::state::AppState;
use crate::services::error::ServiceError;
use crate::services::user_password;
use crate::templates::{
    PasswordErrorTemplate, PasswordSuccessTemplate, ProfileTemplate, SshKeyErrorTemplate,
    SshKeysListTemplate,
};

use crate::api::extractors::AuthenticatedUserFromCookie;

use shared::validators::validate_ssh_key;

/// Form data for password update
#[derive(Debug, Deserialize)]
pub struct UpdatePasswordForm {
    current_password: String,
    new_password: String,
    confirm_new_password: String,
}

/// Form data for adding SSH key
#[derive(Debug, Deserialize)]
pub struct AddSshKeyForm {
    ssh_key: String,
}

/// Display user profile page
///
/// GET /profile
///
/// Shows user profile with password change form and SSH key management.
/// Requires authentication via cookie.
pub async fn profile_handler(
    State(state): State<AppState>,
    auth: AuthenticatedUserFromCookie,
) -> impl IntoResponse {
    // Fetch user from database to get current SSH keys
    let user = match db::get_user(&state.db, &auth.username).await {
        Ok(user) => user,
        Err(e) => {
            tracing::error!(
                "Failed to load user '{}' for profile page: {:?}",
                auth.username,
                e
            );
            return (StatusCode::INTERNAL_SERVER_ERROR, "Failed to load profile").into_response();
        }
    };

    ProfileTemplate {
        username: auth.username.clone(),
        is_admin: auth.is_admin,
        active_page: "profile".to_string(),
        ssh_keys_html: SshKeysListTemplate {
            ssh_keys: user.ssh_keys,
        }
        .render()
        .unwrap_or_else(|_| String::from("Error rendering SSH keys")),
    }
    .into_response()
}

/// Update user password
///
/// POST /profile/password
///
/// Validates current password, checks new password strength, and updates password.
/// Returns HTML fragment for HTMX swap (success or error message).
pub async fn update_password_handler(
    State(state): State<AppState>,
    auth: AuthenticatedUserFromCookie,
    axum::Form(form): axum::Form<UpdatePasswordForm>,
) -> impl IntoResponse {
    // 1. Validate passwords match
    if form.new_password != form.confirm_new_password {
        return PasswordErrorTemplate {
            message: "New passwords do not match".to_string(),
        }
        .into_response();
    }

    let ctx = auth.into_context();
    if let Err(error) = user_password::change_password(
        &ctx,
        &ctx.username,
        Some(&form.current_password),
        &form.new_password,
        &state.db,
    )
    .await
    {
        let message = match ServiceError::find(&error) {
            Some(ServiceError::Denied(_)) => "Current password is incorrect".to_owned(),
            Some(refusal @ ServiceError::Invalid(_)) => refusal.to_string(),
            _ => {
                tracing::error!(username = %ctx.username, "Failed to update password: {error:?}");
                "Failed to update password".to_owned()
            }
        };
        return PasswordErrorTemplate { message }.into_response();
    }

    PasswordSuccessTemplate {
        message: "Password updated successfully".to_string(),
    }
    .into_response()
}

/// Add SSH key to user profile
///
/// POST /profile/ssh-keys
///
/// Validates SSH key format and adds it to user's profile.
/// Returns updated SSH keys list HTML fragment for HTMX swap.
pub async fn add_ssh_key_handler(
    State(state): State<AppState>,
    auth: AuthenticatedUserFromCookie,
    axum::Form(form): axum::Form<AddSshKeyForm>,
) -> impl IntoResponse {
    // 1. Validate SSH key format
    if let Err(e) = validate_ssh_key(&form.ssh_key) {
        return SshKeyErrorTemplate {
            message: format!("Invalid SSH key: {}", e),
        }
        .into_response();
    }

    // 2. Get user from database
    let mut user = match db::get_user(&state.db, &auth.username).await {
        Ok(user) => user,
        Err(e) => {
            tracing::error!(
                "Failed to load user '{}' for SSH key addition: {:?}",
                auth.username,
                e
            );
            return SshKeyErrorTemplate {
                message: "Failed to add SSH key".to_string(),
            }
            .into_response();
        }
    };

    // 3. Check if key already exists
    if user.ssh_keys.contains(&form.ssh_key) {
        return SshKeyErrorTemplate {
            message: "This SSH key is already added".to_string(),
        }
        .into_response();
    }

    // 4. Add SSH key to user
    user.ssh_keys.push(form.ssh_key.clone());

    if let Err(e) = db::update_user(&state.db, user).await {
        tracing::error!(
            "Failed to add SSH key for user '{}': {:?}",
            auth.username,
            e
        );
        return SshKeyErrorTemplate {
            message: "Failed to add SSH key".to_string(),
        }
        .into_response();
    }

    tracing::info!("SSH key added for user '{}'", auth.username);

    // 5. Return updated SSH keys list
    fetch_and_render_ssh_keys(&state, &auth.username).await
}

/// Delete SSH key from user profile
///
/// DELETE /profile/ssh-keys/{index}
///
/// Removes SSH key at the specified index from user's profile.
/// Returns updated SSH keys list HTML fragment for HTMX swap.
pub async fn delete_ssh_key_handler(
    State(state): State<AppState>,
    auth: AuthenticatedUserFromCookie,
    Path(index): Path<usize>,
) -> impl IntoResponse {
    // 1. Get user from database
    let mut user = match db::get_user(&state.db, &auth.username).await {
        Ok(user) => user,
        Err(e) => {
            tracing::error!(
                "Failed to load user '{}' for SSH key deletion: {:?}",
                auth.username,
                e
            );
            return SshKeyErrorTemplate {
                message: "Failed to delete SSH key".to_string(),
            }
            .into_response();
        }
    };

    // 2. Validate index
    if index >= user.ssh_keys.len() {
        return SshKeyErrorTemplate {
            message: "SSH key not found".to_string(),
        }
        .into_response();
    }

    // 3. Remove SSH key
    user.ssh_keys.remove(index);

    if let Err(e) = db::update_user(&state.db, user).await {
        tracing::error!(
            "Failed to delete SSH key for user '{}': {:?}",
            auth.username,
            e
        );
        return SshKeyErrorTemplate {
            message: "Failed to delete SSH key".to_string(),
        }
        .into_response();
    }

    tracing::info!(
        "SSH key at index {} deleted for user '{}'",
        index,
        auth.username
    );

    // 4. Return updated SSH keys list
    fetch_and_render_ssh_keys(&state, &auth.username).await
}

/// Helper function to fetch user's SSH keys and render the list template
async fn fetch_and_render_ssh_keys(state: &AppState, username: &str) -> Response {
    let user = match db::get_user(&state.db, username).await {
        Ok(user) => user,
        Err(e) => {
            tracing::error!(
                "Failed to load user '{}' for SSH keys list: {:?}",
                username,
                e
            );
            return SshKeyErrorTemplate {
                message: "Failed to load SSH keys".to_string(),
            }
            .into_response();
        }
    };

    SshKeysListTemplate {
        ssh_keys: user.ssh_keys,
    }
    .into_response()
}

// ============================================================================
// Lab Management Handlers
// ============================================================================
