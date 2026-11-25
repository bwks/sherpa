use std::fs;
use std::io;
use std::path::Path;

use anyhow::{Context, Result, ensure};
use clap::Subcommand;
use jiff::Timestamp;

use shared::data::{self, ServerConnection};
use shared::konst::SSH_KEYS_DOCS_URL;
use shared::util::{confirm, emoji_success, emoji_warning, expand_path, normalize_username};
use shared::validators::validate_username;

use super::OutputFormat;
use super::rpc_call;
use crate::error::ClientError;
use crate::table::render_ssh_keys_table;
use crate::token::{load_token, username_from_token};

#[derive(Debug, Subcommand)]
pub enum UserCommands {
    /// Create a new user (admin only). The user adds their own SSH keys
    Create {
        /// Username for the new user
        username: String,

        /// Make the user an administrator
        #[arg(long)]
        admin: bool,
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

    /// Unlock an account that failed logins locked (admin only)
    Unlock {
        /// Username to unlock
        username: String,
    },

    /// Change user password
    Passwd {
        /// Username (defaults to current user)
        username: Option<String>,
    },

    /// Set your own device password, your login password on lab nodes. Prompts for your Sherpa
    /// login password, then the new device password twice
    DevicePassword,

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

    /// Add an SSH public key to your own account. Nobody, admins included, can add a key to
    /// another account
    Add {
        /// The public key, or the path to a public key file such as ~/.`ssh/id_ed25519.pub`
        key: String,
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
        UserCommands::Create { username, admin } => {
            create_user(
                username,
                *admin,
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
        UserCommands::Unlock { username } => {
            unlock_user(username, server_url, server_connection, output_format).await
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
        UserCommands::DevicePassword => {
            set_device_password(server_url, server_connection, output_format).await
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
        SshKeyCommands::Add { key } => {
            let request = data::AddSshKeyRequest {
                username: username_from_token(&load_token()?)?,
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
                    let created = Timestamp::from_second(user.created_at).ok().map_or_else(
                        || "Unknown".to_string(),
                        |ts| ts.strftime("%Y-%m-%d %H:%M:%S UTC").to_string(),
                    );
                    println!(
                        "    Device password: {}",
                        device_password_state(user.device_password_set)
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

async fn unlock_user(
    username: &str,
    server_url: &str,
    server_connection: &ServerConnection,
    output_format: &OutputFormat,
) -> Result<()> {
    let request = data::UnlockUserRequest {
        username: username.to_string(),
        token: String::new(),
    };

    let response: data::UnlockUserResponse =
        rpc_call("user.unlock", request, server_url, server_connection)
            .await
            .context("Failed to unlock user")?;

    match output_format {
        OutputFormat::Json => {
            println!("{}", serde_json::to_string_pretty(&response)?);
        }
        OutputFormat::Text => {
            println!(
                "{}",
                emoji_success(&format!("User '{}' unlocked", response.username))
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
                .context(ClientError::PasswordNotRead)?,
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
                    "Password changed successfully for user '{}'. Their other login sessions \
                     have ended",
                    response.username
                ))
            );
        }
    }

    Ok(())
}

async fn set_device_password(
    server_url: &str,
    server_connection: &ServerConnection,
    output_format: &OutputFormat,
) -> Result<()> {
    // Only your own: the server refuses any other account, admins included.
    let username = username_from_token(&load_token()?)?;
    let request = device_password_request(username, rpassword::prompt_password)?;

    let response: data::SetDevicePasswordResponse = rpc_call(
        "user.device_password.set",
        request,
        server_url,
        server_connection,
    )
    .await
    .context("Failed to set the device password")?;

    match output_format {
        OutputFormat::Json => {
            println!("{}", serde_json::to_string_pretty(&response)?);
        }
        OutputFormat::Text => {
            println!(
                "{}",
                emoji_success(&format!(
                    "Device password set for user '{}'. It applies to nodes you create or \
                     redeploy from now on",
                    response.username
                ))
            );
        }
    }

    Ok(())
}

/// The request that sets `username`'s device password, its passwords read with `prompt`: the
/// Sherpa login password, then the new device password and again to confirm it. They are
/// always typed, never flags or `SHERPA_USER_PASSWORD`, as for `passwd`.
fn device_password_request(
    username: String,
    mut prompt: impl FnMut(String) -> io::Result<String>,
) -> Result<data::SetDevicePasswordRequest> {
    let login_password = prompt(format!("Sherpa login password for {username}: "))
        .context(ClientError::PasswordNotRead)?;
    let device_password =
        prompt("New device password: ".to_owned()).context(ClientError::PasswordNotRead)?;
    let confirm =
        prompt("Confirm device password: ".to_owned()).context(ClientError::PasswordNotRead)?;
    let device_password = confirmed_password(device_password, &confirm)?;
    Ok(data::SetDevicePasswordRequest {
        username,
        login_password,
        device_password,
        token: String::new(),
    })
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
            let created = Timestamp::from_second(user.created_at).ok().map_or_else(
                || "Unknown".to_string(),
                |ts| ts.strftime("%Y-%m-%d %H:%M:%S UTC").to_string(),
            );
            let updated = Timestamp::from_second(user.updated_at).ok().map_or_else(
                || "Unknown".to_string(),
                |ts| ts.strftime("%Y-%m-%d %H:%M:%S UTC").to_string(),
            );
            println!(
                "  Device password: {}",
                device_password_state(user.device_password_set)
            );
            println!("  Created: {created}");
            println!("  Updated: {updated}");
            println!();
        }
    }

    Ok(())
}

/// Whether a device password is set, as the CLI prints it.
fn device_password_state(set: bool) -> &'static str {
    if set { "set" } else { "not set" }
}

/// Ask for a new password with `prompt`, then again to confirm it. It is always typed, never
/// taken from `SHERPA_USER_PASSWORD`, which holds the caller's own password for `login`.
fn prompt_new_password(prompt: &str) -> Result<String> {
    let password = rpassword::prompt_password(prompt).context(ClientError::PasswordNotRead)?;
    let confirm =
        rpassword::prompt_password("Confirm password: ").context(ClientError::PasswordNotRead)?;
    confirmed_password(password, &confirm)
}

/// The new `password`, if it is not empty and `confirm` matches it.
fn confirmed_password(password: String, confirm: &str) -> Result<String> {
    ensure!(!password.is_empty(), ClientError::EmptyPassword);
    ensure!(password == confirm, ClientError::PasswordsDiffer);
    Ok(password)
}

#[cfg(test)]
mod tests {
    use clap::Parser;
    use shared::util::ssh_key_fingerprint;

    use super::*;

    /// A prompt that answers each question with the next of `answers`, recording the questions.
    fn scripted<'a>(
        answers: &'a [&'a str],
        asked: &'a mut Vec<String>,
    ) -> impl FnMut(String) -> io::Result<String> + 'a {
        let mut answers = answers.iter();
        move |question| {
            asked.push(question);
            answers
                .next()
                .map(|answer| (*answer).to_owned())
                .ok_or_else(|| io::Error::other("no more answers"))
        }
    }

    const KEY: &str =
        "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIJbRB5JhR5JhR5JhR5JhR5JhR5JhR5JhR5JhR5JhR5Jh me@host";

    #[derive(Debug, Parser)]
    struct TestCli {
        #[command(subcommand)]
        command: UserCommands,
    }

    #[test]
    fn device_password_013_the_cli_asks_for_both_passwords_and_takes_no_flag_for_them() {
        let mut asked = Vec::new();
        let request = device_password_request(
            "alice".to_owned(),
            scripted(
                &["Login-Pass-1!", "Granite-Owl42", "Granite-Owl42"],
                &mut asked,
            ),
        )
        .unwrap();
        assert_eq!(
            asked,
            [
                "Sherpa login password for alice: ",
                "New device password: ",
                "Confirm device password: ",
            ]
        );
        assert_eq!(request.username, "alice");
        assert_eq!(request.login_password, "Login-Pass-1!");
        assert_eq!(request.device_password, "Granite-Owl42");

        let mut asked = Vec::new();
        let error = device_password_request(
            "alice".to_owned(),
            scripted(
                &["Login-Pass-1!", "Granite-Owl42", "Granite-Owl43"],
                &mut asked,
            ),
        )
        .unwrap_err();
        assert!(matches!(
            error.downcast_ref::<ClientError>(),
            Some(ClientError::PasswordsDiffer)
        ));
        let mut asked = Vec::new();
        let error = device_password_request(
            "alice".to_owned(),
            scripted(&["Login-Pass-1!", "", ""], &mut asked),
        )
        .unwrap_err();
        assert!(matches!(
            error.downcast_ref::<ClientError>(),
            Some(ClientError::EmptyPassword)
        ));

        let mut asked = Vec::new();
        let error =
            device_password_request("alice".to_owned(), scripted(&["Login-Pass-1!"], &mut asked))
                .unwrap_err();
        assert!(matches!(
            error.downcast_ref::<ClientError>(),
            Some(ClientError::PasswordNotRead)
        ));
        assert_eq!(asked.len(), 2, "it asked on after a failed read: {asked:?}");

        let cli = TestCli::try_parse_from(["user", "device-password"]).unwrap();
        assert!(matches!(cli.command, UserCommands::DevicePassword));
        for extra in [["alice"], ["--password"], ["--device-password"]] {
            let mut args = vec!["user", "device-password"];
            args.extend(extra);
            assert!(TestCli::try_parse_from(args).is_err(), "{extra:?}");
        }
    }

    #[test]
    fn account_keys_002_a_key_file_or_fingerprint_is_read_and_the_keys_are_listed() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("id_ed25519.pub");
        fs::write(&path, format!("{KEY}\n")).unwrap();
        assert_eq!(read_key_argument(path.to_str().unwrap()).unwrap(), KEY);

        assert_eq!(read_key_argument(&format!(" {KEY} ")).unwrap(), KEY);
        assert_eq!(
            read_key_argument("SHA256:/UpBoWzMiq6/+/wzQIGDT2VxOLzg/LUTfKhc0x+ktEs").unwrap(),
            "SHA256:/UpBoWzMiq6/+/wzQIGDT2VxOLzg/LUTfKhc0x+ktEs"
        );

        let keys = vec![
            "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIJbRB5JhR5JhR5JhR5JhR5JhR5JhR5JhR5JhR5JhR5Jh brad@laptop"
                .to_string(),
            "ssh-rsa broken".to_string(),
        ];
        let table = render_ssh_keys_table("brad", &keys);
        assert!(table.contains("SSH Keys for brad"));
        assert!(table.contains("SHA256:/UpBoWzMiq6/+/wzQIGDT2VxOLzg/LUTfKhc0x+ktEs"));
        assert!(table.contains("brad@laptop"));
        assert!(table.contains("invalid key"));

        assert_eq!(
            ssh_key_fingerprint(&keys[0]).unwrap(),
            "SHA256:/UpBoWzMiq6/+/wzQIGDT2VxOLzg/LUTfKhc0x+ktEs"
        );
        assert!(ssh_key_fingerprint("not a key").is_err());
    }
}
