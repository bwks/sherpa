use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::{StatusCode, header};
use axum::response::IntoResponse;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::HashMap;
use std::path::PathBuf;

use crate::auth::{cookies, jwt};
use crate::daemon::state::AppState;
use crate::services::{inspect, list_labs};
use crate::templates::{
    DashboardTemplate, EmptyStateTemplate, ErrorTemplate, LabsGridTemplate, LoginErrorTemplate,
    LoginPageTemplate, SignupErrorTemplate, SignupPageTemplate,
};

use super::errors::ApiError;
use super::extractors::{AuthenticatedUser, AuthenticatedUserFromCookie};

use shared::auth::password;
use shared::data::{
    InspectRequest, InspectResponse, ListLabsResponse, LoginRequest, LoginResponse,
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

    // Return response with Set-Cookie header and HX-Redirect
    (
        StatusCode::OK,
        [
            (header::SET_COOKIE, cookie_value),
            ("HX-Redirect".parse().unwrap(), "/".to_string()),
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
        username: auth.username,
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
