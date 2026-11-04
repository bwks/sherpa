use serde_derive::{Deserialize, Serialize};

use super::ssh::SshPublicKey;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct User {
    pub username: String,
    pub password: Option<String>,
    pub ssh_public_key: SshPublicKey,
    pub sudo: bool,
}

/// The lab owner's login on a network device, rendered next to the `sherpa` user. It has
/// administrator rights and logs in by key.
#[derive(Clone, Debug)]
pub struct OwnerUser {
    pub username: String,
    /// A random password nobody is told, for devices that need one.
    pub secret: String,
    pub ssh_keys: Vec<OwnerSshKey>,
}

/// One of the owner's public keys, in the form the device's config takes it.
#[derive(Clone, Debug)]
pub struct OwnerSshKey {
    /// The key type as the device names it, such as `ssh-rsa`, or `ssh-ecdsa` on Junos.
    pub kind: String,
    /// The key as the device takes it: the base64 key data, a hash of it, or a whole key line.
    pub value: String,
}
