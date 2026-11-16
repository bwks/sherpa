/// The lab owner's login on a network device: an administrator who logs in by key, or with
/// their device password.
#[derive(Clone, Debug)]
pub struct OwnerUser {
    pub username: String,
    /// The owner's device password, in the form the device takes it: the password itself, or a
    /// salted crypt hash of it. A device's fixed accounts, such as `admin` or `root`, take it too.
    pub password: String,
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
