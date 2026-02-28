//! Login command - authenticate user and save JWT token.

use anyhow::{Context, Result};
use shared::data::{LoginRequest, LoginResponse};
use shared::konst::{SHERPA_BASE_DIR, SHERPA_CONFIG_DIR, SHERPA_CONFIG_FILE};
use shared::util::{emoji_error, emoji_success, load_client_config};
use std::io::{self, Write};
use std::time::Duration;
use uuid::Uuid;

use crate::token;
use crate::ws_client::{WebSocketClient, messages::RpcRequest};

/// Execute the login command
///
/// Prompts for username and password, authenticates with server,
/// and saves the JWT token to ~/.sherpa/token
pub async fn login(server_url: &str, insecure: bool) -> Result<()> {
    // Prompt for username
    print!("Username: ");
    io::stdout().flush()?;
    let mut username = String::new();
    io::stdin()
        .read_line(&mut username)
        .context("Failed to read username")?;
    let username = username.trim().to_string();

    if username.is_empty() {
        anyhow::bail!("Username cannot be empty");
    }

    // Prompt for password (without echo)
    let password = rpassword::prompt_password("Password: ").context("Failed to read password")?;

    if password.is_empty() {
        anyhow::bail!("Password cannot be empty");
    }

    println!("Authenticating...");

    // Load config to get server connection settings (for TLS)
    let config_path = format!("{SHERPA_BASE_DIR}/{SHERPA_CONFIG_DIR}/{SHERPA_CONFIG_FILE}");
    let mut config = load_client_config(&config_path).context("Failed to load configuration")?;

    // Apply insecure flag if set
    if insecure {
        config.server_connection.insecure = true;
        eprintln!("WARNING: TLS certificate validation disabled (--insecure)");
    }

    // Connect to server
    let ws_client = WebSocketClient::new(
        server_url.to_string(),
        Duration::from_secs(10),
        config.server_connection,
    );
    let mut rpc_client = ws_client
        .connect()
        .await
        .context("Failed to connect to server")?;

    // Create login request
    let request = LoginRequest { username, password };
    let params = serde_json::to_value(&request).context("Failed to serialize login request")?;

    let rpc_request = RpcRequest {
        r#type: "rpc_request".to_string(),
        id: Uuid::new_v4().to_string(),
        method: "auth.login".to_string(),
        params,
    };

    // Send request and wait for response
    let response = rpc_client
        .call(rpc_request)
        .await
        .context("Login request failed")?;

    // Check for error
    if let Some(error) = response.error {
        anyhow::bail!("Login failed: {}", error.message);
    }

    // Parse successful response
    let result = response.result.context("No result in response")?;
    let login_response: LoginResponse =
        serde_json::from_value(result).context("Failed to parse login response")?;

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
pub async fn whoami(server_url: &str, insecure: bool) -> Result<()> {
    // Load token
    let token_str = token::load_token().context("Not logged in")?;

    // Load config to get server connection settings (for TLS)
    let config_path = format!("{SHERPA_BASE_DIR}/{SHERPA_CONFIG_DIR}/{SHERPA_CONFIG_FILE}");
    let mut config = load_client_config(&config_path).context("Failed to load configuration")?;

    // Apply insecure flag if set
    if insecure {
        config.server_connection.insecure = true;
        eprintln!("WARNING: TLS certificate validation disabled (--insecure)");
    }

    // Connect to server
    let ws_client = WebSocketClient::new(
        server_url.to_string(),
        Duration::from_secs(10),
        config.server_connection,
    );
    let mut rpc_client = ws_client
        .connect()
        .await
        .context("Failed to connect to server")?;

    // Create validate request
    let params = serde_json::json!({
        "token": token_str
    });

    let rpc_request = RpcRequest {
        r#type: "rpc_request".to_string(),
        id: Uuid::new_v4().to_string(),
        method: "auth.validate".to_string(),
        params,
    };

    // Send request and wait for response
    let response = rpc_client
        .call(rpc_request)
        .await
        .context("Token validation failed")?;

    // Check for error
    if let Some(error) = response.error {
        anyhow::bail!("Validation failed: {}", error.message);
    }

    // Parse response
    let result = response.result.context("No result in response")?;
    let validate_response: shared::data::ValidateResponse =
        serde_json::from_value(result).context("Failed to parse validation response")?;

    if !validate_response.valid {
        println!("{}", emoji_error("Token is invalid or expired"));
        println!("   Please run: sherpa login");
        return Ok(());
    }

    println!("{}", emoji_success("Authenticated"));
    if let Some(username) = validate_response.username {
        println!("   Username: {}", username);
    }
    if let Some(is_admin) = validate_response.is_admin {
        println!("   Admin: {}", if is_admin { "Yes" } else { "No" });
    }
    if let Some(expires_at) = validate_response.expires_at {
        let now = jiff::Timestamp::now().as_second();
        let expires_in_seconds = expires_at - now;
        if expires_in_seconds > 0 {
            let expires_in_days = expires_in_seconds / 86400;
            println!("   Token expires in: {} days", expires_in_days);
        } else {
            println!("   Token has expired");
        }
    }

    // Close the WebSocket connection gracefully
    rpc_client.close().await.ok();

    Ok(())
}
