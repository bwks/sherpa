//! Resolve the lab owner's guest identity before provisioning changes resources.
use std::collections::HashSet;

use anyhow::{Context, Result, bail, ensure};
use rand::Rng;
use rand::distributions::{Alphanumeric, DistString};
use rand::seq::SliceRandom;
use ssh_key::{Algorithm, PublicKey};
use tracing::instrument;

use shared::data::{
    DbUser, NodeConfig, NodeKind, NodeModel, OwnerSshKey, OwnerUser, RecordId, ZtpMethod,
};
use shared::konst::{
    NODE_CLOUD_INIT_SUDO_RULE, NODE_DOAS_CONF_DIR, NODE_NIXOS_SHELL, NODE_NO_PASSWORD_HASH,
    RESERVED_USERNAMES, SHERPA_USERNAME, SSH_KEYS_DOCS_URL,
};
use shared::util::{
    base64_encode, base64_encode_bytes, pub_ssh_key_to_md5_hash, pub_ssh_key_to_sha256_hash,
};
use shared::validators::validate_guest_username;
use shared::validators::validate_ssh_key;
use template::{
    CloudInitUser, CloudInitWriteFile, CloudbaseInitUser, IgnitionFile, IgnitionFileContents,
    IgnitionUser,
};

/// Length of the random password given to owner accounts on devices that need one.
/// cloudbase-init takes at most 20 characters, and Cisco IOS 25.
const OWNER_SECRET_LENGTH: usize = 20;
/// The characters of a crypt(3) hash.
const CRYPT_ALPHABET: &[u8] = b"./0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";
const SHA256_CRYPT_SALT_LENGTH: usize = 16;
const SHA256_CRYPT_HASH_LENGTH: usize = 43;
const ANY_DEVICE_KEY: &[KeyType] = &[KeyType::Ed25519, KeyType::Ecdsa, KeyType::Rsa];
const RSA_KEY: &[KeyType] = &[KeyType::Rsa];
const ECDSA_OR_RSA_KEY: &[KeyType] = &[KeyType::Ecdsa, KeyType::Rsa];
const ED25519_OR_RSA_KEY: &[KeyType] = &[KeyType::Ed25519, KeyType::Rsa];

#[derive(Debug, Clone)]
pub(crate) struct GuestAccount {
    pub username: String,
    pub ssh_keys: Vec<String>,
}

/// The owner's account as one node takes it.
#[derive(Debug, Clone)]
pub(crate) struct NodeAccount {
    /// The login, with only the keys the node accepts.
    pub account: GuestAccount,
    /// The owner's keys the node does not accept, by key type, such as `ssh-ed25519`.
    pub skipped_keys: Vec<String>,
}

/// The SSH key types a lab node can take for a login.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum KeyType {
    Ed25519,
    Ecdsa,
    Rsa,
}

/// The key types a model accepts for the owner's login.
#[derive(Debug, Clone, Copy)]
enum AcceptedKeys {
    /// Every key the owner registered, as on Linux and BSD guests.
    All,
    /// Only these types, most preferred first.
    Only(&'static [KeyType]),
}

/// The owner keys a model takes: their types, and how many.
#[derive(Debug, Clone, Copy)]
struct KeyRules {
    accepted: AcceptedKeys,
    /// The most keys one login can have; the owner's first accepted keys are used.
    max_keys: usize,
}

/// How a device's config writes a public key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum KeyFormat {
    /// The key type and the base64 key data, as in an OpenSSH `authorized_keys` line.
    OpenSsh,
    /// Cisco IOS and IOS XE `key-hash`: an uppercase MD5 hash of the key data.
    CiscoMd5,
    /// Cisco ASA `hashed` keys: a SHA-256 hash of the key data.
    CiscoSha256,
    /// Junos: `ssh-rsa`, `ssh-ecdsa` or `ssh-ed25519`, then the whole key line.
    Junos,
    /// PAN-OS: the whole key line, base64-encoded.
    PanosBase64,
}

/// Whether the image's ZTP generator adds the owner's account. Keep this list aligned with the
/// model branches in the ZTP generators. Cisco FTDv, Talos, the application containers and
/// unikernels have no way to add an account at boot.
#[instrument(level = "debug", skip(image))]
pub(crate) fn supports_guest_account(image: &NodeConfig) -> bool {
    if !image.ztp_enable {
        return false;
    }
    match image.kind {
        NodeKind::VirtualMachine => match image.ztp_method {
            ZtpMethod::CloudInit => matches!(
                image.model,
                NodeModel::CentosLinux
                    | NodeModel::AlmaLinux
                    | NodeModel::DevboxLinux
                    | NodeModel::RockyLinux
                    | NodeModel::FedoraLinux
                    | NodeModel::OpensuseLinux
                    | NodeModel::RedhatLinux
                    | NodeModel::SuseLinux
                    | NodeModel::UbuntuLinux
                    | NodeModel::OmarchyLinux
                    | NodeModel::NixosLinux
                    | NodeModel::KaliLinux
                    | NodeModel::InfrahubServer
                    | NodeModel::JenkinsServer
                    | NodeModel::NautobotServer
                    | NodeModel::NetboxServer
                    | NodeModel::SignozServer
                    | NodeModel::VirtServer
                    | NodeModel::AlpineLinux
                    | NodeModel::FreeBsd
                    | NodeModel::OpenBsd
                    | NodeModel::GenericVm
                    | NodeModel::DevboxWindows
                    | NodeModel::WindowsServer
            ),
            ZtpMethod::Ignition => image.model == NodeModel::FlatcarLinux,
            ZtpMethod::Tftp => matches!(
                image.model,
                NodeModel::AristaVeos
                    | NodeModel::ArubaAoscx
                    | NodeModel::JuniperVevolved
                    | NodeModel::JuniperVsrxv3
            ),
            ZtpMethod::Cdrom => matches!(
                image.model,
                NodeModel::CiscoCsr1000v
                    | NodeModel::CiscoCat8000v
                    | NodeModel::CiscoCat9000v
                    | NodeModel::CiscoAsav
                    | NodeModel::CiscoNexus9300v
                    | NodeModel::CiscoIosxrv9000
                    | NodeModel::JuniperVsrxv3
                    | NodeModel::JuniperVrouter
                    | NodeModel::JuniperVswitch
                    | NodeModel::PaloaltoPanos
            ),
            ZtpMethod::Disk => matches!(
                image.model,
                NodeModel::CiscoIosv
                    | NodeModel::CiscoIosvl2
                    | NodeModel::CiscoIse
                    | NodeModel::MikrotikChr
            ),
            ZtpMethod::Usb => matches!(
                image.model,
                NodeModel::CumulusLinux | NodeModel::JuniperVevolved
            ),
            ZtpMethod::Http => image.model == NodeModel::SonicLinux,
            ZtpMethod::NoCloud | ZtpMethod::Volume | ZtpMethod::Ipxe | ZtpMethod::None => false,
        },
        NodeKind::Container => matches!(
            image.model,
            NodeModel::AristaCeos
                | NodeModel::NokiaSrlinux
                | NodeModel::FrrLinux
                | NodeModel::JuniperCevolved
        ),
        NodeKind::Unikernel => false,
    }
}

/// Whether the node gets the owner's account: its image supports one and the node has no custom
/// ZTP config, which replaces the generated config the account is written into.
#[instrument(level = "debug", skip(node, image))]
pub(crate) fn provisions_owner(node: &topology::NodeExpanded, image: &NodeConfig) -> bool {
    supports_guest_account(image) && node.ztp_config.is_none()
}

/// The owner keys a model takes, from each vendor's documentation of user public keys.
fn key_rules(model: &NodeModel) -> KeyRules {
    let (accepted, max_keys) = match model {
        // IOS `key-hash` and IOS XE take RSA only, two keys per user.
        NodeModel::CiscoIosv
        | NodeModel::CiscoIosvl2
        | NodeModel::CiscoCsr1000v
        | NodeModel::CiscoCat8000v
        | NodeModel::CiscoCat9000v => (AcceptedKeys::Only(RSA_KEY), 2),
        // IOS XR takes four RSA keys: the first, then `second`, `third` and `fourth`.
        NodeModel::CiscoIosxrv9000 => (AcceptedKeys::Only(RSA_KEY), 4),
        NodeModel::CiscoAsav => (AcceptedKeys::Only(ANY_DEVICE_KEY), 1),
        // NX-OS lists Ed25519 as unsupported.
        NodeModel::CiscoNexus9300v => (AcceptedKeys::Only(ECDSA_OR_RSA_KEY), 1),
        NodeModel::CiscoIse => (AcceptedKeys::Only(RSA_KEY), 1),
        NodeModel::PaloaltoPanos => (AcceptedKeys::Only(ED25519_OR_RSA_KEY), 1),
        NodeModel::MikrotikChr => (AcceptedKeys::Only(ED25519_OR_RSA_KEY), usize::MAX),
        // A key and a `secondary` key.
        NodeModel::AristaVeos | NodeModel::AristaCeos => (AcceptedKeys::Only(ANY_DEVICE_KEY), 2),
        NodeModel::ArubaAoscx => (AcceptedKeys::Only(ANY_DEVICE_KEY), 32),
        NodeModel::NokiaSrlinux => (AcceptedKeys::Only(ANY_DEVICE_KEY), 32),
        NodeModel::CumulusLinux
        | NodeModel::JuniperVrouter
        | NodeModel::JuniperVswitch
        | NodeModel::JuniperVevolved
        | NodeModel::JuniperCevolved
        | NodeModel::JuniperVsrxv3 => (AcceptedKeys::Only(ANY_DEVICE_KEY), usize::MAX),
        // Linux, BSD and Windows guests run OpenSSH, which takes every registered key.
        _ => (AcceptedKeys::All, usize::MAX),
    };
    KeyRules { accepted, max_keys }
}

/// The key type of `algorithm`, or `None` for types no network device takes, such as DSA and
/// security keys.
fn key_type(algorithm: &Algorithm) -> Option<KeyType> {
    match algorithm {
        Algorithm::Ed25519 => Some(KeyType::Ed25519),
        Algorithm::Ecdsa { .. } => Some(KeyType::Ecdsa),
        Algorithm::Rsa { .. } => Some(KeyType::Rsa),
        _ => None,
    }
}

/// The `ssh-keygen` command that makes a key of `key_type`.
fn keygen_command(key_type: KeyType) -> &'static str {
    match key_type {
        KeyType::Ed25519 => "ssh-keygen -t ed25519",
        KeyType::Ecdsa => "ssh-keygen -t ecdsa -b 256",
        KeyType::Rsa => "ssh-keygen -t rsa -b 4096",
    }
}

/// The name of `key_type` in messages.
fn key_type_name(key_type: KeyType) -> &'static str {
    match key_type {
        KeyType::Ed25519 => "Ed25519",
        KeyType::Ecdsa => "ECDSA",
        KeyType::Rsa => "RSA",
    }
}

/// The owner's account as `image`'s model takes it: the login, and only the keys the model
/// accepts, up to the number it takes. Cisco ISE has a single administrator, `sherpa`, which
/// takes the owner's first RSA key.
///
/// # Errors
///
/// - If none of the owner's keys is a type the model accepts. The error names the accepted
///   types and the command that makes one.
/// - If a registered key cannot be parsed
///
#[instrument(level = "debug", skip(account, image), fields(username = %account.username))]
pub(crate) fn node_account(
    account: &GuestAccount,
    node_name: &str,
    image: &NodeConfig,
) -> Result<NodeAccount> {
    let KeyRules { accepted, max_keys } = key_rules(&image.model);
    let mut ssh_keys = Vec::new();
    let mut skipped_keys = Vec::new();
    for key in &account.ssh_keys {
        let parsed = PublicKey::from_openssh(key)
            .context("Invalid SSH public key registered for lab owner")?;
        let algorithm = parsed.algorithm();
        let taken = match (accepted, key_type(&algorithm)) {
            (AcceptedKeys::All, _) => true,
            (AcceptedKeys::Only(types), Some(key_type)) => types.contains(&key_type),
            (AcceptedKeys::Only(_), None) => false,
        };
        if taken && ssh_keys.len() < max_keys {
            ssh_keys.push(key.clone());
        } else {
            skipped_keys.push(algorithm.to_string());
        }
    }

    if let AcceptedKeys::Only(types) = accepted
        && ssh_keys.is_empty()
    {
        let names: Vec<&str> = types.iter().copied().map(key_type_name).collect();
        let Some(preferred) = types.first() else {
            bail!("Model '{}' accepts no SSH key types", image.model);
        };
        bail!(
            "Node '{node_name}' ({}) takes {} SSH keys only, and none of the keys registered \
             for '{}' is one. Make one with '{}', then add the public key on the web UI Profile \
             page or with 'sherpa server user ssh-key add'. See {SSH_KEYS_DOCS_URL}",
            image.model,
            names.join(" or "),
            account.username,
            keygen_command(*preferred)
        );
    }

    let username = if image.model == NodeModel::CiscoIse {
        SHERPA_USERNAME.to_owned()
    } else {
        account.username.clone()
    };

    Ok(NodeAccount {
        account: GuestAccount { username, ssh_keys },
        skipped_keys,
    })
}

/// The owner's login on `node`, or `None` when the lab has no owner account or the node gets
/// none (see [`provisions_owner`]).
///
/// # Errors
///
/// - As [`node_account`]
///
#[instrument(level = "debug", skip(account, node, image), fields(node_name = %node.name))]
pub(crate) fn node_owner(
    account: Option<&GuestAccount>,
    node: &topology::NodeExpanded,
    image: &NodeConfig,
) -> Result<Option<GuestAccount>> {
    match account {
        Some(account) if provisions_owner(node, image) => {
            Ok(Some(node_account(account, &node.name, image)?.account))
        }
        _ => Ok(None),
    }
}

/// The owner's account for a network device's config template, with each key written as
/// `format` and a random password nobody is told, or `None` when the node has no owner account.
///
/// # Errors
///
/// - If a key cannot be parsed or encoded
///
#[instrument(level = "debug", skip(account))]
pub(crate) fn owner_user(
    account: Option<&GuestAccount>,
    format: KeyFormat,
) -> Result<Option<OwnerUser>> {
    let Some(account) = account else {
        return Ok(None);
    };
    let mut ssh_keys = Vec::new();
    for key in &account.ssh_keys {
        let parsed = PublicKey::from_openssh(key)
            .context("Invalid SSH public key registered for lab owner")?;
        let algorithm = parsed.algorithm();
        let key_data = base64_encode_bytes(
            &parsed
                .to_bytes()
                .context("Failed to encode owner SSH public key")?,
        );
        let line = format!("{} {key_data}", algorithm.as_str());
        let owner_key = match format {
            KeyFormat::OpenSsh => OwnerSshKey {
                kind: algorithm.as_str().to_owned(),
                value: key_data,
            },
            KeyFormat::CiscoMd5 => OwnerSshKey {
                kind: algorithm.as_str().to_owned(),
                value: pub_ssh_key_to_md5_hash(&key_data)?,
            },
            KeyFormat::CiscoSha256 => OwnerSshKey {
                kind: algorithm.as_str().to_owned(),
                value: pub_ssh_key_to_sha256_hash(&key_data)?,
            },
            KeyFormat::Junos => OwnerSshKey {
                kind: junos_key_kind(&algorithm)?.to_owned(),
                value: line,
            },
            KeyFormat::PanosBase64 => OwnerSshKey {
                kind: algorithm.as_str().to_owned(),
                value: base64_encode(&line),
            },
        };
        ssh_keys.push(owner_key);
    }
    Ok(Some(OwnerUser {
        username: account.username.clone(),
        secret: random_secret(),
        ssh_keys,
    }))
}

/// A well-formed SHA-256 crypt hash of random characters, for devices that store a password hash,
/// such as PAN-OS. No known password matches it, so it works like a random password nobody is
/// told.
pub(crate) fn unusable_sha256_crypt_hash() -> String {
    let salt = random_crypt_chars(SHA256_CRYPT_SALT_LENGTH);
    let hash = random_crypt_chars(SHA256_CRYPT_HASH_LENGTH);
    format!("$5${salt}${hash}")
}

/// `length` random characters of the crypt(3) alphabet.
fn random_crypt_chars(length: usize) -> String {
    let mut rng = rand::thread_rng();
    let mut chars = String::with_capacity(length);
    for _ in 0..length {
        chars.push(char::from(
            CRYPT_ALPHABET[rng.gen_range(0..CRYPT_ALPHABET.len())],
        ));
    }
    chars
}

/// The Junos statement for a key of `algorithm`.
fn junos_key_kind(algorithm: &Algorithm) -> Result<&'static str> {
    match key_type(algorithm) {
        Some(KeyType::Ed25519) => Ok("ssh-ed25519"),
        Some(KeyType::Ecdsa) => Ok("ssh-ecdsa"),
        Some(KeyType::Rsa) => Ok("ssh-rsa"),
        None => bail!("Junos does not take {algorithm} SSH keys"),
    }
}

/// A random password with lowercase and uppercase letters and digits, which devices with
/// password strength rules require. Nobody is told it: the owner logs in by key.
fn random_secret() -> String {
    let mut rng = rand::thread_rng();
    let mut chars: Vec<char> = Alphanumeric
        .sample_string(&mut rng, OWNER_SECRET_LENGTH - 3)
        .chars()
        .collect();
    chars.push(rng.gen_range('a'..='z'));
    chars.push(rng.gen_range('A'..='Z'));
    chars.push(rng.gen_range('0'..='9'));
    chars.shuffle(&mut rng);
    chars.into_iter().collect()
}

/// Require the lab owner to have at least one registered SSH public key.
///
/// Labs are started only by users with a key, so their account on lab nodes can be provisioned.
#[instrument(level = "debug", skip(user), fields(username = %user.username))]
pub(crate) fn require_ssh_key(user: &DbUser) -> Result<()> {
    ensure!(
        !user.ssh_keys.is_empty(),
        "User '{}' has no SSH public key registered, and labs need one. Add a key on the web UI \
         Profile page or with 'sherpa server user ssh-key add'. See {SSH_KEYS_DOCS_URL}",
        user.username
    );
    Ok(())
}

/// Startup uses the authenticated creator; redeploy supplies the DB lab owner ID.
#[instrument(level = "debug", skip(db))]
pub(crate) async fn resolve_owner(
    db: &db::Database,
    owner_id: RecordId,
) -> Result<Option<GuestAccount>> {
    let owner = db::get_user_by_id(db, owner_id)
        .await
        .context("Failed to look up the lab owner")?
        .context("Lab owner account no longer exists")?;
    from_user(&owner)
}

/// The owner's guest account, or `None` when their username cannot be a login on lab nodes,
/// in which case the nodes keep only the `sherpa` account.
///
/// Usernames created since server usernames follow the guest rules always qualify; older names
/// may not. Missing or invalid keys are an error.
#[instrument(level = "debug", skip(user), fields(username = %user.username))]
pub(crate) fn from_user(user: &DbUser) -> Result<Option<GuestAccount>> {
    if let Err(error) = validate_guest_username(&user.username, RESERVED_USERNAMES) {
        tracing::warn!(
            username = %user.username,
            %error,
            "Lab owner's username cannot be a guest login; nodes keep the sherpa account"
        );
        return Ok(None);
    }
    require_ssh_key(user)?;
    let mut seen = HashSet::new();
    let mut ssh_keys = Vec::new();
    for key in &user.ssh_keys {
        ensure!(
            !key.trim().contains(['\n', '\r']),
            "SSH public keys must occupy a single line"
        );
        validate_ssh_key(key).context("Invalid SSH public key registered for lab owner")?;
        // Parsing also checks the wire format, unlike the existing format validator.
        let parsed = PublicKey::from_openssh(key.trim())
            .context("Invalid SSH public key registered for lab owner")?;
        let encoded = parsed
            .to_bytes()
            .context("Failed to encode owner SSH public key")?;
        if seen.insert(encoded) {
            let canonical = PublicKey::new(parsed.key_data().clone(), "");
            ssh_keys.push(
                canonical
                    .to_openssh()
                    .context("Failed to serialize owner SSH key")?,
            );
        }
    }
    Ok(Some(GuestAccount {
        username: user.username.clone(),
        ssh_keys,
    }))
}

#[instrument(level = "debug", skip(account))]
pub(crate) fn cloud_init_user(account: &GuestAccount) -> CloudInitUser {
    CloudInitUser {
        name: account.username.clone(),
        plain_text_passwd: None,
        hashed_passwd: None,
        lock_passwd: true,
        homedir: Some(format!("/home/{}", account.username)),
        ssh_authorized_keys: account.ssh_keys.clone(),
        sudo: Some(NODE_CLOUD_INIT_SUDO_RULE.to_owned()),
        groups: vec![],
        shell: "/bin/bash".to_owned(),
    }
}

/// The personal account on Alpine, which keeps its native tools: `ash` as the shell and
/// `doas`, through [`doas_file`], for admin rights. Nothing is installed at boot. Its password
/// is unusable rather than locked, because Alpine's sshd has no PAM and refuses locked accounts,
/// even for key logins.
#[instrument(level = "debug", skip(account))]
pub(crate) fn alpine_cloud_init_user(account: &GuestAccount) -> CloudInitUser {
    CloudInitUser {
        hashed_passwd: Some(NODE_NO_PASSWORD_HASH.to_owned()),
        lock_passwd: false,
        groups: vec!["wheel".to_owned()],
        shell: "/bin/sh".to_owned(),
        ..cloud_init_user(account)
    }
}

/// The personal account on NixOS: `wheel` for admin rights, since NixOS's generated sudoers does
/// not read cloud-init's sudo rules, and NixOS's bash, since it has no `/bin/bash`.
#[instrument(level = "debug", skip(account))]
pub(crate) fn nixos_cloud_init_user(account: &GuestAccount) -> CloudInitUser {
    CloudInitUser {
        sudo: None,
        groups: vec!["wheel".to_owned()],
        shell: NODE_NIXOS_SHELL.to_owned(),
        ..cloud_init_user(account)
    }
}

/// The personal account on FreeBSD and OpenBSD, which keeps their native tools: `sh` as the
/// shell and `wheel`, with `doas` on OpenBSD. The OS picks the home, which is under `/usr/home`
/// on FreeBSD. Its password is unusable rather than locked, because OpenSSH refuses locked
/// accounts on FreeBSD, even for key logins.
#[instrument(level = "debug", skip(account))]
pub(crate) fn bsd_cloud_init_user(account: &GuestAccount) -> CloudInitUser {
    CloudInitUser {
        homedir: None,
        hashed_passwd: Some(NODE_NO_PASSWORD_HASH.to_owned()),
        lock_passwd: false,
        groups: vec!["wheel".to_owned()],
        shell: "/bin/sh".to_owned(),
        ..cloud_init_user(account)
    }
}

/// The personal account on Windows: a local administrator. cloudbase-init needs a password, so it
/// gets a random one nobody is told. Its keys go in its own file, which the node's sshd_config
/// reads.
#[instrument(level = "debug", skip(account))]
pub(crate) fn cloudbase_user(account: &GuestAccount) -> CloudbaseInitUser {
    CloudbaseInitUser {
        name: account.username.clone(),
        passwd: random_secret(),
        groups: vec!["Administrators".to_owned()],
        ssh_authorized_keys: account.ssh_keys.clone(),
    }
}

/// A dedicated doas rule letting the personal account run commands as root without a password.
#[instrument(level = "debug", skip(account))]
pub(crate) fn doas_file(account: &GuestAccount) -> CloudInitWriteFile {
    CloudInitWriteFile {
        path: format!("{}/{}.conf", NODE_DOAS_CONF_DIR, account.username),
        content: format!("permit nopass {}\n", account.username),
        permissions: "0400".to_owned(),
        owner: None,
        encoding: None,
    }
}

/// The login that owns guest-side work: the personal account when provisioned, otherwise `sherpa`.
pub(crate) fn guest_username(account: Option<&GuestAccount>) -> &str {
    match account {
        Some(account) => &account.username,
        None => SHERPA_USERNAME,
    }
}

#[instrument(level = "debug", skip(account))]
pub(crate) fn ignition_user(account: &GuestAccount) -> IgnitionUser {
    IgnitionUser {
        name: account.username.clone(),
        password_hash: None,
        home_dir: Some(format!("/home/{}", account.username)),
        shell: Some("/bin/bash".to_owned()),
        ssh_authorized_keys: account.ssh_keys.clone(),
        groups: vec![],
    }
}

#[instrument(level = "debug", skip(account))]
pub(crate) fn sudoers_file(account: &GuestAccount) -> IgnitionFile {
    let content = format!("{} ALL=(ALL) NOPASSWD: ALL\n", account.username);
    IgnitionFile {
        path: format!("/etc/sudoers.d/{}", account.username),
        mode: 440,
        contents: Some(IgnitionFileContents::new(&format!(
            "data:;base64,{}",
            base64_encode(&content)
        ))),
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use jiff::Timestamp;
    use shared::konst::{ADMIN_USERNAME, SHERPA_PASSWORD, SHERPA_PASSWORD_HASH, SHERPA_USERNAME};
    use shared::util::base64_decode;
    use template::{CloudInitConfig, IgnitionConfig};

    const KEY: &str =
        "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIJbRB5JhR5JhR5JhR5JhR5JhR5JhR5JhR5JhR5JhR5Jh";
    const RSA_KEY: &str = "ssh-rsa AAAAB3NzaC1yc2EAAAADAQABAAAAgQDIz70/ubAJdlDE/jn+ySoukxqEDeP3lx0SFpcNNdeyQ91TgcLx7xfJ43qTlZLKWEWGoikFJdHQGztXjJ4urbC3nS5PUVLhjKvMfz9ynb8s3n/lEmfPJmh0C1SPPshOfBqvallYaehLUzTrv1kh6nPiOB23AEhgCrhGmtSvZm0LpQ==";
    const ECDSA_KEY: &str = "ecdsa-sha2-nistp256 AAAAE2VjZHNhLXNoYTItbmlzdHAyNTYAAAAIbmlzdHAyNTYAAABBBCbcNUEIVbyPNTPN2PLLO755EJzMwBaEYmLA3MUuKO5RJ9jQ5ruNCGmQJu2hA+cuUZ1/nrzF17LzOCPZLVpW4B4=";

    fn account(keys: &[&str]) -> GuestAccount {
        let mut ssh_keys = Vec::new();
        for key in keys {
            ssh_keys.push(key.to_string());
        }
        GuestAccount {
            username: "alice".to_owned(),
            ssh_keys,
        }
    }

    fn first_key(account: &GuestAccount, format: KeyFormat) -> OwnerSshKey {
        owner_user(Some(account), format).unwrap().unwrap().ssh_keys[0].clone()
    }

    fn owner(username: &str, keys: Vec<String>) -> DbUser {
        DbUser {
            id: Some(RecordId::new("user", username)),
            username: username.to_owned(),
            password_hash: "server-login-hash-never-copy".to_owned(),
            is_admin: false,
            ssh_keys: keys,
            created_at: Timestamp::UNIX_EPOCH,
            updated_at: Timestamp::UNIX_EPOCH,
        }
    }

    #[test]
    fn equivalent_keys_are_deduplicated_and_comments_removed() {
        let user = owner(
            "alice",
            vec![format!("{KEY} laptop"), format!("  {KEY} desktop  ")],
        );
        let account = from_user(&user).unwrap().unwrap();
        assert_eq!(account.username, "alice");
        assert_eq!(account.ssh_keys, vec![KEY]);
    }

    #[test]
    fn rejects_missing_invalid_and_malformed_keys() {
        for keys in [
            vec![],
            vec!["invalid".to_owned()],
            vec![format!("{KEY}\n{KEY}")],
            vec!["ssh-ed25519 AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA".to_owned()],
            vec![KEY.to_owned(), "broken".to_owned()],
        ] {
            assert!(from_user(&owner("alice", keys)).is_err());
        }
    }

    #[test]
    fn owners_whose_name_cannot_be_a_guest_login_get_no_account() {
        for name in [
            "root",
            SHERPA_USERNAME,
            ADMIN_USERNAME,
            "Alice",
            "alice@example.com",
        ] {
            assert!(
                from_user(&owner(name, vec![KEY.to_owned()]))
                    .unwrap()
                    .is_none()
            );
        }
    }

    #[test]
    fn nixos_account_uses_wheel_and_the_system_bash() {
        let account = GuestAccount {
            username: "alice".to_owned(),
            ssh_keys: vec![KEY.to_owned()],
        };
        let user = nixos_cloud_init_user(&account);
        assert_eq!(user.groups, vec!["wheel".to_owned()]);
        assert_eq!(user.shell, NODE_NIXOS_SHELL);
        assert_eq!(user.homedir.as_deref(), Some("/home/alice"));
        assert!(user.lock_passwd);
        assert_eq!(user.ssh_authorized_keys, vec![KEY.to_owned()]);
    }

    #[test]
    fn starting_labs_requires_a_registered_ssh_key() {
        let error = require_ssh_key(&owner("alice", vec![]))
            .unwrap_err()
            .to_string();
        assert!(error.contains("no SSH public key"), "{error}");
        assert!(error.contains(SSH_KEYS_DOCS_URL), "{error}");
        assert!(require_ssh_key(&owner("alice", vec![KEY.to_owned()])).is_ok());
    }

    #[test]
    fn every_model_that_can_add_an_account_at_boot_gets_one_with_its_default_image() {
        for model in [
            NodeModel::UbuntuLinux,
            NodeModel::OmarchyLinux,
            NodeModel::NixosLinux,
            NodeModel::AlpineLinux,
            NodeModel::FlatcarLinux,
            NodeModel::FreeBsd,
            NodeModel::OpenBsd,
            NodeModel::GenericVm,
            NodeModel::WindowsServer,
            NodeModel::DevboxWindows,
            NodeModel::AristaVeos,
            NodeModel::AristaCeos,
            NodeModel::ArubaAoscx,
            NodeModel::CiscoIosv,
            NodeModel::CiscoIosvl2,
            NodeModel::CiscoCsr1000v,
            NodeModel::CiscoCat8000v,
            NodeModel::CiscoCat9000v,
            NodeModel::CiscoAsav,
            NodeModel::CiscoNexus9300v,
            NodeModel::CiscoIosxrv9000,
            NodeModel::CiscoIse,
            NodeModel::JuniperVrouter,
            NodeModel::JuniperVswitch,
            NodeModel::JuniperVevolved,
            NodeModel::JuniperCevolved,
            NodeModel::JuniperVsrxv3,
            NodeModel::NokiaSrlinux,
            NodeModel::PaloaltoPanos,
            NodeModel::MikrotikChr,
            NodeModel::FrrLinux,
            NodeModel::CumulusLinux,
            NodeModel::SonicLinux,
        ] {
            let mut image = NodeConfig::get_model(model);
            assert!(supports_guest_account(&image), "{model}");
            image.ztp_enable = false;
            assert!(!supports_guest_account(&image), "{model}");
        }
        for model in [
            NodeModel::CiscoFtdv,
            NodeModel::TalosLinux,
            NodeModel::GitlabCe,
            NodeModel::GenericContainer,
            NodeModel::NanosUnikernel,
        ] {
            assert!(
                !supports_guest_account(&NodeConfig::get_model(model)),
                "{model}"
            );
        }
        // A model only gets one through the generator that writes it.
        let mut image = NodeConfig::get_model(NodeModel::UbuntuLinux);
        image.kind = NodeKind::Container;
        assert!(!supports_guest_account(&image));
        let mut image = NodeConfig::get_model(NodeModel::CiscoIosv);
        image.ztp_method = ZtpMethod::Tftp;
        assert!(!supports_guest_account(&image));
    }

    #[test]
    fn nodes_with_custom_ztp_get_no_owner_account() {
        let node = topology::Node {
            name: "r1".to_owned(),
            model: NodeModel::CiscoIosv,
            ztp_config: Some("opaque".to_owned()),
            ..Default::default()
        };
        let nodes = crate::services::up::process_manifest_nodes(&[node]);
        let image = NodeConfig::get_model(NodeModel::CiscoIosv);
        assert!(!provisions_owner(&nodes[0], &image));
        let mut node = nodes[0].clone();
        node.ztp_config = None;
        assert!(provisions_owner(&node, &image));
    }

    #[test]
    fn linux_nodes_take_every_registered_key() {
        let image = NodeConfig::get_model(NodeModel::UbuntuLinux);
        let node_account =
            node_account(&account(&[KEY, RSA_KEY, ECDSA_KEY]), "vm", &image).unwrap();
        assert_eq!(node_account.account.username, "alice");
        assert_eq!(node_account.account.ssh_keys, vec![KEY, RSA_KEY, ECDSA_KEY]);
        assert!(node_account.skipped_keys.is_empty());
    }

    #[test]
    fn devices_take_only_the_key_types_they_accept() {
        let image = NodeConfig::get_model(NodeModel::CiscoNexus9300v);
        let node_account = node_account(&account(&[KEY, ECDSA_KEY]), "nx1", &image).unwrap();
        assert_eq!(node_account.account.ssh_keys, vec![ECDSA_KEY]);
        assert_eq!(node_account.skipped_keys, vec!["ssh-ed25519"]);
    }

    #[test]
    fn devices_take_no_more_keys_than_they_hold() {
        let image = NodeConfig::get_model(NodeModel::PaloaltoPanos);
        let node_account = node_account(&account(&[RSA_KEY, KEY]), "fw1", &image).unwrap();
        assert_eq!(node_account.account.ssh_keys, vec![RSA_KEY]);
        assert_eq!(node_account.skipped_keys, vec!["ssh-ed25519"]);
    }

    #[test]
    fn a_device_without_an_accepted_key_fails_with_how_to_make_one() {
        let image = NodeConfig::get_model(NodeModel::CiscoIosv);
        let error = node_account(&account(&[KEY, ECDSA_KEY]), "r1", &image)
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("Node 'r1' (cisco_iosv) takes RSA SSH keys only"),
            "{error}"
        );
        assert!(error.contains("'ssh-keygen -t rsa -b 4096'"), "{error}");
        assert!(error.contains(SSH_KEYS_DOCS_URL), "{error}");
    }

    #[test]
    fn ise_admin_is_sherpa_with_the_owners_first_rsa_key() {
        let image = NodeConfig::get_model(NodeModel::CiscoIse);
        let node_account = node_account(&account(&[KEY, RSA_KEY]), "ise1", &image).unwrap();
        assert_eq!(node_account.account.username, SHERPA_USERNAME);
        assert_eq!(node_account.account.ssh_keys, vec![RSA_KEY]);
    }

    #[test]
    fn owner_keys_are_written_in_each_device_format() {
        let account = account(&[RSA_KEY]);
        let key_data = RSA_KEY.strip_prefix("ssh-rsa ").unwrap();
        let openssh = first_key(&account, KeyFormat::OpenSsh);
        assert_eq!(
            (openssh.kind.as_str(), openssh.value.as_str()),
            ("ssh-rsa", key_data)
        );
        assert_eq!(
            first_key(&account, KeyFormat::CiscoMd5).value,
            pub_ssh_key_to_md5_hash(key_data).unwrap()
        );
        assert_eq!(
            first_key(&account, KeyFormat::CiscoSha256).value,
            pub_ssh_key_to_sha256_hash(key_data).unwrap()
        );
        let junos = first_key(&account, KeyFormat::Junos);
        assert_eq!(
            (junos.kind.as_str(), junos.value.as_str()),
            ("ssh-rsa", RSA_KEY)
        );
        assert_eq!(
            first_key(&account, KeyFormat::PanosBase64).value,
            base64_encode(RSA_KEY)
        );
        let ecdsa = first_key(&self::account(&[ECDSA_KEY]), KeyFormat::Junos);
        assert_eq!(ecdsa.kind, "ssh-ecdsa");
        assert!(owner_user(None, KeyFormat::OpenSsh).unwrap().is_none());
    }

    #[test]
    fn owner_secrets_are_random_and_mix_letters_and_digits() {
        let first = owner_user(Some(&account(&[KEY])), KeyFormat::OpenSsh)
            .unwrap()
            .unwrap()
            .secret;
        let second = random_secret();
        assert_ne!(first, second);
        for secret in [first, second] {
            assert_eq!(secret.len(), OWNER_SECRET_LENGTH);
            assert!(secret.chars().any(|c| c.is_ascii_lowercase()), "{secret}");
            assert!(secret.chars().any(|c| c.is_ascii_uppercase()), "{secret}");
            assert!(secret.chars().any(|c| c.is_ascii_digit()), "{secret}");
            assert!(
                secret.chars().all(|c| c.is_ascii_alphanumeric()),
                "{secret}"
            );
        }
    }

    #[test]
    fn unusable_hash_is_a_well_formed_sha256_crypt_hash() {
        let hash = unusable_sha256_crypt_hash();
        let parts: Vec<&str> = hash.split('$').collect();
        assert_eq!(parts.len(), 4, "{hash}");
        assert_eq!(parts[1], "5");
        assert_eq!(parts[2].len(), SHA256_CRYPT_SALT_LENGTH);
        assert_eq!(parts[3].len(), SHA256_CRYPT_HASH_LENGTH);
        assert!(
            hash[3..]
                .bytes()
                .all(|b| b == b'$' || CRYPT_ALPHABET.contains(&b))
        );
        assert_ne!(hash, unusable_sha256_crypt_hash());
    }

    #[test]
    fn windows_owner_is_an_administrator_with_a_random_password() {
        let user = cloudbase_user(&account(&[KEY]));
        assert_eq!(user.name, "alice");
        assert_eq!(user.groups, vec!["Administrators"]);
        assert_eq!(user.ssh_authorized_keys, vec![KEY]);
        assert_eq!(user.passwd.len(), OWNER_SECRET_LENGTH);
        assert_ne!(user.passwd, SHERPA_PASSWORD);
    }

    #[test]
    fn guest_username_is_the_personal_account_when_provisioned() {
        let account = from_user(&owner("alice", vec![KEY.to_owned()]))
            .unwrap()
            .unwrap();
        assert_eq!(guest_username(Some(&account)), "alice");
        assert_eq!(guest_username(None), SHERPA_USERNAME);
    }

    #[test]
    fn personal_cloud_init_user_has_keys_home_shell_and_sudo_without_password() {
        let account = from_user(&owner("alice", vec![KEY.to_owned()]))
            .unwrap()
            .unwrap();
        let config = CloudInitConfig {
            users: vec![cloud_init_user(&account)],
            ..Default::default()
        }
        .to_string()
        .unwrap();
        let parsed: serde_json::Value =
            serde_json::from_str(config.strip_prefix("#cloud-config\n").unwrap()).unwrap();
        let users = parsed["users"].as_array().unwrap();
        assert_eq!(users.len(), 1, "only the personal account is added");
        let user = &users[0];
        assert_eq!(user["name"], "alice");
        assert_eq!(user["homedir"], "/home/alice");
        assert_eq!(user["shell"], "/bin/bash");
        assert_eq!(user["ssh_authorized_keys"][0], KEY);
        assert!(user["sudo"].as_str().unwrap().contains("NOPASSWD:ALL"));
        assert_eq!(user["lock_passwd"], true);
        assert!(!config.contains("plain_text_passwd"));
        assert!(!config.contains("server-login-hash"));
        assert!(!config.contains(SHERPA_PASSWORD_HASH));
    }

    #[test]
    fn personal_alpine_user_uses_ash_and_wheel() {
        let account = from_user(&owner("alice", vec![KEY.to_owned()]))
            .unwrap()
            .unwrap();
        let user = alpine_cloud_init_user(&account);
        assert_eq!(user.name, "alice");
        assert_eq!(user.shell, "/bin/sh");
        assert_eq!(user.groups, vec!["wheel"]);
        assert_eq!(user.ssh_authorized_keys, vec![KEY]);
        // Alpine's sshd has no PAM and refuses locked (`!`) accounts, even for key logins.
        assert!(!user.lock_passwd);
        assert_eq!(user.hashed_passwd.as_deref(), Some("*"));
        assert_eq!(user.plain_text_passwd, None);
    }

    #[test]
    fn personal_bsd_user_uses_sh_wheel_and_the_os_home() {
        let account = from_user(&owner("alice", vec![KEY.to_owned()]))
            .unwrap()
            .unwrap();
        let user = bsd_cloud_init_user(&account);
        assert_eq!(user.name, "alice");
        assert_eq!(user.shell, "/bin/sh");
        assert_eq!(user.groups, vec!["wheel"]);
        assert_eq!(user.ssh_authorized_keys, vec![KEY]);
        // FreeBSD homes are under /usr/home, so the OS picks the home.
        assert_eq!(user.homedir, None);
        // OpenSSH refuses locked accounts on FreeBSD, even for key logins.
        assert!(!user.lock_passwd);
        assert_eq!(user.hashed_passwd.as_deref(), Some("*"));
        assert_eq!(user.plain_text_passwd, None);
    }

    #[test]
    fn personal_alpine_user_gets_a_dedicated_doas_rule() {
        let account = from_user(&owner("alice", vec![KEY.to_owned()]))
            .unwrap()
            .unwrap();
        let file = doas_file(&account);
        assert_eq!(file.path, "/etc/doas.d/alice.conf");
        assert_eq!(file.content, "permit nopass alice\n");
        assert_eq!(file.permissions, "0400");
        assert_eq!(file.owner, None);
    }

    #[test]
    fn personal_ignition_user_has_native_fields_and_dedicated_sudoers() {
        let account = from_user(&owner("alice", vec![KEY.to_owned()]))
            .unwrap()
            .unwrap();
        let config = IgnitionConfig::new(
            vec![ignition_user(&account)],
            vec![sudoers_file(&account)],
            vec![],
            vec![],
            vec![],
            vec![],
        )
        .to_json_pretty()
        .unwrap();
        let json: serde_json::Value = serde_json::from_str(&config).unwrap();
        let user = &json["passwd"]["users"][0];
        assert_eq!(user["name"], "alice");
        assert_eq!(user["homeDir"], "/home/alice");
        assert_eq!(user["shell"], "/bin/bash");
        assert_eq!(user["sshAuthorizedKeys"], serde_json::json!([KEY]));
        assert!(user.get("passwordHash").is_none());
        assert!(!config.contains("server-login-hash"));
        let file = &json["storage"]["files"][0];
        assert_eq!(file["path"], "/etc/sudoers.d/alice");
        assert_eq!(file["mode"], 0o440);
        let encoded = file["contents"]["source"]
            .as_str()
            .unwrap()
            .strip_prefix("data:;base64,")
            .unwrap();
        assert_eq!(
            base64_decode(encoded).unwrap(),
            "alice ALL=(ALL) NOPASSWD: ALL\n"
        );
    }
}
