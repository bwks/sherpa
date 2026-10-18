use std::env;

use anyhow::{Result, anyhow};

use super::ssh::get_ssh_public_key;
use crate::data::User;
use crate::konst::{SHERPA_PASSWORD, SHERPA_SSH_PUBLIC_KEY_PATH, SHERPA_USERNAME};

/// Get the username of the current user from environment variables.
pub fn get_username() -> Result<String> {
    env::var("USER")
        .or_else(|_| env::var("USERNAME"))
        .map_err(|_| anyhow!("Failed to get current user from environment variables"))
}

/// Normalise a Sherpa server username for storage and lookup.
///
/// Usernames are case-insensitive: they are trimmed and stored in lower case, and every lookup
/// lowercases its input the same way.
pub fn normalize_username(username: &str) -> String {
    username.trim().to_lowercase()
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
    use std::env;

    use std::sync::Mutex;

    use super::*;

    /// Serialises the tests that change `USER` and `USERNAME`, which are process-wide.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn test_get_username_from_user() {
        let _guard = ENV_LOCK.lock().unwrap();
        // SAFETY: ENV_LOCK keeps the other environment tests from running concurrently.
        unsafe {
            env::set_var("USER", "testuser");
            env::remove_var("USERNAME");
        }
        let username = get_username().unwrap();
        assert_eq!(username, "testuser");
    }

    #[test]
    fn test_get_username_from_username_fallback() {
        let _guard = ENV_LOCK.lock().unwrap();
        // SAFETY: ENV_LOCK keeps the other environment tests from running concurrently.
        unsafe {
            env::remove_var("USER");
            env::set_var("USERNAME", "fallbackuser");
        }
        let username = get_username().unwrap();
        assert_eq!(username, "fallbackuser");
        unsafe {
            env::remove_var("USERNAME");
        }
    }

    #[test]
    fn test_get_username_failure_when_both_unset() {
        let _guard = ENV_LOCK.lock().unwrap();
        // SAFETY: ENV_LOCK keeps the other environment tests from running concurrently.
        unsafe {
            env::remove_var("USER");
            env::remove_var("USERNAME");
        }
        let result = get_username();
        assert!(result.is_err());
    }

    #[test]
    fn test_normalize_username_trims_and_lowercases() {
        assert_eq!(normalize_username("  BRadmin "), "bradmin");
        assert_eq!(normalize_username("bradmin"), "bradmin");
    }
}
