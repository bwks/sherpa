use serde_derive::{Deserialize, Serialize};

use super::ssh::SshPublicKey;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct User {
    pub username: String,
    pub password: Option<String>,
    pub ssh_public_key: SshPublicKey,
    pub sudo: bool,
}
