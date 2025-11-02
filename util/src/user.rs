use std::env;

use anyhow::{Result, anyhow};

use crate::data::User;

/// Get the username of the current user from environment variables.
pub fn get_username() -> Result<String> {
    env::var("USER")
        .or_else(|_| env::var("USERNAME"))
        .map_err(|_| anyhow!("Failed to get current user from environment variables"))
}

impl User {
    /// Returns the default sherpa user and set sudo to True.
    pub fn default() -> Result<User> {
        let username = SHERPA_USERNAME;
        let ssh_public_key =
            get_ssh_public_key(&format!("{SHERPA_CONFIG_DIR}/{SHERPA_SSH_PUBLIC_KEY_FILE}"))?;
        Ok(User {
            username: username.to_owned(),
            password: Some(SHERPA_PASSWORD.to_owned()),
            ssh_public_key,
            sudo: true,
        })
    }

    /// Create a User.
    #[allow(dead_code)]
    pub fn new(
        username: &str,
        password: Option<&str>,
        ssh_public_key_file: &str,
        sudo: bool,
    ) -> Result<User> {
        let ssh_public_key = get_ssh_public_key(ssh_public_key_file)?;
        Ok(User {
            username: username.to_owned(),
            password: password.map(|p| p.to_owned()),
            ssh_public_key,
            sudo,
        })
    }
}

// #[cfg(test)]
// mod tests {
//     use super::*;
//     use std::env;

//     #[test]
//     fn test_get_username_from_user() {
//         env::set_var("USER", "testuser");
//         env::remove_var("USERNAME");
//         let username = get_username().unwrap();
//         assert_eq!(username, "testuser");
//         env::remove_var("USER");
//     }

//     #[test]
//     fn test_get_username_from_username() {
//         env::remove_var("USER");
//         env::set_var("USERNAME", "testuser");
//         let username = get_username().unwrap();
//         assert_eq!(username, "testuser");
//         env::remove_var("USERNAME");
//     }

//     #[test]
//     fn test_get_username_failure() {
//         env::remove_var("USER");
//         env::remove_var("USERNAME");
//         let result = get_username();
//         assert!(result.is_err());
//     }
// }
