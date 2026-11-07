use std::env;

use anyhow::{Result, anyhow};

use super::ssh::get_ssh_public_key;
use crate::data::User;
use crate::konst::{SHERPA_PASSWORD, SHERPA_SSH_PUBLIC_KEY_PATH, SHERPA_USERNAME};

/// Get the username of the current user from environment variables.
///
/// # Errors
///
/// Fails when neither `USER` nor `USERNAME` (Windows) is set.
pub fn get_username() -> Result<String> {
    username_from(env::var("USER").ok(), env::var("USERNAME").ok())
}

/// The current user's name from the values of `USER` and `USERNAME`, preferring `USER`.
fn username_from(user: Option<String>, username: Option<String>) -> Result<String> {
    user.or(username)
        .ok_or_else(|| anyhow!("Failed to get current user from environment variables"))
}

/// Normalise a Sherpa server username for storage and lookup.
///
/// Usernames are case-insensitive: they are trimmed and stored in lower case, and every lookup
/// lowercases its input the same way.
#[must_use]
pub fn normalize_username(username: &str) -> String {
    username.trim().to_lowercase()
}

/// Returns the default sherpa user and set sudo to True.
///
/// # Errors
///
/// Fails when Sherpa's SSH public key cannot be read.
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_username_from_user() {
        let username = username_from(Some("testuser".into()), Some("other".into())).unwrap();
        assert_eq!(username, "testuser");
    }

    #[test]
    fn test_get_username_from_username_fallback() {
        let username = username_from(None, Some("fallbackuser".into())).unwrap();
        assert_eq!(username, "fallbackuser");
    }

    #[test]
    fn test_get_username_failure_when_both_unset() {
        assert!(username_from(None, None).is_err());
    }

    #[test]
    fn test_normalize_username_trims_and_lowercases() {
        assert_eq!(normalize_username("  BRadmin "), "bradmin");
        assert_eq!(normalize_username("bradmin"), "bradmin");
    }
}
