use askama::Template;
use axum::extract::{Path, Query, State};
use axum::http::{StatusCode, header};
use axum::response::{Html, IntoResponse, Response};
use axum::{Form, Json};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::HashMap;
use std::path::PathBuf;
use std::str::FromStr;
use strum::IntoEnumIterator;
use surrealdb_types::Datetime;

use crate::auth::{cookies, jwt};
use crate::daemon::state::AppState;
use crate::services::{inspect, list_labs};
use crate::templates::{
    AdminDashboardTemplate, AdminPasswordErrorTemplate, AdminPasswordSuccessTemplate,
    AdminSshKeysListTemplate, AdminUserEditTemplate, DashboardTemplate, EmptyStateTemplate,
    Error403Template, Error404Template, ErrorTemplate, LabDetailTemplate, LabsGridTemplate,
    LoginErrorTemplate, LoginPageTemplate, PasswordErrorTemplate, PasswordSuccessTemplate,
    ProfileTemplate, SignupErrorTemplate, SignupPageTemplate, SshKeyErrorTemplate,
    SshKeysListTemplate,
};

use super::errors::ApiError;
use super::extractors::{AdminUser, AuthenticatedUser, AuthenticatedUserFromCookie};

use shared::auth::{password, ssh};
use shared::data::{
    BiosTypes, CpuArchitecture, CpuModels, DiskBuses, InspectRequest, InspectResponse,
    InterfaceType, ListLabsResponse, LoginRequest, LoginResponse, MachineType, NodeModel,
    OsVariant, ZtpMethod,
};
use shared::konst::{
    JWT_TOKEN_EXPIRY_SECONDS, SHERPA_BASE_DIR, SHERPA_CERTS_DIR, SHERPA_SERVER_CERT_FILE,
};

/// Authenticate user and issue JWT token
///
/// # Request Body
/// ```json
/// {
///   "username": "alice",
///   "password": "SecurePass123!"
/// }
/// ```
///
/// # Response (200 OK)
/// ```json
/// {
///   "token": "eyJhbGc...",
///   "username": "alice",
///   "is_admin": false,
///   "expires_at": 1234567890
/// }
/// ```
///
/// # Errors
/// - `401 Unauthorized` - Invalid username or password
/// - `500 Internal Server Error` - Server error during authentication
pub async fn login(
    State(state): State<AppState>,
    Json(request): Json<LoginRequest>,
) -> Result<Json<LoginResponse>, ApiError> {
    // Get user from database (includes password_hash)
    let user = db::get_user_for_auth(&state.db, &request.username)
        .await
        .map_err(|_| {
            // Don't reveal whether user exists
            tracing::debug!("Login attempt for non-existent user: {}", request.username);
            ApiError::unauthorized("Invalid username or password")
        })?;

    // Verify password against stored hash
    let is_valid =
        password::verify_password(&request.password, &user.password_hash).map_err(|e| {
            tracing::error!("Password verification error: {:?}", e);
            ApiError::internal("Authentication error")
        })?;

    if !is_valid {
        tracing::debug!("Invalid password for user: {}", request.username);
        return Err(ApiError::unauthorized("Invalid username or password"));
    }

    // Create JWT token
    let token = jwt::create_token(
        &state.jwt_secret,
        &user.username,
        user.is_admin,
        JWT_TOKEN_EXPIRY_SECONDS,
    )
    .map_err(|e| {
        tracing::error!("Failed to create JWT token: {:?}", e);
        ApiError::internal("Failed to create authentication token")
    })?;

    let now = jiff::Timestamp::now().as_second();
    let expires_at = now + JWT_TOKEN_EXPIRY_SECONDS;

    tracing::info!("User '{}' logged in successfully", user.username);

    Ok(Json(LoginResponse {
        token,
        username: user.username,
        is_admin: user.is_admin,
        expires_at,
    }))
}

// ============================================================================
// HTML Authentication Handlers (Cookie-based)
// ============================================================================

/// Login form data
#[derive(Debug, Deserialize)]
pub struct LoginForm {
    username: String,
    password: String,
    #[serde(default)]
    remember_me: bool,
}

/// Signup form data
#[derive(Debug, Deserialize)]
pub struct SignupForm {
    username: String,
    password: String,
    confirm_password: String,
}

/// Display login page
///
/// GET /login
///
/// Shows the login form. If user is already authenticated (valid cookie),
/// redirects to dashboard.
///
/// Query parameters:
/// - `error`: Optional error code (session_required, session_expired, logout_success)
/// - `message`: Optional informational message
pub async fn login_page_handler(
    Query(params): Query<HashMap<String, String>>,
) -> impl IntoResponse {
    let error = params
        .get("error")
        .map(|s| s.to_string())
        .unwrap_or_default();
    let message = params
        .get("message")
        .map(|s| s.to_string())
        .unwrap_or_default();

    LoginPageTemplate { error, message }
}

/// Process login form submission
///
/// POST /login
///
/// Validates credentials and sets authentication cookie on success.
/// Returns HTMX-compatible response (either error HTML or redirect header).
///
/// Form fields:
/// - username: User's username
/// - password: User's password
/// - remember_me: Optional checkbox (extends cookie to 30 days)
///
/// Success: Returns HX-Redirect header to dashboard
/// Failure: Returns error HTML fragment for HTMX swap
pub async fn login_form_handler(
    State(state): State<AppState>,
    axum::Form(form): axum::Form<LoginForm>,
) -> impl IntoResponse {
    // Get user from database
    let user = match db::get_user_for_auth(&state.db, &form.username).await {
        Ok(user) => user,
        Err(_) => {
            tracing::debug!("Login attempt for non-existent user: {}", form.username);
            return LoginErrorTemplate {
                message: "Invalid username or password".to_string(),
            }
            .into_response();
        }
    };

    // Verify password
    let is_valid = match password::verify_password(&form.password, &user.password_hash) {
        Ok(valid) => valid,
        Err(e) => {
            tracing::error!("Password verification error: {:?}", e);
            return LoginErrorTemplate {
                message: "An error occurred during authentication".to_string(),
            }
            .into_response();
        }
    };

    if !is_valid {
        tracing::debug!("Invalid password for user: {}", form.username);
        return LoginErrorTemplate {
            message: "Invalid username or password".to_string(),
        }
        .into_response();
    }

    // Determine cookie expiry based on remember_me
    let expiry_seconds = if form.remember_me {
        cookies::COOKIE_MAX_AGE_REMEMBER
    } else {
        cookies::COOKIE_MAX_AGE_NORMAL
    };

    // Create JWT token
    let token = match jwt::create_token(
        &state.jwt_secret,
        &user.username,
        user.is_admin,
        expiry_seconds,
    ) {
        Ok(token) => token,
        Err(e) => {
            tracing::error!("Failed to create JWT token: {:?}", e);
            return LoginErrorTemplate {
                message: "An error occurred during authentication".to_string(),
            }
            .into_response();
        }
    };

    // Create auth cookie
    let cookie_value = cookies::create_auth_cookie(&token, form.remember_me);

    tracing::info!(
        "User '{}' logged in successfully (remember_me: {})",
        user.username,
        form.remember_me
    );

    // Redirect admin users to /admin, regular users to /
    let redirect_path = if user.is_admin { "/admin" } else { "/" };

    // Return response with Set-Cookie header and HX-Redirect
    (
        StatusCode::OK,
        [
            (header::SET_COOKIE, cookie_value),
            ("HX-Redirect".parse().unwrap(), redirect_path.to_string()),
        ],
    )
        .into_response()
}

/// Display signup page
///
/// GET /signup
///
/// Shows the signup form with password requirements.
pub async fn signup_page_handler() -> impl IntoResponse {
    SignupPageTemplate {}
}

/// Process signup form submission
///
/// POST /signup
///
/// Creates a new user account with the provided credentials.
/// All self-registered users are created as non-admin.
///
/// Form fields:
/// - username: Desired username (min 3 chars, alphanumeric + @._-)
/// - password: Desired password (must meet strength requirements)
/// - confirm_password: Password confirmation (must match password)
///
/// Success: Creates user, sets auth cookie, returns HX-Redirect to dashboard
/// Failure: Returns error HTML fragment for HTMX swap
///
/// # Validation
/// - Passwords must match
/// - Username must meet format requirements (handled by db::create_user)
/// - Password must meet strength requirements (handled by db::create_user)
/// - Username must be unique (enforced by database)
pub async fn signup_form_handler(
    State(state): State<AppState>,
    axum::Form(form): axum::Form<SignupForm>,
) -> impl IntoResponse {
    tracing::info!("Signup attempt for username: {}", form.username);

    // 1. Validate passwords match
    if form.password != form.confirm_password {
        tracing::debug!(
            "Password mismatch during signup for user: {}",
            form.username
        );
        return SignupErrorTemplate {
            message: "Passwords do not match".to_string(),
        }
        .into_response();
    }

    // 2. Create user in database
    // This will validate username format and password strength
    let user = match db::create_user(
        &state.db,
        form.username.clone(),
        &form.password,
        false,  // is_admin = false (all self-registered users are non-admin)
        vec![], // ssh_keys = empty (can be added later in profile)
    )
    .await
    {
        Ok(user) => user,
        Err(e) => {
            // Parse error and return appropriate user-friendly message
            let error_str = e.to_string();

            let error_msg = if error_str.contains("unique")
                || error_str.contains("already exists")
                || error_str.contains("duplicate")
            {
                // Username already taken
                tracing::debug!("Username already exists: {}", form.username);
                "Username is unavailable".to_string()
            } else if error_str.contains("Username") {
                // Username validation error - extract the message
                tracing::debug!("Username validation failed: {}", error_str);
                // Extract the specific validation message from anyhow error
                extract_validation_message(&error_str, "Username")
            } else if error_str.contains("Password") || error_str.contains("password") {
                // Password validation error - extract the message
                tracing::debug!("Password validation failed for user: {}", form.username);
                // Extract the specific validation message from anyhow error
                extract_validation_message(&error_str, "Password")
            } else {
                // Generic database or other error
                tracing::error!("Registration error for user '{}': {:?}", form.username, e);
                "Registration failed. Please try again.".to_string()
            };

            return SignupErrorTemplate { message: error_msg }.into_response();
        }
    };

    // 3. Generate JWT token for auto-login (7-day expiry)
    let token = match jwt::create_token(
        &state.jwt_secret,
        &user.username,
        user.is_admin,
        cookies::COOKIE_MAX_AGE_NORMAL,
    ) {
        Ok(token) => token,
        Err(e) => {
            // User was created successfully but JWT generation failed
            // This is rare but possible
            tracing::error!(
                "Failed to create JWT token after registration for user '{}': {:?}",
                user.username,
                e
            );
            return SignupErrorTemplate {
                message: "Registration succeeded but login failed. Please sign in manually."
                    .to_string(),
            }
            .into_response();
        }
    };

    // 4. Create auth cookie and redirect to dashboard
    let cookie_value = cookies::create_auth_cookie(&token, false);

    tracing::info!(
        "New user '{}' registered successfully and logged in",
        user.username
    );

    // Return success response with cookie and redirect
    (
        StatusCode::OK,
        [
            (header::SET_COOKIE, cookie_value),
            ("HX-Redirect".parse().unwrap(), "/".to_string()),
        ],
    )
        .into_response()
}

/// Helper function to extract user-friendly validation messages from error strings
///
/// Takes an error string and extracts the most relevant part for display to the user.
/// Handles errors from username and password validation.
fn extract_validation_message(error_str: &str, context: &str) -> String {
    // For password errors, look for the bulleted list of requirements
    if context == "Password" {
        // The password validation error includes a detailed message like:
        // "Password does not meet security requirements:\n• Minimum 8 characters\n• ..."
        if let Some(pos) = error_str.find("Password does not meet security requirements") {
            // Extract everything after this point until we hit a non-validation message
            let msg_part = &error_str[pos..];
            // Find the end of the validation message (usually at the next context marker or end)
            if let Some(end) = msg_part.find("\n\nCaused by:") {
                return msg_part[..end].to_string();
            }
            return msg_part.to_string();
        }
    }

    // For username errors, extract the specific requirement message
    if context == "Username" {
        // Username errors typically have the format:
        // "Username must be at least 3 characters long" or
        // "Username can only contain alphanumeric characters and @._- symbols"
        if let Some(pos) = error_str.find("Username") {
            let msg_part = &error_str[pos..];
            // Take everything until we hit a newline or end
            if let Some(end) = msg_part.find('\n') {
                return msg_part[..end].to_string();
            }
            return msg_part.to_string();
        }
    }

    // Fallback: return a generic message
    format!(
        "{} validation failed. Please check the requirements.",
        context
    )
}

/// Logout handler
///
/// POST /logout
///
/// Clears the authentication cookie and redirects to login page.
pub async fn logout_handler() -> impl IntoResponse {
    tracing::debug!("User logging out");

    let clear_cookie = cookies::create_clear_cookie();

    (
        StatusCode::SEE_OTHER,
        [
            (header::SET_COOKIE, clear_cookie),
            (header::LOCATION, "/login?error=logout_success".to_string()),
        ],
    )
}

// ============================================================================
// Profile Management Handlers
// ============================================================================

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

    // 2. Get user from database (with password_hash)
    let user = match db::get_user_for_auth(&state.db, &auth.username).await {
        Ok(user) => user,
        Err(e) => {
            tracing::error!(
                "Failed to load user '{}' for password update: {:?}",
                auth.username,
                e
            );
            return PasswordErrorTemplate {
                message: "Failed to verify current password".to_string(),
            }
            .into_response();
        }
    };

    // 3. Verify current password
    let is_valid = match password::verify_password(&form.current_password, &user.password_hash) {
        Ok(valid) => valid,
        Err(e) => {
            tracing::error!(
                "Password verification error for user '{}': {:?}",
                auth.username,
                e
            );
            return PasswordErrorTemplate {
                message: "Failed to verify current password".to_string(),
            }
            .into_response();
        }
    };

    if !is_valid {
        return PasswordErrorTemplate {
            message: "Current password is incorrect".to_string(),
        }
        .into_response();
    }

    // 4. Validate new password strength
    if let Err(e) = password::validate_password_strength(&form.new_password) {
        return PasswordErrorTemplate {
            message: format!("New password does not meet requirements: {}", e),
        }
        .into_response();
    }

    // 5. Hash new password
    let new_password_hash = match password::hash_password(&form.new_password) {
        Ok(hash) => hash,
        Err(e) => {
            tracing::error!(
                "Failed to hash new password for user '{}': {:?}",
                auth.username,
                e
            );
            return PasswordErrorTemplate {
                message: "Failed to update password".to_string(),
            }
            .into_response();
        }
    };

    // 6. Update user in database
    let mut updated_user = user;
    updated_user.password_hash = new_password_hash;
    updated_user.updated_at = Datetime::default();

    if let Err(e) = db::update_user(&state.db, updated_user).await {
        tracing::error!(
            "Failed to update password for user '{}': {:?}",
            auth.username,
            e
        );
        return PasswordErrorTemplate {
            message: "Failed to update password".to_string(),
        }
        .into_response();
    }

    tracing::info!("Password updated successfully for user '{}'", auth.username);

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
    if let Err(e) = shared::auth::ssh::validate_ssh_key(&form.ssh_key) {
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
    user.updated_at = Datetime::default();

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
    user.updated_at = Datetime::default();

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

/// Get detailed information about a lab
///
/// Returns the current state of a lab including:
/// - Lab metadata (name, topology, description)
/// - Active devices with status, management IPs, and disk information
/// - Inactive devices (nodes that should exist but aren't running)
///
/// # Authentication
/// Requires valid JWT token in `Authorization: Bearer <token>` header.
///
/// # Authorization
/// Users can only inspect their own labs. Admins can inspect any lab.
///
/// # Path Parameters
/// - `id` - The lab ID to inspect
///
/// # Response (200 OK)
/// Returns `InspectResponse` with lab details.
///
/// # Errors
/// - `401 Unauthorized` - Missing or invalid token
/// - `403 Forbidden` - User doesn't own this lab
/// - `404 Not Found` - Lab doesn't exist
/// - `500 Internal Server Error` - Server-side error
///
/// # Example
/// ```bash
/// curl -H "Authorization: Bearer <token>" \
///      https://server:3030/api/v1/labs/my-lab
/// ```
pub async fn get_lab(
    State(state): State<AppState>,
    Path(lab_id): Path<String>,
    auth: AuthenticatedUser,
) -> Result<Json<InspectResponse>, ApiError> {
    tracing::debug!(
        "User '{}' requesting inspect for lab '{}'",
        auth.username,
        lab_id
    );

    let request = InspectRequest {
        lab_id: lab_id.clone(),
        username: auth.username.clone(),
    };

    // Call service layer
    let response = inspect::inspect_lab(request, &state).await.map_err(|e| {
        tracing::error!(
            "Failed to inspect lab '{}' for user '{}': {:?}",
            lab_id,
            auth.username,
            e
        );
        ApiError::from(e)
    })?;

    tracing::info!(
        "Successfully inspected lab '{}' for user '{}'",
        lab_id,
        auth.username
    );

    Ok(Json(response))
}

/// Health check endpoint
///
/// Returns server status and configuration information.
/// Always publicly accessible (no authentication required).
///
/// # Response (200 OK)
/// ```json
/// {
///   "status": "ok",
///   "service": "sherpad",
///   "tls": "enabled"
/// }
/// ```
pub async fn health_check(State(state): State<AppState>) -> impl IntoResponse {
    let tls_status = if state.config.tls.enabled {
        "enabled"
    } else {
        "disabled"
    };

    (
        StatusCode::OK,
        Json(json!({
            "status": "ok",
            "service": "sherpad",
            "tls": tls_status,
        })),
    )
}

/// Certificate download endpoint
///
/// Returns the server's TLS certificate in PEM format.
/// Always available over HTTP to allow clients to fetch certificate before trusting.
/// Publicly accessible (no authentication required).
///
/// # Response
/// - `200 OK` - Returns certificate file (application/x-pem-file)
/// - `404 Not Found` - Certificate file doesn't exist
/// - `500 Internal Server Error` - Failed to read certificate
/// - `503 Service Unavailable` - TLS is disabled on server
pub async fn get_certificate_handler(State(state): State<AppState>) -> impl IntoResponse {
    // Check if TLS is enabled
    if !state.config.tls.enabled {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            [(header::CONTENT_TYPE, "application/json")],
            Json(json!({
                "error": "TLS is disabled on this server",
                "message": "The server is not using TLS, no certificate is available"
            }))
            .to_string(),
        )
            .into_response();
    }

    // Determine certificate path
    let cert_path = if let Some(ref path) = state.config.tls.cert_path {
        PathBuf::from(path)
    } else {
        PathBuf::from(format!(
            "{}/{}/{}",
            SHERPA_BASE_DIR, SHERPA_CERTS_DIR, SHERPA_SERVER_CERT_FILE
        ))
    };

    // Check if certificate exists
    if !cert_path.exists() {
        tracing::error!("Certificate file not found at: {}", cert_path.display());
        return (
            StatusCode::NOT_FOUND,
            [(header::CONTENT_TYPE, "application/json")],
            Json(json!({
                "error": "Certificate not found",
                "message": "Server certificate file does not exist. This should not happen."
            }))
            .to_string(),
        )
            .into_response();
    }

    // Read certificate file
    match tokio::fs::read_to_string(&cert_path).await {
        Ok(cert_pem) => {
            tracing::info!("Serving certificate from: {}", cert_path.display());
            (
                StatusCode::OK,
                [
                    (header::CONTENT_TYPE, "application/x-pem-file"),
                    (
                        header::CONTENT_DISPOSITION,
                        "inline; filename=\"server.crt\"",
                    ),
                ],
                cert_pem,
            )
                .into_response()
        }
        Err(e) => {
            tracing::error!("Failed to read certificate file: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                [(header::CONTENT_TYPE, "application/json")],
                Json(json!({
                    "error": "Failed to read certificate",
                    "message": "An error occurred while reading the certificate file"
                }))
                .to_string(),
            )
                .into_response()
        }
    }
}

/// List all labs for a specific user (JSON API)
///
/// Query all labs owned by the specified user.
/// Returns JSON response for programmatic API access.
/// Publicly accessible (no authentication) for initial development.
///
/// # Query Parameters
/// - `username` (required) - The username to list labs for
///
/// # Response (200 OK)
/// Returns `ListLabsResponse` with lab summaries
///
/// # Errors
/// - `404 Not Found` - User doesn't exist
/// - `500 Internal Server Error` - Database error
///
/// # Example
/// ```bash
/// curl https://server:3030/api/v1/labs?username=bradmin
/// ```
pub async fn get_labs_json(
    State(state): State<AppState>,
    Query(params): Query<ListLabsQuery>,
) -> Result<Json<ListLabsResponse>, ApiError> {
    let username = params.username.trim();

    if username.is_empty() {
        return Err(ApiError::bad_request("Username parameter is required"));
    }

    tracing::debug!("Listing labs for user '{}'", username);

    // Call service layer
    let response = list_labs::list_labs(username, &state).await.map_err(|e| {
        tracing::error!("Failed to list labs for user '{}': {:?}", username, e);
        // Check if it's a user not found error
        if e.to_string().contains("User not found") {
            ApiError::not_found("User", format!("User '{}' not found", username))
        } else {
            ApiError::from(e)
        }
    })?;

    tracing::info!(
        "Successfully listed {} labs for user '{}'",
        response.total,
        username
    );

    Ok(Json(response))
}

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
/// # Example
/// ```bash
/// curl -b "sherpa_auth=..." https://server:3030/
/// ```
pub async fn dashboard_handler(
    auth: AuthenticatedUserFromCookie,
) -> Result<DashboardTemplate, ApiError> {
    tracing::debug!("Serving dashboard for user '{}'", auth.username);

    Ok(DashboardTemplate {
        username: auth.username.clone(),
        is_admin: auth.is_admin,
    })
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
/// # Example
/// ```bash
/// curl -b "sherpa_auth=..." https://server:3030/labs
/// ```
pub async fn get_labs_html(
    State(state): State<AppState>,
    auth: AuthenticatedUserFromCookie,
) -> impl IntoResponse {
    tracing::debug!("Fetching labs HTML for user '{}'", auth.username);

    // Call service layer with authenticated username
    match list_labs::list_labs(&auth.username, &state).await {
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
                    labs: response.labs,
                }
                .into_response()
            }
        }
        Err(e) => {
            tracing::error!("Failed to load labs for user '{}': {:?}", auth.username, e);
            let message = if e.to_string().contains("User not found") {
                format!("User '{}' not found", auth.username)
            } else {
                format!("Failed to load labs: {}", e)
            };
            ErrorTemplate { message }.into_response()
        }
    }
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
/// # Example
/// ```bash
/// curl -b "sherpa_auth=..." https://server:3030/labs/550e8400-e29b-41d4-a716-446655440000
/// ```
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

    // Create inspect request
    let request = InspectRequest {
        lab_id: lab_id.clone(),
        username: auth.username.clone(),
    };

    // Call inspect service
    match inspect::inspect_lab(request, &state).await {
        Ok(response) => {
            tracing::info!(
                "Successfully loaded lab '{}' details for user '{}'",
                lab_id,
                auth.username
            );
            let device_count = response.devices.len();
            let link_count = response.links.len();
            let bridge_count = response.bridges.len();
            LabDetailTemplate {
                username: auth.username.clone(),
                is_admin: auth.is_admin,
                lab_info: response.lab_info,
                devices: response.devices,
                device_count,
                links: response.links,
                link_count,
                bridges: response.bridges,
                bridge_count,
            }
            .into_response()
        }
        Err(e) => {
            let error_msg = e.to_string();

            // Check for permission denied
            if error_msg.contains("Permission denied") {
                tracing::warn!(
                    "User '{}' attempted to access lab '{}' without permission",
                    auth.username,
                    lab_id
                );
                Error403Template {
                    username: auth.username,
                    message: "You don't have permission to view this lab.".to_string(),
                }
                .into_response()
            }
            // Check for not found
            else if error_msg.contains("not found") {
                tracing::debug!("Lab '{}' not found for user '{}'", lab_id, auth.username);
                Error404Template {
                    username: auth.username,
                    message: "Lab not found.".to_string(),
                }
                .into_response()
            }
            // Generic error - log and return error page
            else {
                tracing::error!(
                    "Failed to load lab '{}' for user '{}': {:?}",
                    lab_id,
                    auth.username,
                    e
                );
                Error404Template {
                    username: auth.username,
                    message: "An error occurred loading the lab.".to_string(),
                }
                .into_response()
            }
        }
    }
}

// ============================================================================
// Admin User Management Handlers
// ============================================================================

/// Helper struct for displaying user information in admin dashboard
#[derive(Debug, Clone)]
pub struct UserSummary {
    pub username: String,
    pub is_admin: bool,
    pub ssh_key_count: usize,
    pub lab_count: usize,
    pub created_at_formatted: String,
}

/// Admin dashboard - lists all users
pub async fn admin_dashboard_handler(
    State(state): State<AppState>,
    admin: AdminUser,
) -> Result<Response, ApiError> {
    tracing::info!("Admin '{}' accessing admin dashboard", admin.username);

    // Get all users from database
    let users = db::list_users(&state.db).await.map_err(|e| {
        tracing::error!("Failed to list users: {:?}", e);
        ApiError::internal("Failed to load users")
    })?;

    // Transform users into display format and filter out the logged-in admin
    let mut user_summaries: Vec<UserSummary> = Vec::new();

    for user in users.into_iter() {
        // Filter out current admin
        if user.username == admin.username {
            continue;
        }

        // Format created_at date using jiff
        let created_at_formatted = format_date_simple(user.created_at);

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

    tracing::debug!(
        "Loaded {} users for admin dashboard (filtered out current admin '{}')",
        user_summaries.len(),
        admin.username
    );

    Ok(AdminDashboardTemplate {
        username: admin.username,
        users: user_summaries,
    }
    .into_response())
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
            ApiError::not_found("User", format!("User '{}' not found", target_username))
        })?;

    // Check if admin is editing themselves
    let is_self = admin.username == target_username;

    if is_self {
        tracing::debug!("Admin is editing their own account");
    }

    // Render SSH keys list
    let ssh_keys_html = AdminSshKeysListTemplate {
        target_username: target_username.clone(),
        ssh_keys: target_user.ssh_keys.clone(),
        success_message: String::new(),
        is_error: false,
    }
    .render()
    .map_err(|e| {
        tracing::error!("Failed to render SSH keys template: {:?}", e);
        ApiError::internal("Failed to render page")
    })?;

    Ok(AdminUserEditTemplate {
        admin_username: admin.username,
        target_user,
        is_self,
        ssh_keys_html,
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
    if admin.username == target_username {
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

    // Validate password strength
    if let Err(e) = password::validate_password_strength(&form.new_password) {
        return Ok(AdminPasswordErrorTemplate {
            error_message: format!("Password validation failed: {}", e),
        }
        .into_response());
    }

    // Get user from database
    let mut user = db::get_user(&state.db, &target_username)
        .await
        .map_err(|e| {
            tracing::error!("Failed to get user '{}': {:?}", target_username, e);
            ApiError::internal("Failed to update password")
        })?;

    // Hash the new password
    let new_password_hash = password::hash_password(&form.new_password).map_err(|e| {
        tracing::error!("Failed to hash password: {:?}", e);
        ApiError::internal("Failed to update password")
    })?;

    // Update user password
    user.password_hash = new_password_hash;
    user.updated_at = Datetime::default();

    // Save to database
    db::update_user(&state.db, user).await.map_err(|e| {
        tracing::error!(
            "Failed to update user '{}' in database: {:?}",
            target_username,
            e
        );
        ApiError::internal("Failed to update password")
    })?;

    tracing::info!(
        "Admin '{}' successfully updated password for user '{}'",
        admin.username,
        target_username
    );

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
    if let Err(e) = ssh::validate_ssh_key(ssh_key) {
        tracing::debug!("Invalid SSH key format: {:?}", e);
        return Ok(AdminSshKeysListTemplate {
            target_username: target_username.clone(),
            ssh_keys: vec![],
            success_message: format!("Error: {}", e),
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
    user.updated_at = Datetime::default();

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
    user.updated_at = Datetime::default();

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
pub async fn admin_delete_user_handler(
    State(state): State<AppState>,
    Path(target_username): Path<String>,
    admin: AdminUser,
) -> Result<Response, ApiError> {
    tracing::info!(
        "Admin '{}' attempting to delete user '{}'",
        admin.username,
        target_username
    );

    // Get user from database to get their ID
    let user = db::get_user(&state.db, &target_username)
        .await
        .map_err(|e| {
            tracing::error!("Failed to get user '{}': {:?}", target_username, e);
            ApiError::not_found("User", format!("User '{}' not found", target_username))
        })?;

    let user_id = user.id.ok_or_else(|| {
        tracing::error!("User '{}' has no ID", target_username);
        ApiError::internal("User record is invalid")
    })?;

    // Attempt safe deletion (will fail if user owns labs)
    match db::delete_user_safe(&state.db, user_id).await {
        Ok(_) => {
            tracing::info!(
                "Admin '{}' successfully deleted user '{}'",
                admin.username,
                target_username
            );
            // Return empty response - HTMX will remove the table row
            Ok(StatusCode::OK.into_response())
        }
        Err(e) => {
            let error_msg = e.to_string();
            tracing::warn!(
                "Admin '{}' failed to delete user '{}': {}",
                admin.username,
                target_username,
                error_msg
            );

            // Check if error is about labs
            if error_msg.contains("owns") && error_msg.contains("lab") {
                Err(ApiError::bad_request(
                    "Cannot delete user: user owns one or more labs. Delete the labs first.",
                ))
            } else {
                Err(ApiError::internal("Failed to delete user"))
            }
        }
    }
}

/// Helper function to format datetime as "MMM DD, YYYY"
fn format_date_simple(dt: Datetime) -> String {
    let timestamp = dt.timestamp();

    // Convert to jiff Timestamp
    match jiff::Timestamp::from_second(timestamp) {
        Ok(ts) => {
            // Format as "Feb 17, 2025"
            let zoned = ts.in_tz("UTC").expect("UTC timezone should always work");
            let month = match zoned.month() {
                1 => "Jan",
                2 => "Feb",
                3 => "Mar",
                4 => "Apr",
                5 => "May",
                6 => "Jun",
                7 => "Jul",
                8 => "Aug",
                9 => "Sep",
                10 => "Oct",
                11 => "Nov",
                12 => "Dec",
                _ => "???",
            };
            format!("{} {}, {}", month, zoned.day(), zoned.year())
        }
        Err(_) => {
            // Fallback to timestamp if conversion fails
            format!("{}", timestamp)
        }
    }
}

/// Helper struct for extracting node image path parameters with optional version
#[derive(Debug, Deserialize)]
pub struct NodeImagePath {
    pub model: String,
    pub version: Option<String>,
}

/// Helper struct to summarize node image data for list view
#[derive(Debug, Clone, Serialize)]
pub struct NodeImageSummary {
    pub model: String,
    pub kind: String,
    pub version: String,
    pub cpu_count: u8,
    pub memory: u16,
    pub data_interface_count: u8,
    pub default: bool,
}

/// Admin handler to list all node imageurations
pub async fn admin_node_images_list_handler(
    State(state): State<AppState>,
    _admin: AdminUser,
) -> Result<impl IntoResponse, ApiError> {
    tracing::debug!("Admin requesting node images list");

    // Fetch all node images from database
    let configs = db::list_node_images(&state.db).await.map_err(|e| {
        tracing::error!("Failed to list node images: {:?}", e);
        ApiError::internal("Failed to load node imageurations")
    })?;

    // Group configs by model and pick the best representative:
    // - Prefer default=true, otherwise pick the first entry
    let mut best_by_model: std::collections::HashMap<String, NodeImageSummary> =
        std::collections::HashMap::new();

    for config in configs {
        let model_key = config.model.to_string();
        let summary = NodeImageSummary {
            model: model_key.clone(),
            kind: config.kind.to_string(),
            version: config.version,
            cpu_count: config.cpu_count,
            memory: config.memory,
            data_interface_count: config.data_interface_count,
            default: config.default,
        };

        let dominated = match best_by_model.get(&model_key) {
            None => true,
            Some(existing) => summary.default && !existing.default,
        };

        if dominated {
            best_by_model.insert(model_key, summary);
        }
    }

    let mut summaries: Vec<NodeImageSummary> = best_by_model.into_values().collect();
    summaries.sort_by(|a, b| a.model.cmp(&b.model));

    let template = crate::templates::AdminNodeImagesListTemplate {
        username: _admin.username.clone(),
        is_admin: true,
        configs: summaries,
    };

    Ok(template)
}

/// Admin handler to view a single node imageuration detail
pub async fn admin_node_image_detail_handler(
    State(state): State<AppState>,
    Path(params): Path<NodeImagePath>,
    _admin: AdminUser,
) -> Result<impl IntoResponse, ApiError> {
    tracing::debug!(
        "Admin requesting node image detail: {} (version: {:?})",
        params.model,
        params.version
    );

    // Parse model from URL string
    let node_model = NodeModel::from_str(&params.model).map_err(|e| {
        tracing::warn!("Invalid node model in URL: {} - {}", params.model, e);
        ApiError::not_found("Node Image", format!("Invalid model: {}", params.model))
    })?;

    // Derive kind from model
    let node_kind = node_model.kind();

    // Fetch the specific config from database
    // If version is specified, fetch that specific version; otherwise fetch default
    let config = if let Some(version) = &params.version {
        db::get_node_image_by_model_kind_version(&state.db, &node_model, &node_kind, version)
            .await
            .map_err(|e| {
                tracing::error!(
                    "Failed to get node image for {}/{}: {:?}",
                    params.model,
                    version,
                    e
                );
                ApiError::not_found(
                    "Node Image",
                    format!(
                        "Configuration for {} version {} not found",
                        params.model, version
                    ),
                )
            })?
            .ok_or_else(|| {
                tracing::warn!("Node config not found for {}/{}", params.model, version);
                ApiError::not_found(
                    "Node Image",
                    format!(
                        "Configuration for {} version {} not found",
                        params.model, version
                    ),
                )
            })?
    } else {
        db::get_default_node_image(&state.db, &node_model, &node_kind)
            .await
            .map_err(|e| {
                tracing::error!(
                    "Failed to get default node image for {}: {:?}",
                    params.model,
                    e
                );
                ApiError::not_found(
                    "Node Image",
                    format!("Default configuration for {} not found", params.model),
                )
            })?
            .ok_or_else(|| {
                tracing::warn!("Default node image not found for {}", params.model);
                ApiError::not_found(
                    "Node Image",
                    format!("Default configuration for {} not found", params.model),
                )
            })?
    };

    let template = crate::templates::AdminNodeImageDetailTemplate {
        username: _admin.username.clone(),
        is_admin: true,
        config,
    };

    Ok(template)
}

/// Admin handler to render the edit page for a node imageuration
pub async fn admin_node_image_edit_page_handler(
    State(state): State<AppState>,
    Path(params): Path<NodeImagePath>,
    _admin: AdminUser,
) -> Result<impl IntoResponse, ApiError> {
    tracing::debug!(
        "Admin requesting node image edit page: {} (version: {:?})",
        params.model,
        params.version
    );

    // Parse model from URL string
    let node_model = NodeModel::from_str(&params.model).map_err(|e| {
        tracing::warn!("Invalid node model in URL: {} - {}", params.model, e);
        ApiError::not_found("Node Image", format!("Invalid model: {}", params.model))
    })?;

    // Derive kind from model
    let node_kind = node_model.kind();

    // Fetch the specific config from database
    // If version is specified, fetch that specific version; otherwise fetch default
    let config = if let Some(version) = &params.version {
        db::get_node_image_by_model_kind_version(&state.db, &node_model, &node_kind, version)
            .await
            .map_err(|e| {
                tracing::error!(
                    "Failed to get node image for {}/{}: {:?}",
                    params.model,
                    version,
                    e
                );
                ApiError::not_found(
                    "Node Image",
                    format!(
                        "Configuration for {} version {} not found",
                        params.model, version
                    ),
                )
            })?
            .ok_or_else(|| {
                tracing::warn!("Node config not found for {}/{}", params.model, version);
                ApiError::not_found(
                    "Node Image",
                    format!(
                        "Configuration for {} version {} not found",
                        params.model, version
                    ),
                )
            })?
    } else {
        db::get_default_node_image(&state.db, &node_model, &node_kind)
            .await
            .map_err(|e| {
                tracing::error!(
                    "Failed to get default node image for {}: {:?}",
                    params.model,
                    e
                );
                ApiError::not_found(
                    "Node Image",
                    format!("Default configuration for {} not found", params.model),
                )
            })?
            .ok_or_else(|| {
                tracing::warn!("Default node image not found for {}", params.model);
                ApiError::not_found(
                    "Node Image",
                    format!("Default configuration for {} not found", params.model),
                )
            })?
    };

    // Generate enum options for dropdowns
    let template = crate::templates::AdminNodeImageEditTemplate {
        username: _admin.username.clone(),
        is_admin: true,
        config,
        os_variants: OsVariant::iter().map(|v| v.to_string()).collect(),
        bios_types: BiosTypes::iter().map(|v| v.to_string()).collect(),
        cpu_architectures: CpuArchitecture::iter().map(|v| v.to_string()).collect(),
        cpu_models: CpuModels::iter().map(|v| v.to_string()).collect(),
        machine_types: MachineType::iter().map(|v| v.to_string()).collect(),
        disk_buses: DiskBuses::iter().map(|v| v.to_string()).collect(),
        ztp_methods: ZtpMethod::iter().map(|v| v.to_string()).collect(),
        interface_types: InterfaceType::iter().map(|v| v.to_string()).collect(),
    };

    Ok(template)
}

/// Form data for updating a node imageuration
#[derive(Deserialize)]
pub struct NodeImageForm {
    // Model, kind, and management_interface come from URL path/existing config, not form
    pub version: String,
    pub repo: Option<String>,
    pub os_variant: String,
    pub bios: String,
    pub cpu_count: u8,
    pub cpu_architecture: String,
    pub cpu_model: String,
    pub machine_type: String,
    pub vmx_enabled: Option<String>, // checkbox
    pub memory: u16,
    pub hdd_bus: String,
    pub cdrom: Option<String>,
    pub cdrom_bus: String,
    pub ztp_enable: Option<String>, // checkbox
    pub ztp_method: String,
    pub ztp_username: Option<String>,
    pub ztp_password: Option<String>,
    pub ztp_password_auth: Option<String>, // checkbox
    pub data_interface_count: u8,
    pub interface_prefix: String,
    pub interface_type: String,
    pub interface_mtu: u16,
    pub first_interface_index: u8,
    pub dedicated_management_interface: Option<String>, // checkbox
    pub reserved_interface_count: u8,
    pub default: Option<String>, // checkbox
}

/// Admin handler to process node imageuration update
pub async fn admin_node_image_update_handler(
    State(state): State<AppState>,
    Path(params): Path<NodeImagePath>,
    _admin: AdminUser,
    Form(form): Form<NodeImageForm>,
) -> Result<Response, ApiError> {
    tracing::info!(
        "Admin updating node image: {} (version: {:?})",
        params.model,
        params.version
    );

    // Parse model from URL string
    let node_model = NodeModel::from_str(&params.model).map_err(|e| {
        tracing::warn!("Invalid node model in URL: {} - {}", params.model, e);
        ApiError::not_found("Node Image", format!("Invalid model: {}", params.model))
    })?;

    // Derive kind from model
    let node_kind = node_model.kind();

    // Fetch the specific config from database
    // If version is specified, fetch that specific version; otherwise fetch default
    let config = if let Some(version) = &params.version {
        db::get_node_image_by_model_kind_version(&state.db, &node_model, &node_kind, version)
            .await
            .map_err(|e| {
                tracing::error!(
                    "Failed to get node image for {}/{}: {:?}",
                    params.model,
                    version,
                    e
                );
                ApiError::not_found(
                    "Node Image",
                    format!(
                        "Configuration for {} version {} not found",
                        params.model, version
                    ),
                )
            })?
            .ok_or_else(|| {
                tracing::warn!("Node config not found for {}/{}", params.model, version);
                ApiError::not_found(
                    "Node Image",
                    format!(
                        "Configuration for {} version {} not found",
                        params.model, version
                    ),
                )
            })?
    } else {
        db::get_default_node_image(&state.db, &node_model, &node_kind)
            .await
            .map_err(|e| {
                tracing::error!(
                    "Failed to get default node image for {}: {:?}",
                    params.model,
                    e
                );
                ApiError::not_found(
                    "Node Image",
                    format!("Default configuration for {} not found", params.model),
                )
            })?
            .ok_or_else(|| {
                tracing::warn!("Default node image not found for {}", params.model);
                ApiError::not_found(
                    "Node Image",
                    format!("Default configuration for {} not found", params.model),
                )
            })?
    };

    // Parse enum fields from form strings using serde_json
    // The enums have #[serde(rename_all = "snake_case")] so we parse via JSON

    // Helper to parse enum from snake_case string
    let parse_enum = |value: &str, field_name: &str| -> Result<serde_json::Value, ApiError> {
        serde_json::from_str(&format!("\"{}\"", value))
            .map_err(|_| ApiError::bad_request(format!("Invalid {}: {}", field_name, value)))
    };

    let os_variant: OsVariant = serde_json::from_value(parse_enum(&form.os_variant, "OS variant")?)
        .map_err(|_| ApiError::bad_request(format!("Invalid OS variant: {}", form.os_variant)))?;

    let bios: BiosTypes = serde_json::from_value(parse_enum(&form.bios, "BIOS type")?)
        .map_err(|_| ApiError::bad_request(format!("Invalid BIOS type: {}", form.bios)))?;

    let cpu_architecture: CpuArchitecture =
        serde_json::from_value(parse_enum(&form.cpu_architecture, "CPU architecture")?).map_err(
            |_| {
                ApiError::bad_request(format!(
                    "Invalid CPU architecture: {}",
                    form.cpu_architecture
                ))
            },
        )?;

    let cpu_model: CpuModels = serde_json::from_value(parse_enum(&form.cpu_model, "CPU model")?)
        .map_err(|_| ApiError::bad_request(format!("Invalid CPU model: {}", form.cpu_model)))?;

    let machine_type: MachineType =
        serde_json::from_value(parse_enum(&form.machine_type, "machine type")?).map_err(|_| {
            ApiError::bad_request(format!("Invalid machine type: {}", form.machine_type))
        })?;

    let hdd_bus: DiskBuses = serde_json::from_value(parse_enum(&form.hdd_bus, "HDD bus")?)
        .map_err(|_| ApiError::bad_request(format!("Invalid HDD bus: {}", form.hdd_bus)))?;

    let cdrom_bus: DiskBuses = serde_json::from_value(parse_enum(&form.cdrom_bus, "CDROM bus")?)
        .map_err(|_| ApiError::bad_request(format!("Invalid CDROM bus: {}", form.cdrom_bus)))?;

    let ztp_method: ZtpMethod = serde_json::from_value(parse_enum(&form.ztp_method, "ZTP method")?)
        .map_err(|_| ApiError::bad_request(format!("Invalid ZTP method: {}", form.ztp_method)))?;

    let interface_type: InterfaceType =
        serde_json::from_value(parse_enum(&form.interface_type, "interface type")?).map_err(
            |_| ApiError::bad_request(format!("Invalid interface type: {}", form.interface_type)),
        )?;

    // Convert checkboxes (Some("on") or None) to bool
    let vmx_enabled = form.vmx_enabled.is_some();
    let ztp_enable = form.ztp_enable.is_some();
    let ztp_password_auth = form.ztp_password_auth.is_some();
    let dedicated_management_interface = form.dedicated_management_interface.is_some();
    let form_default = form.default.is_some();

    // Validate: prevent unsetting the last default for this (model, kind)
    if config.default && !form_default {
        // User is trying to unset default - check if this is the only default
        let versions = db::get_node_image_versions(&state.db, &node_model, &node_kind)
            .await
            .map_err(|e| {
                tracing::error!("Failed to get versions for {}: {:?}", params.model, e);
                ApiError::internal("Failed to validate default status")
            })?;

        let default_count = versions.iter().filter(|v| v.default).count();

        if default_count == 1 {
            tracing::warn!(
                "Cannot unset default - only one default exists for {}",
                params.model
            );
            return Ok(Html(format!(
                r#"<div class="notification-error">Cannot unset default - at least one version must be marked as default for {}</div>"#,
                params.model
            )).into_response());
        }
    }

    // Handle optional string fields - convert empty strings to None
    let repo = form.repo.filter(|s| !s.trim().is_empty());
    let cdrom = form.cdrom.filter(|s| !s.trim().is_empty());
    let ztp_username = form.ztp_username.filter(|s| !s.trim().is_empty());
    let ztp_password = form.ztp_password.filter(|s| !s.trim().is_empty());

    // Validate form data
    if let Err(e) = validate::validate_node_image_update(
        form.cpu_count,
        form.memory,
        form.data_interface_count,
        form.interface_mtu,
        &form.version,
        &form.interface_prefix,
    ) {
        tracing::warn!("Node config validation failed: {:?}", e);
        return Ok(Html(format!(r#"<div class="notification-error">{}</div>"#, e)).into_response());
    }

    // Create updated config (keeping id, model, kind, management_interface from existing)
    let updated_config = shared::data::NodeConfig {
        id: config.id,
        model: config.model, // Keep original (read-only)
        kind: config.kind,   // Keep original (read-only)
        version: form.version,
        repo,
        os_variant,
        bios,
        cpu_count: form.cpu_count,
        cpu_architecture,
        cpu_model,
        machine_type,
        vmx_enabled,
        memory: form.memory,
        hdd_bus,
        cdrom,
        cdrom_bus,
        ztp_enable,
        ztp_method,
        ztp_username,
        ztp_password,
        ztp_password_auth,
        data_interface_count: form.data_interface_count,
        interface_prefix: form.interface_prefix,
        interface_type,
        interface_mtu: form.interface_mtu,
        first_interface_index: form.first_interface_index,
        dedicated_management_interface,
        management_interface: config.management_interface, // Keep original (read-only)
        reserved_interface_count: form.reserved_interface_count,
        default: form_default, // Use value from form checkbox
    };

    // Update in database
    db::update_node_image(&state.db, updated_config)
        .await
        .map_err(|e| {
            tracing::error!("Failed to update node image: {:?}", e);
            ApiError::internal(format!("Failed to update configuration: {}", e))
        })?;

    tracing::info!(
        "Successfully updated node image: {} (version: {:?})",
        params.model,
        params.version
    );

    // Redirect to detail page using HX-Redirect header
    // Include version in URL if it was specified
    let redirect_url = if let Some(version) = &params.version {
        format!("/admin/node-images/{}/{}", params.model, version)
    } else {
        format!("/admin/node-images/{}", params.model)
    };

    let mut response = Html("").into_response();
    response.headers_mut().insert(
        header::HeaderName::from_static("hx-redirect"),
        header::HeaderValue::from_str(&redirect_url).unwrap(),
    );

    Ok(response)
}

/// Handler for listing all versions of a node image (GET /admin/node-images/{model}/versions)
pub async fn admin_node_image_versions_handler(
    State(state): State<AppState>,
    AdminUser { username }: AdminUser,
    Path(model): Path<String>,
) -> Result<Response, ApiError> {
    tracing::debug!("Node config versions list request: model={}", model);

    // Parse model enum
    let model_enum = shared::data::NodeModel::from_str(&model)
        .map_err(|_| ApiError::bad_request(format!("Invalid model: {}", model)))?;

    // Derive kind from model
    let kind_enum = model_enum.kind();

    // Fetch all versions for this model/kind
    let versions = db::get_node_image_versions(&state.db, &model_enum, &kind_enum)
        .await
        .map_err(|e| {
            tracing::error!("Failed to fetch node image versions: {:?}", e);
            ApiError::internal(format!("Failed to fetch versions: {}", e))
        })?;

    tracing::debug!("Found {} versions for {}", versions.len(), model);

    // Create template and render
    let template = crate::templates::AdminNodeImageVersionsTemplate {
        username,
        is_admin: true,
        model,
        kind: kind_enum.to_string(), // Derive kind for display in template
        versions,
    };

    Ok(template.into_response())
}

// Stub handlers for future implementation

/// Handler for creating a lab (stub)
pub async fn lab_up(Json(payload): Json<LabId>) -> String {
    format!("Creating Lab {}", payload.id)
}

/// Handler for destroying a lab (stub)
pub async fn lab_destroy(Json(payload): Json<LabId>) -> String {
    format!("Destroying Lab {}", payload.id)
}

// Request/Response types

/// Query parameters for listing labs
#[derive(Deserialize)]
pub struct ListLabsQuery {
    pub username: String,
}

#[derive(Deserialize)]
#[allow(dead_code)]
pub struct CreateUser {
    pub username: String,
}

#[derive(Deserialize)]
pub struct LabId {
    pub id: String,
}

#[derive(Serialize)]
#[allow(dead_code)]
pub struct User {
    pub id: u64,
    pub username: String,
}
