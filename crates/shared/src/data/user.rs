/// The lab owner's login on a network device: an administrator who logs in by key, or with
/// their device password.
#[derive(Clone, Debug)]
pub struct OwnerUser {
    pub username: String,
    pub password: String,
    pub ssh_keys: Vec<OwnerSshKey>,
}

/// One of the owner's public keys, in the form the device's config takes it.
#[derive(Clone, Debug)]
pub struct OwnerSshKey {
    pub kind: String,
    pub value: String,
}
