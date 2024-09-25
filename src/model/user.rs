use anyhow::Result;
use serde_derive::{Deserialize, Serialize};

use crate::core::konst::{SSH_DIR, SSH_PUBLIC_KEY_FILE};
use crate::util::{get_ssh_public_key, get_username};

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct User {
    pub username: String,
    pub ssh_public_key: String,
    pub sudo: bool,
}

impl User {
    /// Default will return a User from the current user and set sudo to True.
    pub fn default() -> Result<User> {
        let username = get_username()?;
        let ssh_public_key = get_ssh_public_key(&format!("{SSH_DIR}/{SSH_PUBLIC_KEY_FILE}"))?;
        Ok(User {
            username,
            ssh_public_key,
            sudo: true,
        })
    }

    /// Create a User.
    #[allow(dead_code)]
    pub fn new(username: &str, ssh_public_key_file: &str, sudo: bool) -> Result<User> {
        let ssh_public_key = get_ssh_public_key(ssh_public_key_file)?;
        Ok(User {
            username: username.to_owned(),
            ssh_public_key,
            sudo,
        })
    }
}
