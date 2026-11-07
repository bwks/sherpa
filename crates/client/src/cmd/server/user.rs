use std::fs;
use std::path::Path;

use anyhow::{Context, Result, ensure};
use base64::{Engine as _, engine::general_purpose};
use clap::Subcommand;

use shared::auth::jwt::Claims;
use shared::data::{self, ServerConnection};
use shared::konst::SSH_KEYS_DOCS_URL;
use shared::util::{
    confirm, emoji_success, emoji_warning, expand_path, normalize_username, render_ssh_keys_table,
};
use shared::validators::validate_username;

use super::OutputFormat;
use super::rpc_call;
use crate::token::load_token;

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

    /// Manage SSH public keys. Labs only start for users with at least one key
    SshKey {
        #[command(subcommand)]
        command: SshKeyCommands,
    },
}

#[derive(Debug, Subcommand)]
pub enum SshKeyCommands {
    /// List SSH public keys and their fingerprints
    List {
        /// User whose keys to list (defaults to current user; admins may name any user)
        #[arg(long)]
        user: Option<String>,
    },

    /// Add an SSH public key
    Add {
        /// The public key, or the path to a public key file such as ~/.`ssh/id_ed25519.pub`
        key: String,

        /// User to add the key to (defaults to current user; admins may name any user)
        #[arg(long)]
        user: Option<String>,
    },

    /// Remove an SSH public key
    Remove {
        /// The public key, its SHA256 fingerprint, or the path to a public key file
        key: String,

        /// User to remove the key from (defaults to current user; admins may name any user)
        #[arg(long)]
        user: Option<String>,
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
        UserCommands::SshKey { command } => {
            ssh_key_commands(command, server_url, server_connection, output_format).await
        }
    }
}

async fn ssh_key_commands(
    command: &SshKeyCommands,
    server_url: &str,
    server_connection: &ServerConnection,
    output_format: &OutputFormat,
) -> Result<()> {
    match command {
        SshKeyCommands::List { user } => {
            let request = data::GetUserInfoRequest {
                username: target_user(user.as_deref())?,
                token: String::new(),
            };
            let response: data::GetUserInfoResponse =
                rpc_call("user.info", request, server_url, server_connection)
                    .await
                    .context("Failed to list SSH keys")?;
            print_ssh_keys(
                &response.user.username,
                &response.user.ssh_keys,
                None,
                output_format,
            )
        }
        SshKeyCommands::Add { key, user } => {
            let request = data::AddSshKeyRequest {
                username: target_user(user.as_deref())?,
                ssh_key: read_key_argument(key)?,
                token: String::new(),
            };
            let response: data::UserSshKeysResponse =
                rpc_call("user.ssh_key.add", request, server_url, server_connection)
                    .await
                    .context("Failed to add SSH key")?;
            print_ssh_keys(
                &response.username,
                &response.ssh_keys,
                Some("SSH key added"),
                output_format,
            )
        }
        SshKeyCommands::Remove { key, user } => {
            let request = data::RemoveSshKeyRequest {
                username: target_user(user.as_deref())?,
                ssh_key: read_key_argument(key)?,
                token: String::new(),
            };
            let response: data::UserSshKeysResponse = rpc_call(
                "user.ssh_key.remove",
                request,
                server_url,
                server_connection,
            )
            .await
            .context("Failed to remove SSH key")?;
            print_ssh_keys(
                &response.username,
                &response.ssh_keys,
                Some("SSH key removed"),
                output_format,
            )
        }
    }
}

/// The named user, or the logged-in user.
fn target_user(user: Option<&str>) -> Result<String> {
    match user {
        Some(name) => Ok(name.to_string()),
        None => username_from_token(&load_token()?),
    }
}

/// A key argument is the key itself, or the path to a public key file.
fn read_key_argument(argument: &str) -> Result<String> {
    let path = expand_path(argument);
    if Path::new(&path).is_file() {
        let key = fs::read_to_string(&path)
            .context(format!("Failed to read SSH public key file '{path}'"))?;
        return Ok(key.trim().to_string());
    }
    Ok(argument.trim().to_string())
}

fn print_ssh_keys(
    username: &str,
    ssh_keys: &[String],
    message: Option<&str>,
    output_format: &OutputFormat,
) -> Result<()> {
    match output_format {
        OutputFormat::Json => {
            let response = data::UserSshKeysResponse {
                username: username.to_string(),
                ssh_keys: ssh_keys.to_vec(),
            };
            println!("{}", serde_json::to_string_pretty(&response)?);
        }
        OutputFormat::Text => {
            if let Some(message) = message {
                println!("{}", emoji_success(message));
            }
            if ssh_keys.is_empty() {
                println!(
                    "{}",
                    emoji_warning(&format!(
                        "'{username}' has no SSH keys, so labs will not start. Add one with \
                         'sherpa server user ssh-key add <key or .pub file>'. See {SSH_KEYS_DOCS_URL}"
                    ))
                );
            } else {
                println!("{}", render_ssh_keys_table(username, ssh_keys));
            }
        }
    }
    Ok(())
}

async fn create_user(
    username: &str,
    is_admin: bool,
    ssh_keys: Vec<String>,
    server_url: &str,
    server_connection: &ServerConnection,
    output_format: &OutputFormat,
) -> Result<()> {
    // Check the name before prompting for a password; the server enforces the same rule.
    let username = normalize_username(username);
    validate_username(&username)?;

    let password = prompt_new_password(&format!("Password for {username}: "))?;

    let request = data::CreateUserRequest {
        username,
        password,
        is_admin,
        ssh_keys,
        token: String::new(),
    };

    let response: data::CreateUserResponse =
        rpc_call("user.create", request, server_url, server_connection)
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
    let request = data::ListUsersRequest {
        token: String::new(),
    };

    let response: data::ListUsersResponse =
        rpc_call("user.list", request, server_url, server_connection)
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
                for user in &response.users {
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
                        .map_or_else(
                            || "Unknown".to_string(),
                            |ts| ts.strftime("%Y-%m-%d %H:%M:%S UTC").to_string(),
                        );
                    println!("    Created: {created}");
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
    // Confirm deletion unless --force flag is set
    if !force
        && !confirm(&format!(
            "Are you sure you want to delete user '{username}'? This action cannot be undone."
        ))?
    {
        println!("Deletion cancelled");
        return Ok(());
    }

    let request = data::DeleteUserRequest {
        username: username.to_string(),
        token: String::new(),
    };

    let response: data::DeleteUserResponse =
        rpc_call("user.delete", request, server_url, server_connection)
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
    // If no username provided, use current user from token
    let target_username = target_user(username)?;
    // The server asks for the current password when you change your own, admins included.
    let own_account = normalize_username(&target_username) == username_from_token(&load_token()?)?;
    let current_password = if own_account {
        Some(
            rpassword::prompt_password(format!("Current password for {target_username}: "))
                .context("Failed to read password")?,
        )
    } else {
        None
    };

    let new_password = prompt_new_password(&format!("New password for {target_username}: "))?;

    let request = data::ChangePasswordRequest {
        username: target_username,
        new_password,
        current_password,
        token: String::new(),
    };

    let response: data::ChangePasswordResponse =
        rpc_call("user.passwd", request, server_url, server_connection)
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
    // If no username provided, use current user from token
    let target_username = target_user(username)?;

    let request = data::GetUserInfoRequest {
        username: target_username,
        token: String::new(),
    };

    let response: data::GetUserInfoResponse =
        rpc_call("user.info", request, server_url, server_connection)
            .await
            .context("Failed to get user info")?;

    match output_format {
        OutputFormat::Json => {
            println!("{}", serde_json::to_string_pretty(&response)?);
        }
        OutputFormat::Text => {
            let user = &response.user;
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
                .map_or_else(
                    || "Unknown".to_string(),
                    |ts| ts.strftime("%Y-%m-%d %H:%M:%S UTC").to_string(),
                );
            let updated = jiff::Timestamp::from_second(user.updated_at)
                .ok()
                .map_or_else(
                    || "Unknown".to_string(),
                    |ts| ts.strftime("%Y-%m-%d %H:%M:%S UTC").to_string(),
                );
            println!("  Created: {created}");
            println!("  Updated: {updated}");
            println!();
        }
    }

    Ok(())
}

/// The username a login `token` is for: its `sub` claim. The signature is not checked: this only
/// picks the default user, and the server checks the token on every call.
fn username_from_token(token: &str) -> Result<String> {
    let payload = token
        .split('.')
        .nth(1)
        .context("The login token is not a JWT; run: sherpa login")?;
    let payload = general_purpose::URL_SAFE_NO_PAD
        .decode(payload)
        .context("Failed to decode the login token")?;
    let claims: Claims =
        serde_json::from_slice(&payload).context("Failed to parse the login token")?;
    Ok(claims.sub)
}

/// Ask for a new password with `prompt`, then again to confirm it. It is always typed, never
/// taken from `SHERPA_USER_PASSWORD`, which holds the caller's own password for `login`.
fn prompt_new_password(prompt: &str) -> Result<String> {
    let password = rpassword::prompt_password(prompt).context("Failed to read password")?;
    let confirm =
        rpassword::prompt_password("Confirm password: ").context("Failed to read password")?;
    confirmed_password(password, &confirm)
}

/// The new `password`, if it is not empty and `confirm` matches it.
fn confirmed_password(password: String, confirm: &str) -> Result<String> {
    ensure!(!password.is_empty(), "The password cannot be empty");
    ensure!(password == confirm, "Passwords do not match");
    Ok(password)
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::*;

    #[test]
    fn the_username_is_read_from_a_base64url_token() {
        let claims = br#"{"sub":"dev","exp":1,"iat":1,"is_admin":false,"x":"??>>~~"}"#;
        let payload = general_purpose::URL_SAFE_NO_PAD.encode(claims);
        assert!(payload.contains(['-', '_']), "{payload}");
        let token = format!("header.{payload}.signature");
        assert_eq!(username_from_token(&token).unwrap(), "dev");
    }

    #[test]
    fn a_token_that_is_not_a_jwt_is_refused() {
        assert!(username_from_token("not-a-jwt").is_err());
    }

    #[test]
    fn a_new_password_must_be_typed_the_same_twice() {
        assert_eq!(
            confirmed_password("s3cret".to_owned(), "s3cret").unwrap(),
            "s3cret"
        );
        let error = confirmed_password("s3cret".to_owned(), "s3crex").unwrap_err();
        assert_eq!(error.to_string(), "Passwords do not match");
    }

    #[test]
    fn an_empty_new_password_is_refused() {
        let error = confirmed_password(String::new(), "").unwrap_err();
        assert_eq!(error.to_string(), "The password cannot be empty");
    }

    const KEY: &str =
        "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIJbRB5JhR5JhR5JhR5JhR5JhR5JhR5JhR5JhR5JhR5Jh me@host";

    #[derive(Debug, Parser)]
    struct TestCli {
        #[command(subcommand)]
        command: UserCommands,
    }

    #[test]
    fn key_argument_reads_a_public_key_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("id_ed25519.pub");
        fs::write(&path, format!("{KEY}\n")).unwrap();
        assert_eq!(read_key_argument(path.to_str().unwrap()).unwrap(), KEY);
    }

    #[test]
    fn key_argument_passes_keys_and_fingerprints_through() {
        assert_eq!(read_key_argument(&format!(" {KEY} ")).unwrap(), KEY);
        assert_eq!(
            read_key_argument("SHA256:/UpBoWzMiq6/+/wzQIGDT2VxOLzg/LUTfKhc0x+ktEs").unwrap(),
            "SHA256:/UpBoWzMiq6/+/wzQIGDT2VxOLzg/LUTfKhc0x+ktEs"
        );
    }

    #[test]
    fn ssh_key_subcommands_parse() {
        let cli = TestCli::try_parse_from(["user", "ssh-key", "add", "key.pub", "--user", "alice"])
            .unwrap();
        assert!(matches!(
            cli.command,
            UserCommands::SshKey {
                command: SshKeyCommands::Add { ref key, user: Some(ref user) }
            } if key == "key.pub" && user == "alice"
        ));
    }
}
