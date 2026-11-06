//! Login command - authenticate user and save JWT token.

use anyhow::{Context, Result, bail};
use shared::data::{ClientConfig, LoginRequest, LoginResponse, ValidateResponse};
use shared::konst::{SHERPA_USER_NAME_ENV, SHERPA_USER_PASSWORD_ENV};
use shared::util::emoji_success;
use std::env;
use std::io::{self, Write};
use std::time::Duration;

use crate::token;
use crate::ws_client::{WebSocketClient, messages::RpcRequest};

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
/// Removes the saved JWT token from ~/.sherpa/token
pub fn logout() -> Result<()> {
    if !token::token_exists() {
        println!("No active session found");
        return Ok(());
    }

    token::delete_token().context("Failed to delete authentication token")?;
    println!("{}", emoji_success("Logged out successfully"));
    Ok(())
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
