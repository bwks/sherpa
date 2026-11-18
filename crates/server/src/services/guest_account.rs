//! Resolve the lab owner's login on lab nodes before provisioning changes resources.
//!
//! Every node with a generated login gets one account, the owner's: their name, the keys the
//! model takes, and their device password, as plain text or a salted crypt hash, whichever the
//! device takes. Cisco `FTDv` has only `admin`, which takes the device password. Aruba AOS-CX
//! takes the keys only: it has no hashed form, and its config is served over TFTP, so its
//! template locks the owner and `admin` instead.
use std::collections::HashSet;

use anyhow::{Context, Result, bail, ensure};
use ssh_key::{Algorithm, PublicKey};
use tracing::instrument;

use db::DevicePassword;
use shared::data::{DbUser, NodeConfig, NodeKind, NodeModel, OwnerSshKey, OwnerUser, Ztp};
use shared::konst::{
    DEVICE_PASSWORD_DOCS_URL, NODE_CLOUD_INIT_SUDO_RULE, RESERVED_USERNAMES, SSH_KEYS_DOCS_URL,
};
use shared::util::{
    base64_encode, base64_encode_bytes, pub_ssh_key_to_md5_hash, pub_ssh_key_to_sha256_hash,
};
use shared::validators::{effective_ztp, validate_guest_username, validate_ssh_key};
use template::{
    CloudInitUser, CloudInitWriteFile, CloudbaseInitUser, CryptHash, FileMode, IgnitionFile,
    IgnitionFileContents, IgnitionUser, crypt_hash,
};

use crate::konst::NODE_NIXOS_SHELL;
use crate::validate::validate_login_name;

/// Drop-in directory for per-user doas rules (Alpine).
const NODE_DOAS_CONF_DIR: &str = "/etc/doas.d";
/// The only login on Cisco `FTDv`, which takes the device password.
const FTDV_LOGIN: &str = "admin";
const ANY_DEVICE_KEY: &[KeyType] = &[KeyType::Ed25519, KeyType::Ecdsa, KeyType::Rsa];
const RSA_KEY: &[KeyType] = &[KeyType::Rsa];
const ECDSA_OR_RSA_KEY: &[KeyType] = &[KeyType::Ecdsa, KeyType::Rsa];
const ED25519_OR_RSA_KEY: &[KeyType] = &[KeyType::Ed25519, KeyType::Rsa];

/// The lab owner's login on lab nodes. Its `Debug` shows nothing of the device password.
#[derive(Debug, Clone)]
pub(crate) struct GuestAccount {
    pub username: String,
    pub ssh_keys: Vec<String>,
    pub device_password: DevicePassword,
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

/// How a device's config writes the device password.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PasswordForm {
    /// The password itself, where the device takes nothing else or a hashed form is not
    /// verified on it.
    Plain,
    /// A salted crypt hash of the password.
    Hashed(CryptHash),
}

/// Whether a node's ZTP adds the owner's account. Every method does, on every model that takes
/// it (`NodeModel::ztp_methods`), except Cisco `FTDv` and Talos, whose configs have no way to
/// add an account at boot. With ZTP off, nothing is generated to add it to.
#[instrument(level = "debug")]
pub(crate) fn supports_guest_account(model: NodeModel, ztp: Ztp) -> bool {
    match ztp {
        Ztp::Disabled => false,
        Ztp::CloudInit
        | Ztp::Cdrom
        | Ztp::Disk
        | Ztp::Http
        | Ztp::Ignition
        | Ztp::NoCloud
        | Ztp::Tftp
        | Ztp::Usb
        | Ztp::Volume => !matches!(model, NodeModel::CiscoFtd | NodeModel::TalosLinux),
    }
}

/// Whether the node gets the owner's account: its ZTP setting adds one and the node has no custom
/// ZTP config, which replaces the generated config the account is written into. FRR and the
/// `sonic_linux` container are the exceptions: their custom config is `frr.conf` or `config_db.json`,
/// and the account is in their start script, which is always generated.
#[instrument(level = "debug", skip(node, image))]
pub(crate) fn provisions_owner(node: &topology::NodeExpanded, image: &NodeConfig) -> bool {
    let account_outside_config = image.model == NodeModel::FrrLinux
        || (image.model == NodeModel::SonicLinux && image.kind() == NodeKind::Container);
    supports_guest_account(image.model, effective_ztp(node.ztp.method, image))
        && (node.ztp.config.is_none() || account_outside_config)
}

/// The owner keys a model takes, from each vendor's documentation of user public keys.
fn key_rules(model: NodeModel) -> KeyRules {
    let (accepted, max_keys) = match model {
        // IOS `key-hash` and IOS XE take RSA only, two keys per user.
        NodeModel::CiscoIos
        | NodeModel::CiscoIosl2
        | NodeModel::CiscoIol
        | NodeModel::CiscoIoll2
        | NodeModel::CiscoCsr1000
        | NodeModel::CiscoCat8000
        | NodeModel::CiscoCat9000 => (AcceptedKeys::Only(RSA_KEY), 2),
        // IOS XR takes four RSA keys: the first, then `second`, `third` and `fourth`.
        NodeModel::CiscoIosxr9000 => (AcceptedKeys::Only(RSA_KEY), 4),
        NodeModel::CiscoAsa => (AcceptedKeys::Only(ANY_DEVICE_KEY), 1),
        // NX-OS lists Ed25519 as unsupported.
        NodeModel::CiscoNexus9300 => (AcceptedKeys::Only(ECDSA_OR_RSA_KEY), 1),
        NodeModel::CiscoIse => (AcceptedKeys::Only(RSA_KEY), 1),
        NodeModel::PaloaltoPanos => (AcceptedKeys::Only(ED25519_OR_RSA_KEY), 1),
        NodeModel::MikrotikChr => (AcceptedKeys::Only(ED25519_OR_RSA_KEY), usize::MAX),
        // A key and a `secondary` key.
        NodeModel::AristaEos => (AcceptedKeys::Only(ANY_DEVICE_KEY), 2),
        NodeModel::ArubaAoscx | NodeModel::NokiaSrlinux => (AcceptedKeys::Only(ANY_DEVICE_KEY), 32),
        NodeModel::CumulusLinux
        | NodeModel::JuniperRouter
        | NodeModel::JuniperSwitch
        | NodeModel::JuniperEvolved
        | NodeModel::JuniperSrxv3 => (AcceptedKeys::Only(ANY_DEVICE_KEY), usize::MAX),
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
/// accepts, up to the number it takes. On Cisco ISE it is the single administrator, with the
/// owner's first RSA key.
///
/// # Errors
///
/// - If the model cannot take the owner's name as a login (`validate_login_name`)
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
    let model_kind = image.model_kind()?;
    validate_login_name(model_kind, node_name, &account.username)?;
    let KeyRules { accepted, max_keys } = key_rules(model_kind.model());
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
            "Node '{node_name}' ({model_kind}) takes {} SSH keys only, and none of the keys \
             registered for '{}' is one. Make one with '{}', then add the public key on the web UI \
             Profile page or with 'sherpa server user ssh-key add'. See {SSH_KEYS_DOCS_URL}",
            names.join(" or "),
            account.username,
            keygen_command(*preferred)
        );
    }

    Ok(NodeAccount {
        account: GuestAccount {
            username: account.username.clone(),
            ssh_keys,
            device_password: account.device_password.clone(),
        },
        skipped_keys,
    })
}

/// The owner's login on `node`, or `None` when the node gets none (see [`provisions_owner`]).
///
/// # Errors
///
/// - As [`node_account`]
///
#[instrument(level = "debug", skip(account, node, image), fields(node_name = %node.name))]
pub(crate) fn node_owner(
    account: &GuestAccount,
    node: &topology::NodeExpanded,
    image: &NodeConfig,
) -> Result<Option<GuestAccount>> {
    if provisions_owner(node, image) {
        Ok(Some(node_account(account, &node.name, image)?.account))
    } else {
        Ok(None)
    }
}

/// The login a `model` node's SSH config names when it is not the owner's name: `admin` on
/// Cisco `FTDv`, its only account, whatever its config. Every other node logs in as the owner.
pub(crate) fn fixed_login(model: NodeModel) -> Option<&'static str> {
    (model == NodeModel::CiscoFtd).then_some(FTDV_LOGIN)
}

/// The device password as a device's config writes it, in `form`.
///
/// # Errors
///
/// - If the hash cannot be computed
///
#[instrument(level = "debug", skip(password))]
pub(crate) fn device_password(password: &DevicePassword, form: PasswordForm) -> Result<String> {
    match form {
        PasswordForm::Plain => Ok(password.expose().to_owned()),
        PasswordForm::Hashed(algorithm) => crypt_hash(password.expose(), algorithm),
    }
}

/// The owner's account for a network device's config template, with each key written as
/// `format` and the device password as `password`. A node whose config is generated always has
/// an owner account.
///
/// # Errors
///
/// - If the node has no owner account
/// - If a key cannot be parsed or encoded, or the password hashed
///
#[instrument(level = "debug", skip(account))]
pub(crate) fn owner_user(
    account: Option<&GuestAccount>,
    format: KeyFormat,
    password: PasswordForm,
) -> Result<OwnerUser> {
    let account = account
        .context("The node's generated config needs the lab owner's account, and has none")?;
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
    Ok(OwnerUser {
        username: account.username.clone(),
        password: device_password(&account.device_password, password)?,
        ssh_keys,
    })
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

/// Require the lab owner to have set a device password, and open it. `up` and `redeploy` call it
/// before anything is created; the password is the owner's login password on the lab's nodes.
///
/// # Errors
///
/// - If the user has not set a device password. The error says how to set one.
/// - If the user has no id, or the password cannot be read or opened
///
#[instrument(level = "debug", skip(db, user), fields(username = %user.username))]
pub(crate) async fn require_device_password(
    db: &db::Database,
    user: &DbUser,
) -> Result<DevicePassword> {
    let id = user.id.as_ref().context("User record has no id")?;
    db::open_device_password(db, id)
        .await
        .context("Failed to read the device password")?
        .with_context(|| {
            format!(
                "User '{}' has no device password, and labs need one: it is the password of your \
                 login on lab nodes. Set it with 'sherpa server user device-password' or on the \
                 web UI Profile page. See {DEVICE_PASSWORD_DOCS_URL}",
                user.username
            )
        })
}

/// The owner's account on lab nodes: their name, their registered keys, deduplicated and
/// without comments, and `device_password`.
///
/// # Errors
///
/// - If the username cannot be a login on lab nodes, such as an account created before
///   reserved names were refused. The error names the reason.
/// - If the user has no SSH key, or a registered key is invalid
///
#[instrument(level = "debug", skip(user, device_password), fields(username = %user.username))]
pub(crate) fn from_user(user: &DbUser, device_password: DevicePassword) -> Result<GuestAccount> {
    validate_guest_username(&user.username, RESERVED_USERNAMES).with_context(|| {
        format!(
            "User '{}' cannot start labs: the name cannot be a login on lab nodes. Start the lab \
             from another account",
            user.username
        )
    })?;
    require_ssh_key(user)?;
    let mut seen = HashSet::new();
    let mut ssh_keys = Vec::new();
    for key in &user.ssh_keys {
        validate_ssh_key(key).context("Invalid SSH public key registered for lab owner")?;
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
    Ok(GuestAccount {
        username: user.username.clone(),
        ssh_keys,
        device_password,
    })
}

/// The owner's account on a cloud-init guest: their keys, passwordless `sudo`, and the device
/// password as a SHA-512-crypt hash.
///
/// # Errors
///
/// - If the password cannot be hashed
///
#[instrument(level = "debug", skip(account))]
pub(crate) fn cloud_init_user(account: &GuestAccount) -> Result<CloudInitUser> {
    Ok(CloudInitUser {
        name: account.username.clone(),
        plain_text_passwd: None,
        hashed_passwd: Some(device_password(
            &account.device_password,
            PasswordForm::Hashed(CryptHash::Sha512),
        )?),
        lock_passwd: false,
        homedir: Some(format!("/home/{}", account.username)),
        ssh_authorized_keys: account.ssh_keys.clone(),
        sudo: Some(NODE_CLOUD_INIT_SUDO_RULE.to_owned()),
        groups: vec![],
        shell: "/bin/bash".to_owned(),
    })
}

/// The owner's account on Alpine, which keeps its native tools: `ash` as the shell and
/// `doas`, through [`doas_file`], for admin rights. Nothing is installed at boot.
///
/// # Errors
///
/// - As [`cloud_init_user`]
///
#[instrument(level = "debug", skip(account))]
pub(crate) fn alpine_cloud_init_user(account: &GuestAccount) -> Result<CloudInitUser> {
    Ok(CloudInitUser {
        groups: vec!["wheel".to_owned()],
        shell: "/bin/sh".to_owned(),
        ..cloud_init_user(account)?
    })
}

/// The owner's account on NixOS: `wheel` for admin rights, since NixOS's generated sudoers does
/// not read cloud-init's sudo rules, and NixOS's bash, since it has no `/bin/bash`.
///
/// # Errors
///
/// - As [`cloud_init_user`]
///
#[instrument(level = "debug", skip(account))]
pub(crate) fn nixos_cloud_init_user(account: &GuestAccount) -> Result<CloudInitUser> {
    Ok(CloudInitUser {
        sudo: None,
        groups: vec!["wheel".to_owned()],
        shell: NODE_NIXOS_SHELL.to_owned(),
        ..cloud_init_user(account)?
    })
}

/// The owner's account on FreeBSD and OpenBSD, which keeps their native tools: `sh` as the
/// shell and `wheel`, with `doas` on OpenBSD. The OS picks the home, which is under `/usr/home`
/// on FreeBSD. OpenBSD's crypt(3) takes only bcrypt hashes, so there cloud-init gets the device
/// password itself and hashes it on the node; FreeBSD takes the SHA-512-crypt hash.
///
/// # Errors
///
/// - As [`cloud_init_user`]
///
#[instrument(level = "debug", skip(account))]
pub(crate) fn bsd_cloud_init_user(
    account: &GuestAccount,
    model: NodeModel,
) -> Result<CloudInitUser> {
    let user = CloudInitUser {
        homedir: None,
        groups: vec!["wheel".to_owned()],
        shell: "/bin/sh".to_owned(),
        ..cloud_init_user(account)?
    };
    if model == NodeModel::OpenBsd {
        return Ok(CloudInitUser {
            plain_text_passwd: Some(device_password(
                &account.device_password,
                PasswordForm::Plain,
            )?),
            hashed_passwd: None,
            ..user
        });
    }
    Ok(user)
}

/// The owner's account on Windows: a local administrator with the device password, which
/// cloudbase-init takes as plain text. Its keys go in its own file, which the node's
/// `sshd_config` reads.
#[instrument(level = "debug", skip(account))]
pub(crate) fn cloudbase_user(account: &GuestAccount) -> CloudbaseInitUser {
    CloudbaseInitUser {
        name: account.username.clone(),
        passwd: account.device_password.expose().to_owned(),
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

/// The owner's account on Flatcar: `wheel` and `docker`, as the `sherpa` account had, and the
/// device password as a SHA-512-crypt hash. Passwordless `sudo` is its [`sudoers_file`].
///
/// # Errors
///
/// - If the password cannot be hashed
///
#[instrument(level = "debug", skip(account))]
pub(crate) fn ignition_user(account: &GuestAccount) -> Result<IgnitionUser> {
    Ok(IgnitionUser {
        name: account.username.clone(),
        password_hash: Some(device_password(
            &account.device_password,
            PasswordForm::Hashed(CryptHash::Sha512),
        )?),
        home_dir: Some(format!("/home/{}", account.username)),
        shell: Some("/bin/bash".to_owned()),
        ssh_authorized_keys: account.ssh_keys.clone(),
        groups: vec!["wheel".to_owned(), "docker".to_owned()],
    })
}

#[instrument(level = "debug", skip(account))]
pub(crate) fn sudoers_file(account: &GuestAccount) -> IgnitionFile {
    let content = format!("{} ALL=(ALL) NOPASSWD: ALL\n", account.username);
    IgnitionFile {
        path: format!("/etc/sudoers.d/{}", account.username),
        mode: FileMode::new(0o440),
        contents: Some(IgnitionFileContents::new(&format!(
            "data:;base64,{}",
            base64_encode(&content)
        ))),
        ..Default::default()
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::sync::OnceLock;
    use std::thread;

    use db::SecretKey;
    use jiff::Timestamp;
    use sha_crypt::{PasswordVerifier, ShaCrypt};
    use shared::data::{ModelKind, RecordId};
    use shared::konst::{ADMIN_USERNAME, SHERPA_USERNAME};
    use shared::util::base64_decode;
    use template::{CloudInitConfig, IgnitionConfig};
    use tokio::runtime::Builder;

    /// The device password of every test account.
    const DEVICE_PASSWORD: &str = "Granite-Owl42!";

    /// `password` sealed into a keyed in-memory database for a new user and opened again: the
    /// one way to make a `DevicePassword`.
    async fn open_test_password(password: &str) -> DevicePassword {
        let db = db::open_in_memory()
            .await
            .unwrap()
            .with_secret_key(SecretKey::generate());
        db::apply_schema(&db).await.unwrap();
        let user = db::create_user(&db, "alice".to_owned(), "TestPass123!", false, vec![])
            .await
            .unwrap();
        let id = user.id.unwrap();
        db::set_device_password(&db, &id, password).await.unwrap();
        db::open_device_password(&db, &id).await.unwrap().unwrap()
    }

    /// `DEVICE_PASSWORD`, opened once on its own thread, so sync and async tests can share it.
    pub(crate) fn device_password() -> DevicePassword {
        static OPENED: OnceLock<DevicePassword> = OnceLock::new();
        OPENED
            .get_or_init(|| {
                thread::spawn(|| {
                    Builder::new_current_thread()
                        .build()
                        .unwrap()
                        .block_on(open_test_password(DEVICE_PASSWORD))
                })
                .join()
                .unwrap()
            })
            .clone()
    }

    /// The account of the owner `username`, with `keys` and `DEVICE_PASSWORD`.
    pub(crate) fn test_account(username: &str, keys: &[&str]) -> GuestAccount {
        GuestAccount {
            username: username.to_owned(),
            ssh_keys: keys.iter().map(ToString::to_string).collect(),
            device_password: device_password(),
        }
    }

    /// Whether `hash` is a crypt hash of `DEVICE_PASSWORD`.
    fn verifies(hash: &str) -> bool {
        ShaCrypt::default()
            .verify_password(DEVICE_PASSWORD.as_bytes(), hash)
            .is_ok()
    }

    const KEY: &str =
        "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIJbRB5JhR5JhR5JhR5JhR5JhR5JhR5JhR5JhR5JhR5Jh";
    const RSA_KEY: &str = "ssh-rsa AAAAB3NzaC1yc2EAAAADAQABAAAAgQDIz70/ubAJdlDE/jn+ySoukxqEDeP3lx0SFpcNNdeyQ91TgcLx7xfJ43qTlZLKWEWGoikFJdHQGztXjJ4urbC3nS5PUVLhjKvMfz9ynb8s3n/lEmfPJmh0C1SPPshOfBqvallYaehLUzTrv1kh6nPiOB23AEhgCrhGmtSvZm0LpQ==";
    const ECDSA_KEY: &str = "ecdsa-sha2-nistp256 AAAAE2VjZHNhLXNoYTItbmlzdHAyNTYAAAAIbmlzdHAyNTYAAABBBCbcNUEIVbyPNTPN2PLLO755EJzMwBaEYmLA3MUuKO5RJ9jQ5ruNCGmQJu2hA+cuUZ1/nrzF17LzOCPZLVpW4B4=";

    fn account(keys: &[&str]) -> GuestAccount {
        test_account("alice", keys)
    }

    fn first_key(account: &GuestAccount, format: KeyFormat) -> OwnerSshKey {
        owner_user(Some(account), format, PasswordForm::Plain)
            .unwrap()
            .ssh_keys[0]
            .clone()
    }

    /// `user`'s account, with `DEVICE_PASSWORD`.
    fn account_of(user: &DbUser) -> Result<GuestAccount> {
        from_user(user, device_password())
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
        let account = account_of(&user).unwrap();
        assert_eq!(account.username, "alice");
        assert_eq!(account.ssh_keys, vec![KEY]);
        assert_eq!(account.device_password.expose(), DEVICE_PASSWORD);
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
            assert!(account_of(&owner("alice", keys)).is_err());
        }
    }

    #[test]
    fn owners_whose_name_cannot_be_a_login_are_refused_with_the_reason() {
        // Legacy accounts created before the reserved names were refused, and older names.
        for name in [
            "root",
            SHERPA_USERNAME,
            ADMIN_USERNAME,
            "Alice",
            "alice@example.com",
        ] {
            let error = format!(
                "{:#}",
                account_of(&owner(name, vec![KEY.to_owned()])).unwrap_err()
            );
            assert!(
                error.contains(&format!("User '{name}' cannot start labs")),
                "{error}"
            );
            assert!(error.contains("cannot be a login on lab nodes"), "{error}");
        }
    }

    #[test]
    fn nixos_account_uses_wheel_and_the_system_bash() {
        let user = nixos_cloud_init_user(&account(&[KEY])).unwrap();
        assert_eq!(user.groups, vec!["wheel".to_owned()]);
        assert_eq!(user.shell, NODE_NIXOS_SHELL);
        assert_eq!(user.homedir.as_deref(), Some("/home/alice"));
        assert!(!user.lock_passwd);
        assert!(verifies(user.hashed_passwd.as_deref().unwrap()));
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

    #[tokio::test]
    async fn starting_labs_requires_a_device_password() {
        let db = db::open_in_memory()
            .await
            .unwrap()
            .with_secret_key(SecretKey::generate());
        db::apply_schema(&db).await.unwrap();
        let user = db::create_user(&db, "alice".to_owned(), "TestPass123!", false, vec![])
            .await
            .unwrap();

        let error = require_device_password(&db, &user)
            .await
            .unwrap_err()
            .to_string();
        assert!(
            error.starts_with(
                "User 'alice' has no device password, and labs need one: it is the password of \
                 your login on lab nodes. Set it with 'sherpa server user device-password' or on \
                 the web UI Profile page."
            ),
            "{error}"
        );
        assert!(error.contains(DEVICE_PASSWORD_DOCS_URL), "{error}");

        db::set_device_password(&db, user.id.as_ref().unwrap(), DEVICE_PASSWORD)
            .await
            .unwrap();
        let opened = require_device_password(&db, &user).await.unwrap();
        assert_eq!(opened.expose(), DEVICE_PASSWORD);
    }

    #[test]
    fn an_account_never_shows_its_device_password() {
        let shown = format!("{:?}", account(&[KEY]));
        assert!(!shown.contains(DEVICE_PASSWORD), "{shown}");
        assert!(shown.contains("REDACTED"), "{shown}");
    }

    #[test]
    fn a_model_that_cannot_take_the_owners_name_is_refused() {
        let long = test_account(&"a".repeat(21), &[KEY]);
        let image = ModelKind::default_for(NodeModel::WindowsServer).node_config();
        let error = node_account(&long, "win1", &image).unwrap_err().to_string();
        assert!(
            error.contains("Node 'win1' (windows_server) takes login names of at most 20"),
            "{error}"
        );
        let image = ModelKind::default_for(NodeModel::UbuntuLinux).node_config();
        assert!(node_account(&long, "vm1", &image).is_ok());
    }

    #[test]
    fn ftdv_logs_in_as_admin_and_every_other_model_as_the_owner() {
        assert_eq!(fixed_login(NodeModel::CiscoFtd), Some("admin"));
        for model in NodeModel::to_vec() {
            if model != NodeModel::CiscoFtd {
                assert_eq!(fixed_login(model), None, "{model}");
            }
        }
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
            NodeModel::Generic,
            NodeModel::WindowsServer,
            NodeModel::DevboxWindows,
            NodeModel::AristaEos,
            NodeModel::ArubaAoscx,
            NodeModel::CiscoIos,
            NodeModel::CiscoIosl2,
            NodeModel::CiscoIol,
            NodeModel::CiscoIoll2,
            NodeModel::CiscoCsr1000,
            NodeModel::CiscoCat8000,
            NodeModel::CiscoCat9000,
            NodeModel::CiscoAsa,
            NodeModel::CiscoNexus9300,
            NodeModel::CiscoIosxr9000,
            NodeModel::CiscoIse,
            NodeModel::JuniperRouter,
            NodeModel::JuniperSwitch,
            NodeModel::JuniperEvolved,
            NodeModel::JuniperSrxv3,
            NodeModel::NokiaSrlinux,
            NodeModel::PaloaltoPanos,
            NodeModel::MikrotikChr,
            NodeModel::FrrLinux,
            NodeModel::CumulusLinux,
            NodeModel::SonicLinux,
        ] {
            let image = ModelKind::default_for(model).node_config();
            assert!(supports_guest_account(model, image.ztp()), "{model}");
            assert!(!supports_guest_account(model, Ztp::Disabled), "{model}");
        }
        for model in [
            NodeModel::CiscoFtd,
            NodeModel::TalosLinux,
            NodeModel::GitlabCe,
            NodeModel::NanosUnikernel,
        ] {
            assert!(
                !supports_guest_account(model, ModelKind::default_for(model).node_config().ztp()),
                "{model}"
            );
        }
    }

    /// The models each generator wrote the owner's account for, as the lists kept here by hand
    /// had them before the per-model table replaced them.
    fn listed_by_the_generators(model: NodeModel, kind: NodeKind, ztp: Ztp) -> bool {
        match kind {
            NodeKind::VirtualMachine => match ztp {
                Ztp::CloudInit => matches!(
                    model,
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
                        | NodeModel::Generic
                        | NodeModel::DevboxWindows
                        | NodeModel::WindowsServer
                ),
                Ztp::Ignition => model == NodeModel::FlatcarLinux,
                Ztp::Tftp => matches!(
                    model,
                    NodeModel::AristaEos | NodeModel::ArubaAoscx | NodeModel::JuniperEvolved
                ),
                Ztp::Cdrom => matches!(
                    model,
                    NodeModel::CiscoCsr1000
                        | NodeModel::CiscoCat8000
                        | NodeModel::CiscoCat9000
                        | NodeModel::CiscoAsa
                        | NodeModel::CiscoNexus9300
                        | NodeModel::CiscoIosxr9000
                        | NodeModel::JuniperSrxv3
                        | NodeModel::JuniperRouter
                        | NodeModel::JuniperSwitch
                        | NodeModel::PaloaltoPanos
                ),
                Ztp::Disk => matches!(
                    model,
                    NodeModel::CiscoIos
                        | NodeModel::CiscoIosl2
                        | NodeModel::CiscoIse
                        | NodeModel::MikrotikChr
                ),
                Ztp::Usb => matches!(model, NodeModel::CumulusLinux | NodeModel::JuniperEvolved),
                Ztp::Http => model == NodeModel::SonicLinux,
                Ztp::NoCloud | Ztp::Volume | Ztp::Disabled => false,
            },
            NodeKind::Container => {
                ztp == Ztp::Volume
                    && matches!(
                        model,
                        NodeModel::AristaEos
                            | NodeModel::NokiaSrlinux
                            | NodeModel::FrrLinux
                            | NodeModel::SonicLinux
                            | NodeModel::JuniperEvolved
                            | NodeModel::CiscoIol
                            | NodeModel::CiscoIoll2
                    )
            }
            NodeKind::Unikernel => false,
        }
    }

    #[test]
    fn the_owner_account_follows_the_generators_for_every_model_and_setting() {
        for model_kind in ModelKind::all() {
            let (model, kind) = (model_kind.model(), model_kind.kind());
            let settings = model_kind
                .ztp_methods()
                .iter()
                .copied()
                .chain([Ztp::Disabled]);
            for ztp in settings {
                assert_eq!(
                    supports_guest_account(model, ztp),
                    listed_by_the_generators(model, kind, ztp),
                    "{model} with {ztp}"
                );
            }
        }
    }

    #[test]
    fn a_manifest_ztp_decides_the_owner_account_over_the_images() {
        let image = ModelKind::default_for(NodeModel::FrrLinux).node_config();
        let mut node = topology::NodeExpanded {
            name: "frr01".to_owned(),
            model_kind: ModelKind::default_for(NodeModel::FrrLinux),
            ..Default::default()
        };
        assert!(provisions_owner(&node, &image));
        node.ztp.method = Some(Ztp::Disabled);
        assert!(!provisions_owner(&node, &image));
    }

    #[test]
    fn frr_keeps_the_owner_account_with_a_custom_frr_conf() {
        // FRR's custom config is `frr.conf`; the account is in its generated startup script.
        let image = ModelKind::default_for(NodeModel::FrrLinux).node_config();
        let mut node = topology::NodeExpanded {
            name: "frr01".to_owned(),
            model_kind: ModelKind::default_for(NodeModel::FrrLinux),
            ..Default::default()
        };
        node.ztp.config = Some("router bgp 65000\n".to_owned());
        assert!(provisions_owner(&node, &image));
    }

    #[test]
    fn the_sonic_container_keeps_the_owner_account_with_a_custom_config_db_and_the_vm_does_not() {
        // The container's custom config is `config_db.json`; the account is in its start
        // script. The VM's account is in a ZTP step a custom config replaces.
        let container = ModelKind::new(NodeModel::SonicLinux, Some(NodeKind::Container)).unwrap();
        let vm = ModelKind::default_for(NodeModel::SonicLinux);
        for (model_kind, keeps) in [(container, true), (vm, false)] {
            let mut node = topology::NodeExpanded {
                name: "sonic01".to_owned(),
                model_kind,
                ..Default::default()
            };
            node.ztp.config = Some("{}".to_owned());
            assert_eq!(
                provisions_owner(&node, &model_kind.node_config()),
                keeps,
                "{model_kind}"
            );
        }
    }

    #[test]
    fn nodes_with_custom_ztp_get_no_owner_account() {
        let node = topology::Node {
            name: "r1".to_owned(),
            model: NodeModel::CiscoIos,
            ztp: topology::NodeZtp {
                method: None,
                config: Some("opaque".to_owned()),
            },
            ..Default::default()
        };
        let nodes = crate::services::up::process_manifest_nodes(&[node]).unwrap();
        let image = ModelKind::default_for(NodeModel::CiscoIos).node_config();
        assert!(!provisions_owner(&nodes[0], &image));
        let mut node = nodes[0].clone();
        node.ztp.config = None;
        assert!(provisions_owner(&node, &image));
    }

    #[test]
    fn linux_nodes_take_every_registered_key() {
        let image = ModelKind::default_for(NodeModel::UbuntuLinux).node_config();
        let node_account =
            node_account(&account(&[KEY, RSA_KEY, ECDSA_KEY]), "vm", &image).unwrap();
        assert_eq!(node_account.account.username, "alice");
        assert_eq!(node_account.account.ssh_keys, vec![KEY, RSA_KEY, ECDSA_KEY]);
        assert_eq!(node_account.skipped_keys, [] as [std::string::String; 0]);
    }

    #[test]
    fn devices_take_only_the_key_types_they_accept() {
        let image = ModelKind::default_for(NodeModel::CiscoNexus9300).node_config();
        let node_account = node_account(&account(&[KEY, ECDSA_KEY]), "nx1", &image).unwrap();
        assert_eq!(node_account.account.ssh_keys, vec![ECDSA_KEY]);
        assert_eq!(node_account.skipped_keys, vec!["ssh-ed25519"]);
    }

    #[test]
    fn devices_take_no_more_keys_than_they_hold() {
        let image = ModelKind::default_for(NodeModel::PaloaltoPanos).node_config();
        let node_account = node_account(&account(&[RSA_KEY, KEY]), "fw1", &image).unwrap();
        assert_eq!(node_account.account.ssh_keys, vec![RSA_KEY]);
        assert_eq!(node_account.skipped_keys, vec!["ssh-ed25519"]);
    }

    #[test]
    fn a_device_without_an_accepted_key_fails_with_how_to_make_one() {
        let image = ModelKind::default_for(NodeModel::CiscoIos).node_config();
        let error = node_account(&account(&[KEY, ECDSA_KEY]), "r1", &image)
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("Node 'r1' (cisco_ios) takes RSA SSH keys only"),
            "{error}"
        );
        assert!(error.contains("'ssh-keygen -t rsa -b 4096'"), "{error}");
        assert!(error.contains(SSH_KEYS_DOCS_URL), "{error}");
    }

    #[test]
    fn iol_takes_rsa_keys_only_as_ios_key_hashes_do() {
        for model in [NodeModel::CiscoIol, NodeModel::CiscoIoll2] {
            let image = ModelKind::default_for(model).node_config();
            let node_account =
                node_account(&account(&[KEY, RSA_KEY, ECDSA_KEY]), "r1", &image).unwrap();
            assert_eq!(node_account.account.ssh_keys, vec![RSA_KEY], "{model}");
            assert_eq!(
                node_account.skipped_keys,
                vec!["ssh-ed25519", "ecdsa-sha2-nistp256"],
                "{model}"
            );
        }
    }

    #[test]
    fn ise_admin_is_the_owner_with_their_first_rsa_key() {
        let image = ModelKind::default_for(NodeModel::CiscoIse).node_config();
        let node_account = node_account(&account(&[KEY, RSA_KEY]), "ise1", &image).unwrap();
        assert_eq!(node_account.account.username, "alice");
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
        assert!(owner_user(None, KeyFormat::OpenSsh, PasswordForm::Plain).is_err());
    }

    #[test]
    fn the_owners_password_is_plain_or_a_hash_that_verifies() {
        let account = account(&[KEY]);
        let plain = owner_user(Some(&account), KeyFormat::OpenSsh, PasswordForm::Plain).unwrap();
        assert_eq!(plain.password, DEVICE_PASSWORD);
        for (algorithm, prefix) in [(CryptHash::Sha512, "$6$"), (CryptHash::Sha256, "$5$")] {
            let hashed = owner_user(
                Some(&account),
                KeyFormat::OpenSsh,
                PasswordForm::Hashed(algorithm),
            )
            .unwrap()
            .password;
            assert!(hashed.starts_with(prefix), "{hashed}");
            assert!(!hashed.contains(DEVICE_PASSWORD));
            assert!(verifies(&hashed), "{hashed}");
        }
    }

    #[test]
    fn windows_owner_is_an_administrator_with_the_device_password() {
        let user = cloudbase_user(&account(&[KEY]));
        assert_eq!(user.name, "alice");
        assert_eq!(user.groups, vec!["Administrators"]);
        assert_eq!(user.ssh_authorized_keys, vec![KEY]);
        assert_eq!(user.passwd, DEVICE_PASSWORD);
    }

    #[test]
    fn personal_cloud_init_user_has_keys_home_shell_and_sudo_without_password() {
        let account = account_of(&owner("alice", vec![KEY.to_owned()])).unwrap();
        let config = CloudInitConfig {
            users: vec![cloud_init_user(&account).unwrap()],
            ..Default::default()
        }
        .render()
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
        assert_eq!(user["lock_passwd"], false);
        assert!(verifies(user["hashed_passwd"].as_str().unwrap()));
        assert!(!config.contains("plain_text_passwd"));
        assert!(!config.contains(DEVICE_PASSWORD));
        assert!(!config.contains("server-login-hash"));
    }

    #[test]
    fn personal_alpine_user_uses_ash_and_wheel() {
        let account = account_of(&owner("alice", vec![KEY.to_owned()])).unwrap();
        let user = alpine_cloud_init_user(&account).unwrap();
        assert_eq!(user.name, "alice");
        assert_eq!(user.shell, "/bin/sh");
        assert_eq!(user.groups, vec!["wheel"]);
        assert_eq!(user.ssh_authorized_keys, vec![KEY]);
        // Alpine's sshd has no PAM and refuses locked (`!`) accounts, even for key logins.
        assert!(!user.lock_passwd);
        assert!(verifies(user.hashed_passwd.as_deref().unwrap()));
        assert_eq!(user.plain_text_passwd, None);
    }

    #[test]
    fn personal_bsd_user_uses_sh_wheel_and_the_os_home() {
        let account = account_of(&owner("alice", vec![KEY.to_owned()])).unwrap();
        let user = bsd_cloud_init_user(&account, NodeModel::FreeBsd).unwrap();
        assert_eq!(user.name, "alice");
        assert_eq!(user.shell, "/bin/sh");
        assert_eq!(user.groups, vec!["wheel"]);
        assert_eq!(user.ssh_authorized_keys, vec![KEY]);
        // FreeBSD homes are under /usr/home, so the OS picks the home.
        assert_eq!(user.homedir, None);
        // OpenSSH refuses locked accounts on FreeBSD, even for key logins.
        assert!(!user.lock_passwd);
        assert!(verifies(user.hashed_passwd.as_deref().unwrap()));
        assert_eq!(user.plain_text_passwd, None);
    }

    #[test]
    fn openbsd_gets_the_device_password_to_hash_with_its_own_crypt() {
        let user = bsd_cloud_init_user(&account(&[KEY]), NodeModel::OpenBsd).unwrap();
        assert_eq!(user.plain_text_passwd.as_deref(), Some(DEVICE_PASSWORD));
        assert_eq!(user.hashed_passwd, None);
        assert!(!user.lock_passwd);
    }

    #[test]
    fn personal_alpine_user_gets_a_dedicated_doas_rule() {
        let account = account_of(&owner("alice", vec![KEY.to_owned()])).unwrap();
        let file = doas_file(&account);
        assert_eq!(file.path, "/etc/doas.d/alice.conf");
        assert_eq!(file.content, "permit nopass alice\n");
        assert_eq!(file.permissions, "0400");
        assert_eq!(file.owner, None);
    }

    #[test]
    fn personal_ignition_user_has_native_fields_and_dedicated_sudoers() {
        let account = account_of(&owner("alice", vec![KEY.to_owned()])).unwrap();
        let config = IgnitionConfig::new(
            vec![ignition_user(&account).unwrap()],
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
        assert_eq!(user["groups"], serde_json::json!(["wheel", "docker"]));
        assert!(verifies(user["passwordHash"].as_str().unwrap()));
        assert!(!config.contains(DEVICE_PASSWORD));
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
