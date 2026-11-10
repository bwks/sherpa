//! Login command - authenticate user and save JWT token.

use anyhow::{Context, Result, bail};
use shared::data::{ClientConfig, LoginRequest, LoginResponse, LogoutResponse, ValidateResponse};
use shared::error::RpcErrorCode;
use shared::konst::{SHERPA_USER_NAME_ENV, SHERPA_USER_PASSWORD_ENV};
use shared::util::{Emoji, emoji_success};
use std::env;
use std::io::{self, Write};
use std::time::Duration;

use crate::token;
use crate::ws_client::messages::RpcResponse;
use crate::ws_client::{WebSocketClient, messages::RpcRequest};

/// What the server said when asked to end the session.
#[derive(Debug)]
enum ServerLogout {
    /// The session was active and has ended.
    Ended,
    /// The session had already ended or expired.
    AlreadyEnded,
    /// The server could not be asked, or failed; the session may still be active.
    Failed(anyhow::Error),
}

/// Execute the login command
///
/// Takes the username and password from `SHERPA_USER_NAME` and `SHERPA_USER_PASSWORD`, or
/// prompts for them, authenticates with server, and saves the JWT token to ~/.sherpa/token
#[tracing::instrument(skip_all, fields(%server_url))]
pub async fn login(server_url: &str, config: &ClientConfig) -> Result<()> {
    let username = match env::var(SHERPA_USER_NAME_ENV) {
        Ok(username) => username.trim().to_string(),
        Err(_) => prompt_username()?,
    };

    if username.is_empty() {
        bail!("Username cannot be empty");
    }

    // Prompt for password (without echo)
    let password = match env::var(SHERPA_USER_PASSWORD_ENV) {
        Ok(password) => password,
        Err(_) => rpassword::prompt_password("Password: ").context("Failed to read password")?,
    };

    if password.is_empty() {
        bail!("Password cannot be empty");
    }

    println!("Authenticating...");

    // Connect to server
    let ws_client = WebSocketClient::new(
        server_url.to_string(),
        Duration::from_secs(10),
        config.server_connection.clone(),
    );
    let mut rpc_client = ws_client
        .connect()
        .await
        .context("Failed to connect to server")?;

    // Create login request
    let request = LoginRequest { username, password };
    let params = serde_json::to_value(&request).context("Failed to serialize login request")?;

    let rpc_request = RpcRequest::new("auth.login", params);

    // Send request and wait for response
    let response = rpc_client
        .call(rpc_request)
        .await
        .context("Login request failed")?;

    let login_response: LoginResponse = response.into_result("Login failed")?;

    // Save token
    token::save_token(&login_response.token).context("Failed to save authentication token")?;

    println!("{}", emoji_success("Login successful!"));
    println!("   Username: {}", login_response.username);
    println!(
        "   Admin: {}",
        if login_response.is_admin { "Yes" } else { "No" }
    );

    // Calculate expiry in human-readable format
    let now = jiff::Timestamp::now().as_second();
    let expires_in_seconds = login_response.expires_at - now;
    let expires_in_days = expires_in_seconds / 86400;
    println!("   Token expires in: {} days", expires_in_days);

    // Close the WebSocket connection gracefully
    rpc_client.close().await.ok();

    Ok(())
}

/// Execute the logout command
///
/// Ends the session on the server, so the token stops working everywhere, then removes the
/// saved token from ~/.sherpa/token. The local token is removed even when the server cannot be
/// reached; the command then warns that the session is still active on the server.
///
/// `server` is the server URL and client configuration, or why they could not be loaded.
#[tracing::instrument(skip_all)]
pub async fn logout(server: Result<(String, ClientConfig)>) -> Result<()> {
    if !token::token_exists() {
        println!("No active session found");
        return Ok(());
    }
    let token = token::load_token().context("Failed to read authentication token")?;
    let outcome = match server {
        Ok((server_url, config)) => end_server_session(&server_url, &config, &token).await,
        Err(error) => ServerLogout::Failed(error),
    };

    token::delete_token().context("Failed to delete authentication token")?;
    match outcome {
        ServerLogout::Failed(error) => {
            tracing::warn!(error = format!("{error:#}"), "Server logout failed");
            eprintln!("{}", logout_message(&ServerLogout::Failed(error)));
        }
        outcome => println!("{}", logout_message(&outcome)),
    }
    Ok(())
}

/// Ask the server to end the session of `token`.
async fn end_server_session(server_url: &str, config: &ClientConfig, token: &str) -> ServerLogout {
    let ws_client = WebSocketClient::new(
        server_url.to_string(),
        Duration::from_secs(10),
        config.server_connection.clone(),
    );
    let mut rpc_client = match ws_client.connect().await {
        Ok(client) => client,
        Err(error) => return ServerLogout::Failed(error.context("Failed to connect to server")),
    };
    let request = RpcRequest::new("auth.logout", serde_json::json!({ "token": token }));
    let outcome = match rpc_client.call(request).await {
        Ok(response) => server_logout_outcome(response),
        Err(error) => ServerLogout::Failed(error.context("Logout request failed")),
    };
    rpc_client.close().await.ok();
    outcome
}

/// What a response to `auth.logout` means. A token the server refuses already has no session.
fn server_logout_outcome(response: RpcResponse) -> ServerLogout {
    if let Some(error) = &response.error
        && error.code == RpcErrorCode::AuthRequired
    {
        return ServerLogout::AlreadyEnded;
    }
    match response.into_result::<LogoutResponse>("Logout failed") {
        Ok(LogoutResponse { ended: true }) => ServerLogout::Ended,
        Ok(LogoutResponse { ended: false }) => ServerLogout::AlreadyEnded,
        Err(error) => ServerLogout::Failed(error),
    }
}

/// What `sherpa logout` prints for `outcome`, once the local token is removed.
fn logout_message(outcome: &ServerLogout) -> String {
    match outcome {
        ServerLogout::Ended => emoji_success("Logged out; the session has ended on the server"),
        ServerLogout::AlreadyEnded => {
            emoji_success("Logged out; the session had already ended on the server")
        }
        ServerLogout::Failed(error) => format!(
            "{} Removed the local token, but could not end the session on the server: {error:#}\n   \
             The token stays valid until it expires or your password is changed",
            Emoji::Warning
        ),
    }
}

/// Execute the whoami command
///
/// Validates the current token and displays user information
#[tracing::instrument(skip_all, fields(%server_url))]
pub async fn whoami(server_url: &str, config: &ClientConfig) -> Result<()> {
    // Load token
    let token_str = token::load_token().context("Not logged in")?;

    // Connect to server
    let ws_client = WebSocketClient::new(
        server_url.to_string(),
        Duration::from_secs(10),
        config.server_connection.clone(),
    );
    let mut rpc_client = ws_client
        .connect()
        .await
        .context("Failed to connect to server")?;

    // Create validate request
    let params = serde_json::json!({
        "token": token_str
    });

    let rpc_request = RpcRequest::new("auth.validate", params);

    // Send request and wait for response
    let response = rpc_client
        .call(rpc_request)
        .await
        .context("Token validation failed")?;

    let validate_response: ValidateResponse = response.into_result("Validation failed")?;

    if !validate_response.valid {
        bail!("Token is invalid or expired; run: sherpa login");
    }

    println!("{}", emoji_success("Authenticated"));
    if let Some(username) = validate_response.username {
        println!("   Username: {username}");
    }
    if let Some(is_admin) = validate_response.is_admin {
        println!("   Admin: {}", if is_admin { "Yes" } else { "No" });
    }
    if let Some(expires_at) = validate_response.expires_at {
        let now = jiff::Timestamp::now().as_second();
        let expires_in_seconds = expires_at - now;
        if expires_in_seconds > 0 {
            let expires_in_days = expires_in_seconds / 86400;
            println!("   Token expires in: {expires_in_days} days");
        } else {
            println!("   Token has expired");
        }
    }

    // Close the WebSocket connection gracefully
    rpc_client.close().await.ok();

    Ok(())
}

fn prompt_username() -> Result<String> {
    print!("Username: ");
    io::stdout().flush()?;
    let mut username = String::new();
    io::stdin()
        .read_line(&mut username)
        .context("Failed to read username")?;
    Ok(username.trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::ws_client::messages::RpcError;

    fn response(result: Option<serde_json::Value>, code: Option<RpcErrorCode>) -> RpcResponse {
        RpcResponse {
            id: "1".to_owned(),
            result,
            error: code.map(|code| RpcError {
                code,
                message: "refused".to_owned(),
                context: None,
            }),
        }
    }

    #[test]
    fn an_ended_session_is_reported_as_ended() {
        let outcome =
            server_logout_outcome(response(Some(serde_json::json!({"ended": true})), None));
        assert!(matches!(outcome, ServerLogout::Ended), "{outcome:?}");
    }

    #[test]
    fn a_session_that_was_already_gone_is_not_a_failure() {
        let outcome =
            server_logout_outcome(response(Some(serde_json::json!({"ended": false})), None));
        assert!(matches!(outcome, ServerLogout::AlreadyEnded), "{outcome:?}");
        // A token the server refuses has no session left to end.
        let outcome = server_logout_outcome(response(None, Some(RpcErrorCode::AuthRequired)));
        assert!(matches!(outcome, ServerLogout::AlreadyEnded), "{outcome:?}");
    }

    #[test]
    fn a_server_error_is_a_failure_and_says_the_session_may_still_work() {
        let outcome = server_logout_outcome(response(None, Some(RpcErrorCode::InternalError)));
        assert!(matches!(outcome, ServerLogout::Failed(_)), "{outcome:?}");
        let message = logout_message(&outcome);
        assert!(
            message.contains("could not end the session on the server"),
            "{message}"
        );
        assert!(message.contains("stays valid"), "{message}");
    }
}
