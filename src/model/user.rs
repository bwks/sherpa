use anyhow::Result;
use serde_derive::{Deserialize, Serialize};

use crate::core::konst::{SHERPA_SSH_PUBLIC_KEY_FILE, SHERPA_USERNAME, TEMP_DIR};
use crate::model::SshPublicKey;
use crate::util::get_ssh_public_key;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct User {
    pub username: String,
    pub ssh_public_key: SshPublicKey,
    pub sudo: bool,
}

impl User {
    /// Returns the default sherpa user and set sudo to True.
    pub fn default() -> Result<User> {
        let username = SHERPA_USERNAME;
        let ssh_public_key =
            get_ssh_public_key(&format!("{TEMP_DIR}/{SHERPA_SSH_PUBLIC_KEY_FILE}"))?;
        Ok(User {
            username: username.to_owned(),
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
