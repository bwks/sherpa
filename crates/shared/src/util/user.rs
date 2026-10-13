use std::env;

use anyhow::{Result, anyhow};

use super::ssh::get_ssh_public_key;
use crate::data::User;
use crate::konst::{SHERPA_PASSWORD, SHERPA_SSH_PUBLIC_KEY_PATH, SHERPA_USERNAME};

/// Get the username of the current user from environment variables.
pub fn get_username() -> Result<String> {
    username_from(env::var("USER").ok(), env::var("USERNAME").ok())
}

/// Pick the username from the `USER` value, falling back to the `USERNAME` value.
fn username_from(user: Option<String>, username: Option<String>) -> Result<String> {
    user.or(username)
        .ok_or_else(|| anyhow!("Failed to get current user from environment variables"))
}
/// Returns the default sherpa user and set sudo to True.
pub fn sherpa_user() -> Result<User> {
    let username = SHERPA_USERNAME;
    let ssh_public_key = get_ssh_public_key(SHERPA_SSH_PUBLIC_KEY_PATH)?;
    Ok(User {
        username: username.to_owned(),
        password: Some(SHERPA_PASSWORD.to_owned()),
        ssh_public_key,
        sudo: true,
    })
}
// impl User {
//     /// Returns the default sherpa user and set sudo to True.
//     pub fn default() -> Result<User> {
//         let username = SHERPA_USERNAME;
//         let ssh_public_key =
//             get_ssh_public_key(&format!("{SHERPA_CONFIG_DIR}/{SHERPA_SSH_PUBLIC_KEY_FILE}"))?;
//         Ok(User {
//             username: username.to_owned(),
//             password: Some(SHERPA_PASSWORD.to_owned()),
//             ssh_public_key,
//             sudo: true,
//         })
//     }

//     /// Create a User.
//     #[allow(dead_code)]
//     pub fn new(
//         username: &str,
//         password: Option<&str>,
//         ssh_public_key_file: &str,
//         sudo: bool,
//     ) -> Result<User> {
//         let ssh_public_key = get_ssh_public_key(ssh_public_key_file)?;
//         Ok(User {
//             username: username.to_owned(),
//             password: password.map(|p| p.to_owned()),
//             ssh_public_key,
//             sudo,
//         })
//     }
// }

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_username_from_user() {
        let username = username_from(Some("testuser".to_owned()), None).unwrap();
        assert_eq!(username, "testuser");
    }

    #[test]
    fn test_username_from_user_wins_over_username() {
        let username =
            username_from(Some("testuser".to_owned()), Some("fallbackuser".to_owned())).unwrap();
        assert_eq!(username, "testuser");
    }

    #[test]
    fn test_username_from_username_fallback() {
        let username = username_from(None, Some("fallbackuser".to_owned())).unwrap();
        assert_eq!(username, "fallbackuser");
    }

    #[test]
    fn test_username_from_failure_when_both_unset() {
        let result = username_from(None, None);
        assert!(result.is_err());
    }
}
