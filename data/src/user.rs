use anyhow::Result;
use serde_derive::{Deserialize, Serialize};

use super::SshPublicKey;
use crate::core::{
    SHERPA_CONFIG_DIR, SHERPA_PASSWORD, SHERPA_SSH_PUBLIC_KEY_FILE, SHERPA_USERNAME,
};
use crate::util::get_ssh_public_key;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct User {
    pub username: String,
    pub password: Option<String>,
    pub ssh_public_key: SshPublicKey,
    pub sudo: bool,
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
