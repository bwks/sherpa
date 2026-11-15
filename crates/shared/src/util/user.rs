use anyhow::Result;

use super::ssh::get_ssh_public_key;
use crate::data::User;
use crate::konst::{SHERPA_PASSWORD, SHERPA_USERNAME};

/// Normalise a Sherpa server username for storage and lookup.
///
/// Usernames are case-insensitive: they are trimmed and stored in lower case, and every lookup
/// lowercases its input the same way.
#[must_use]
pub fn normalize_username(username: &str) -> String {
    username.trim().to_lowercase()
}

/// The `sherpa` account on a lab's nodes, with sudo, logging in with the lab's own public key at
/// `public_key_path`.
///
/// # Errors
///
/// Fails when the SSH public key cannot be read.
pub fn sherpa_user(public_key_path: &str) -> Result<User> {
    let username = SHERPA_USERNAME;
    let ssh_public_key = get_ssh_public_key(public_key_path)?;
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
    fn the_sherpa_user_logs_in_with_the_given_public_key() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sherpa_ssh_key.pub");
        std::fs::write(
            &path,
            "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAICwUP1LOygXU4kLJLDSCQ68zDWZKYS3scLybuYvujhPD lab\n",
        )
        .unwrap();
        let user = sherpa_user(path.to_str().unwrap()).unwrap();
        assert_eq!(user.username, SHERPA_USERNAME);
        assert!(user.sudo);
        assert_eq!(
            user.ssh_public_key.key,
            "AAAAC3NzaC1lZDI1NTE5AAAAICwUP1LOygXU4kLJLDSCQ68zDWZKYS3scLybuYvujhPD"
        );
    }

    #[test]
    fn the_sherpa_user_needs_its_public_key() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("missing.pub");
        assert!(sherpa_user(path.to_str().unwrap()).is_err());
    }

    #[test]
    fn test_normalize_username_trims_and_lowercases() {
        assert_eq!(normalize_username("  BRadmin "), "bradmin");
        assert_eq!(normalize_username("bradmin"), "bradmin");
    }
}
