use anyhow::{Context, Result};
use clap::Subcommand;
use std::io::{self, Write};

use crate::cmd::cli::OutputFormat;
use crate::common::rpc::RpcClient;
use crate::token;
use shared::data::{self, ServerConnection};
use shared::util::emoji_success;

#[derive(Debug, Subcommand)]
pub enum UserCommands {
    /// Create a new user (admin only)
    Create {
        /// Username for the new user
        username: String,

        /// Make the user an administrator
        #[arg(long)]
        admin: bool,

        /// SSH public keys (can be specified multiple times)
        #[arg(long = "ssh-key")]
        ssh_keys: Vec<String>,
    },

    /// List all users (admin only)
    List,

    /// Delete a user (admin only)
    Delete {
        /// Username to delete
        username: String,

        /// Skip confirmation prompt
        #[arg(long)]
        force: bool,
    },

    /// Change user password
    Passwd {
        /// Username (defaults to current user)
        username: Option<String>,
    },

    /// Show user information
    Info {
        /// Username (defaults to current user)
        username: Option<String>,
    },
}

pub async fn user_commands(
    command: &UserCommands,
    server_url: &str,
    server_connection: &ServerConnection,
    output_format: &OutputFormat,
) -> Result<()> {
    match command {
        UserCommands::Create {
            username,
            admin,
            ssh_keys,
        } => {
            create_user(
                username,
                *admin,
                ssh_keys.clone(),
                server_url,
                server_connection,
                output_format,
            )
            .await
        }
        UserCommands::List => list_users(server_url, server_connection, output_format).await,
        UserCommands::Delete { username, force } => {
            delete_user(
                username,
                *force,
                server_url,
                server_connection,
                output_format,
            )
            .await
        }
        UserCommands::Passwd { username } => {
            change_password(
                username.as_deref(),
                server_url,
                server_connection,
                output_format,
            )
            .await
        }
        UserCommands::Info { username } => {
            get_user_info(
                username.as_deref(),
                server_url,
                server_connection,
                output_format,
            )
            .await
        }
    }
}

async fn create_user(
    username: &str,
    is_admin: bool,
    ssh_keys: Vec<String>,
    server_url: &str,
    server_connection: &ServerConnection,
    output_format: &OutputFormat,
) -> Result<()> {
    // Get token
    let token = token::load_token().context("Not authenticated. Please login first.")?;

    // Prompt for password from env or interactively
    let password = if let Ok(env_password) = std::env::var("SHERPA_USER_PASSWORD") {
        env_password
    } else {
        let password = rpassword::prompt_password(format!("Password for {}: ", username))
            .context("Failed to read password")?;
        let confirm =
            rpassword::prompt_password("Confirm password: ").context("Failed to read password")?;

        if password != confirm {
            anyhow::bail!("Passwords do not match");
        }
        password
    };

    let request = data::CreateUserRequest {
        username: username.to_string(),
        password,
        is_admin,
        ssh_keys: if ssh_keys.is_empty() {
            None
        } else {
            Some(ssh_keys)
        },
        token: token.clone(),
    };

    let rpc_client = RpcClient::new(server_url.to_string(), server_connection.clone());
    let response: data::CreateUserResponse = rpc_client
        .call("user.create", request, Some(token))
        .await
        .context("Failed to create user")?;

    match output_format {
        OutputFormat::Json => {
            println!("{}", serde_json::to_string_pretty(&response)?);
        }
        OutputFormat::Text => {
            println!("{}", emoji_success("User created successfully"));
            println!("   Username: {}", response.username);
            println!("   Admin: {}", if response.is_admin { "Yes" } else { "No" });
        }
    }

    Ok(())
}

async fn list_users(
    server_url: &str,
    server_connection: &ServerConnection,
    output_format: &OutputFormat,
) -> Result<()> {
    // Get token
    let token = token::load_token().context("Not authenticated. Please login first.")?;

    let request = data::ListUsersRequest {
        token: token.clone(),
    };

    let rpc_client = RpcClient::new(server_url.to_string(), server_connection.clone());
    let response: data::ListUsersResponse = rpc_client
        .call("user.list", request, Some(token))
        .await
        .context("Failed to list users")?;

    match output_format {
        OutputFormat::Json => {
            println!("{}", serde_json::to_string_pretty(&response)?);
        }
        OutputFormat::Text => {
            if response.users.is_empty() {
                println!("No users found");
            } else {
                println!("\n{} user(s) found:\n", response.users.len());
                for user in response.users {
                    println!("  • {}", user.username);
                    println!("    Admin: {}", if user.is_admin { "Yes" } else { "No" });
                    println!(
                        "    SSH Keys: {}",
                        if user.ssh_keys.is_empty() {
                            "None".to_string()
                        } else {
                            format!("{} key(s)", user.ssh_keys.len())
                        }
                    );
                    let created = jiff::Timestamp::from_second(user.created_at)
                        .ok()
                        .map(|ts| ts.strftime("%Y-%m-%d %H:%M:%S UTC").to_string())
                        .unwrap_or_else(|| "Unknown".to_string());
                    println!("    Created: {}", created);
                    println!();
                }
            }
        }
    }

    Ok(())
}

async fn delete_user(
    username: &str,
    force: bool,
    server_url: &str,
    server_connection: &ServerConnection,
    output_format: &OutputFormat,
) -> Result<()> {
    // Get token
    let token = token::load_token().context("Not authenticated. Please login first.")?;

    // Confirm deletion unless --force flag is set
    if !force {
        print!(
            "Are you sure you want to delete user '{}'? This action cannot be undone. [y/N]: ",
            username
        );
        io::stdout().flush()?;

        let mut response = String::new();
        io::stdin()
            .read_line(&mut response)
            .context("Failed to read confirmation")?;

        if !matches!(response.trim().to_lowercase().as_str(), "y" | "yes") {
            println!("Deletion cancelled");
            return Ok(());
        }
    }

    let request = data::DeleteUserRequest {
        username: username.to_string(),
        token: token.clone(),
    };

    let rpc_client = RpcClient::new(server_url.to_string(), server_connection.clone());
    let response: data::DeleteUserResponse = rpc_client
        .call("user.delete", request, Some(token))
        .await
        .context("Failed to delete user")?;

    match output_format {
        OutputFormat::Json => {
            println!("{}", serde_json::to_string_pretty(&response)?);
        }
        OutputFormat::Text => {
            println!(
                "{}",
                emoji_success(&format!(
                    "User '{}' deleted successfully",
                    response.username
                ))
            );
        }
    }

    Ok(())
}

async fn change_password(
    username: Option<&str>,
    server_url: &str,
    server_connection: &ServerConnection,
    output_format: &OutputFormat,
) -> Result<()> {
    // Get token
    let token = token::load_token().context("Not authenticated. Please login first.")?;

    // If no username provided, use current user from token
    let target_username = if let Some(name) = username {
        name.to_string()
    } else {
        // Parse JWT to get current username (simple base64 decode of payload)
        use base64::{Engine as _, engine::general_purpose};
        let parts: Vec<&str> = token.split('.').collect();
        if parts.len() != 3 {
            anyhow::bail!("Invalid token format");
        }
        let payload = general_purpose::STANDARD_NO_PAD
            .decode(parts[1])
            .context("Failed to decode token")?;
        let payload_str = String::from_utf8(payload).context("Invalid token payload")?;
        let payload_json: serde_json::Value =
            serde_json::from_str(&payload_str).context("Failed to parse token payload")?;
        payload_json["sub"]
            .as_str()
            .context("No username in token")?
            .to_string()
    };

    // Prompt for new password from env or interactively
    let new_password = if let Ok(env_password) = std::env::var("SHERPA_USER_PASSWORD") {
        env_password
    } else {
        let password =
            rpassword::prompt_password(format!("New password for {}: ", target_username))
                .context("Failed to read password")?;
        let confirm = rpassword::prompt_password("Confirm new password: ")
            .context("Failed to read password")?;

        if password != confirm {
            anyhow::bail!("Passwords do not match");
        }
        password
    };

    let request = data::ChangePasswordRequest {
        username: target_username.clone(),
        new_password,
        token: token.clone(),
    };

    let rpc_client = RpcClient::new(server_url.to_string(), server_connection.clone());
    let response: data::ChangePasswordResponse = rpc_client
        .call("user.passwd", request, Some(token))
        .await
        .context("Failed to change password")?;

    match output_format {
        OutputFormat::Json => {
            println!("{}", serde_json::to_string_pretty(&response)?);
        }
        OutputFormat::Text => {
            println!(
                "{}",
                emoji_success(&format!(
                    "Password changed successfully for user '{}'",
                    response.username
                ))
            );
        }
    }

    Ok(())
}

async fn get_user_info(
    username: Option<&str>,
    server_url: &str,
    server_connection: &ServerConnection,
    output_format: &OutputFormat,
) -> Result<()> {
    // Get token
    let token = token::load_token().context("Not authenticated. Please login first.")?;

    // If no username provided, use current user from token
    let target_username = if let Some(name) = username {
        name.to_string()
    } else {
        // Parse JWT to get current username (simple base64 decode of payload)
        use base64::{Engine as _, engine::general_purpose};
        let parts: Vec<&str> = token.split('.').collect();
        if parts.len() != 3 {
            anyhow::bail!("Invalid token format");
        }
        let payload = general_purpose::STANDARD_NO_PAD
            .decode(parts[1])
            .context("Failed to decode token")?;
        let payload_str = String::from_utf8(payload).context("Invalid token payload")?;
        let payload_json: serde_json::Value =
            serde_json::from_str(&payload_str).context("Failed to parse token payload")?;
        payload_json["sub"]
            .as_str()
            .context("No username in token")?
            .to_string()
    };

    let request = data::GetUserInfoRequest {
        username: target_username,
        token: token.clone(),
    };

    let rpc_client = RpcClient::new(server_url.to_string(), server_connection.clone());
    let response: data::GetUserInfoResponse = rpc_client
        .call("user.info", request, Some(token))
        .await
        .context("Failed to get user info")?;

    match output_format {
        OutputFormat::Json => {
            println!("{}", serde_json::to_string_pretty(&response)?);
        }
        OutputFormat::Text => {
            let user = response.user;
            println!("\nUser Information:");
            println!("  Username: {}", user.username);
            println!("  Admin: {}", if user.is_admin { "Yes" } else { "No" });
            println!(
                "  SSH Keys: {}",
                if user.ssh_keys.is_empty() {
                    "None".to_string()
                } else {
                    format!("{} key(s)", user.ssh_keys.len())
                }
            );
            if !user.ssh_keys.is_empty() {
                for (i, key) in user.ssh_keys.iter().enumerate() {
                    // Show first 40 chars of the key
                    let key_preview = if key.len() > 40 {
                        format!("{}...", &key[..40])
                    } else {
                        key.clone()
                    };
                    println!("    {}. {}", i + 1, key_preview);
                }
            }
            let created = jiff::Timestamp::from_second(user.created_at)
                .ok()
                .map(|ts| ts.strftime("%Y-%m-%d %H:%M:%S UTC").to_string())
                .unwrap_or_else(|| "Unknown".to_string());
            let updated = jiff::Timestamp::from_second(user.updated_at)
                .ok()
                .map(|ts| ts.strftime("%Y-%m-%d %H:%M:%S UTC").to_string())
                .unwrap_or_else(|| "Unknown".to_string());
            println!("  Created: {}", created);
            println!("  Updated: {}", updated);
            println!();
        }
    }

    Ok(())
}
