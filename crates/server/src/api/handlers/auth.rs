//! Login, signup, logout and token validation.

use axum::Json;
use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use std::collections::HashMap;

use crate::auth::{cookies, login, session};
use crate::daemon::state::AppState;
use crate::templates::{
    LoginError, LoginErrorTemplate, LoginPageTemplate, SignupErrorTemplate, SignupPageTemplate,
};

use crate::api::errors::ApiError;
use crate::api::extractors::{
    AuthenticatedUser, CookieSessionError, trace_rejected_session, trace_validated_session,
    validate_cookie_session,
};

use shared::data::{
    LoginRequest, LoginResponse, LogoutResponse, SessionChannel, ValidateRequest, ValidateResponse,
};
use shared::konst::JWT_TOKEN_EXPIRY_SECONDS;

/// Validate a JWT token, as the `auth.validate` RPC method does. An invalid token, or one whose
/// session has ended, is answered `valid: false`, not an error.
pub async fn validate_token_json(
    State(state): State<AppState>,
    Json(request): Json<ValidateRequest>,
) -> Result<Json<ValidateResponse>, ApiError> {
    session::token_status(&state.db, &state.jwt_secret, &request.token)
        .await
        .map(Json)
        .map_err(|error| {
            tracing::error!(error = ?error, "Failed to check a token's session");
            ApiError::internal("Failed to validate the token")
        })
}

/// End the session of the token the request is made with.
///
/// POST `/api/v1/auth/logout`
///
/// The token stops working at once, on every transport. Other sessions of the same user are not
/// affected.
pub async fn logout_json(
    auth: AuthenticatedUser,
    State(state): State<AppState>,
) -> Result<Json<LogoutResponse>, ApiError> {
    let ended = session::logout(&state.db, &auth.session_id)
        .await
        .map_err(|error| {
            tracing::error!(username = %auth.username, error = ?error, "Logout failed");
            ApiError::internal("Failed to end the session")
        })?;
    Ok(Json(LogoutResponse { ended }))
}

/// Authenticate user and issue JWT token
///
/// # Request Body
/// # Response (200 OK)
/// # Errors
/// - `401 Unauthorized` - Invalid username or password
/// - `500 Internal Server Error` - Server error during authentication
pub async fn login(
    State(state): State<AppState>,
    Json(request): Json<LoginRequest>,
) -> Result<Json<LoginResponse>, ApiError> {
    let user = login::check_credentials(&state.db, &request.username, &request.password)
        .await
        .map_err(|e| {
            tracing::error!("Login check failed: {:?}", e);
            ApiError::internal("Authentication error")
        })?
        .ok_or_else(|| ApiError::unauthorized("Invalid username or password"))?;

    let issued = session::start(
        &state.db,
        &state.jwt_secret,
        &user,
        SessionChannel::Api,
        JWT_TOKEN_EXPIRY_SECONDS,
    )
    .await
    .map_err(|e| {
        tracing::error!("Failed to start a session: {:?}", e);
        ApiError::internal("Failed to create authentication token")
    })?;
    let now = issued.issued_at;
    let expires_at = issued.expires_at;

    tracing::info!(
        event = "auth.login_succeeded",
        channel = "api",
        username = %user.username,
        is_admin = user.is_admin,
        issued_at = now,
        expires_at,
        session_duration_seconds = JWT_TOKEN_EXPIRY_SECONDS,
        "User logged in successfully"
    );

    Ok(Json(LoginResponse {
        token: issued.token,
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
/// GET `/login`
///
/// Shows the login form. If user is already authenticated (valid cookie),
/// redirects to dashboard.
///
/// Query parameters:
/// - `error`: Optional error code (`session_required`, `session_expired`, `logout_success`)
/// - `message`: Optional informational message
pub async fn login_page_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(params): Query<HashMap<String, String>>,
) -> Response {
    let error = params
        .get("error")
        .and_then(|code| LoginError::from_code(code));
    let message = params.get("message").cloned().unwrap_or_default();

    let cookie_header = headers
        .get(header::COOKIE)
        .and_then(|value| value.to_str().ok());
    match validate_cookie_session(cookie_header, &state.jwt_secret) {
        Ok(session) => {
            trace_validated_session("/login", &session);
            let redirect_path = if session.claims.is_admin {
                "/admin/users"
            } else {
                "/"
            };
            axum::response::Redirect::to(redirect_path).into_response()
        }
        Err(CookieSessionError::Missing) => LoginPageTemplate {
            error,
            message,
            allow_signup: state.config.allow_signup,
        }
        .into_response(),
        Err(session_error) => {
            trace_rejected_session("/login", &session_error, "clear_and_render_login");
            (
                [(header::SET_COOKIE, cookies::create_clear_cookie())],
                LoginPageTemplate {
                    error: Some(LoginError::SessionExpired),
                    message,
                    allow_signup: state.config.allow_signup,
                },
            )
                .into_response()
        }
    }
}

/// Process login form submission
///
/// POST `/login`
///
/// Validates credentials and sets authentication cookie on success.
/// Returns HTMX-compatible response (either error HTML or redirect header).
///
/// Form fields:
/// - username: User's username
/// - password: User's password
/// - `remember_me`: Optional checkbox (extends cookie to 30 days)
///
/// Success: Returns HX-Redirect header to dashboard
/// Failure: Returns error HTML fragment for HTMX swap
pub async fn login_form_handler(
    State(state): State<AppState>,
    axum::Form(form): axum::Form<LoginForm>,
) -> impl IntoResponse {
    let user = match login::check_credentials(&state.db, &form.username, &form.password).await {
        Ok(Some(user)) => user,
        Ok(None) => {
            return LoginErrorTemplate {
                message: "Invalid username or password".to_string(),
            }
            .into_response();
        }
        Err(e) => {
            tracing::error!("Login check failed: {:?}", e);
            return LoginErrorTemplate {
                message: "An error occurred during authentication".to_string(),
            }
            .into_response();
        }
    };

    // Determine cookie expiry based on remember_me
    let expiry_seconds = if form.remember_me {
        cookies::COOKIE_MAX_AGE_REMEMBER
    } else {
        cookies::COOKIE_MAX_AGE_NORMAL
    };

    let issued = match session::start(
        &state.db,
        &state.jwt_secret,
        &user,
        SessionChannel::Web,
        expiry_seconds,
    )
    .await
    {
        Ok(issued) => issued,
        Err(e) => {
            tracing::error!("Failed to start a session: {:?}", e);
            return LoginErrorTemplate {
                message: "An error occurred during authentication".to_string(),
            }
            .into_response();
        }
    };

    // Create auth cookie
    let cookie_value = cookies::create_auth_cookie(&issued.token, form.remember_me);

    let issued_at = issued.issued_at;
    let expires_at = issued.expires_at;
    tracing::info!(
        event = "auth.login_succeeded",
        channel = "web",
        username = %user.username,
        is_admin = user.is_admin,
        remember_me = form.remember_me,
        issued_at,
        expires_at,
        session_duration_seconds = expiry_seconds,
        "User logged in successfully"
    );

    // Redirect admin users to /admin, regular users to /
    let redirect_path = if user.is_admin { "/admin/users" } else { "/" };

    // Return response with Set-Cookie header and HX-Redirect
    (
        StatusCode::OK,
        [
            (header::SET_COOKIE, cookie_value),
            (
                header::HeaderName::from_static("hx-redirect"),
                redirect_path.to_string(),
            ),
        ],
    )
        .into_response()
}

/// Display signup page
///
/// GET `/signup`
///
/// Shows the signup form with password requirements.
pub async fn signup_page_handler(State(state): State<AppState>) -> Response {
    if !state.config.allow_signup {
        return StatusCode::NOT_FOUND.into_response();
    }
    SignupPageTemplate {}.into_response()
}

/// Process signup form submission
///
/// POST `/signup`
///
/// Creates a new user account with the provided credentials.
/// All self-registered users are created as non-admin.
///
/// Form fields:
/// - username: Desired username (3–32 lowercase letters, digits, `_` or `-`; not reserved)
/// - password: Desired password (must meet strength requirements)
/// - `confirm_password`: Password confirmation (must match password)
///
/// Success: Creates user, sets auth cookie, returns HX-Redirect to dashboard
/// Failure: Returns error HTML fragment for HTMX swap
///
/// # Validation
/// - Passwords must match
/// - Username must meet format requirements (handled by `db::create_user`)
/// - Password must meet strength requirements (handled by `db::create_user`)
/// - Username must be unique (enforced by database)
pub async fn signup_form_handler(
    State(state): State<AppState>,
    axum::Form(form): axum::Form<SignupForm>,
) -> impl IntoResponse {
    if !state.config.allow_signup {
        tracing::warn!(username = %form.username, "Refused signup: signup is closed");
        return StatusCode::NOT_FOUND.into_response();
    }
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

    // 3. Start a session for auto-login (7-day expiry)
    let token = match session::start(
        &state.db,
        &state.jwt_secret,
        &user,
        SessionChannel::Web,
        cookies::COOKIE_MAX_AGE_NORMAL,
    )
    .await
    {
        Ok(issued) => issued.token,
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
            (
                header::HeaderName::from_static("hx-redirect"),
                "/".to_string(),
            ),
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
        // "Username must be 3–32 lowercase letters, ..." or "Username 'admin' is reserved"
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
    format!("{context} validation failed. Please check the requirements.")
}

/// Logout handler
///
/// POST `/logout`
///
/// Ends the cookie's session on the server, so a copy of the cookie stops working too, then
/// clears the cookie and redirects to the login page. A cookie that is missing or no longer
/// valid is just cleared. When the session cannot be ended, the cookie is kept and the user is
/// told to try again, so they are not shown as logged out while the session still works.
pub async fn logout_handler(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let cookie_header = headers
        .get(header::COOKIE)
        .and_then(|value| value.to_str().ok());
    if let Ok(cookie_session) = validate_cookie_session(cookie_header, &state.jwt_secret)
        && let Err(error) = session::logout(&state.db, &cookie_session.claims.sid).await
    {
        tracing::error!(
            username = %cookie_session.claims.sub,
            error = ?error,
            "Failed to end the session at logout"
        );
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "Could not end your session on the server. Try logging out again.",
        )
            .into_response();
    }

    (
        StatusCode::SEE_OTHER,
        [
            (header::SET_COOKIE, cookies::create_clear_cookie()),
            (header::LOCATION, "/login?error=logout_success".to_string()),
        ],
    )
        .into_response()
}

// ============================================================================
// Profile Management Handlers
// ============================================================================
