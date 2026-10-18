//! Resolve the lab owner's guest identity before provisioning changes resources.
use std::collections::HashSet;

use anyhow::{Context, Result, ensure};
use ssh_key::PublicKey;
use tracing::instrument;

use shared::data::{DbUser, NodeConfig, NodeKind, NodeModel, OsVariant, RecordId, ZtpMethod};
use shared::konst::SHERPA_USERNAME;
use shared::util::base64_encode;
use shared::validators::validate_guest_username;
use shared::validators::validate_ssh_key;
use template::{CloudInitUser, IgnitionFile, IgnitionFileContents, IgnitionUser};

#[derive(Debug, Clone)]
pub(crate) struct GuestAccount {
    pub username: String,
    pub ssh_keys: Vec<String>,
}

/// Keep this list aligned with the Linux branches in the VM ZTP generators.
#[instrument(level = "debug", skip(image))]
pub(crate) fn supports_guest_account(image: &NodeConfig) -> bool {
    image.kind == NodeKind::VirtualMachine
        && image.ztp_enable
        && image.os_variant == OsVariant::Linux
        && match image.ztp_method {
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
                    | NodeModel::KaliLinux
                    | NodeModel::InfrahubServer
                    | NodeModel::JenkinsServer
                    | NodeModel::NautobotServer
                    | NodeModel::NetboxServer
                    | NodeModel::SignozServer
                    | NodeModel::VirtServer
                    | NodeModel::AlpineLinux
            ),
            ZtpMethod::Ignition => image.model == NodeModel::FlatcarLinux,
            _ => false,
        }
}

#[instrument(level = "debug", skip(node, image))]
pub(crate) fn validate_node(node: &topology::NodeExpanded, image: &NodeConfig) -> Result<()> {
    ensure!(
        !supports_guest_account(image) || node.ztp_config.is_none(),
        "Node '{}' cannot use custom ZTP with provision_current_user",
        node.name
    );
    Ok(())
}

/// Startup uses the authenticated creator; redeploy supplies the DB lab owner ID.
#[instrument(level = "debug", skip(db))]
pub(crate) async fn resolve_owner(db: &db::Database, owner_id: RecordId) -> Result<GuestAccount> {
    let owner = db::get_user_by_id(db, owner_id)
        .await
        .context("Failed to look up the lab owner")?
        .context("Lab owner account no longer exists")?;
    from_user(&owner)
}

#[instrument(level = "debug", skip(user))]
pub(crate) fn from_user(user: &DbUser) -> Result<GuestAccount> {
    validate_guest_username(&user.username, &[SHERPA_USERNAME])
        .context("Lab owner cannot be provisioned as a Linux account")?;
    ensure!(
        !user.ssh_keys.is_empty(),
        "Lab owner must register at least one SSH public key"
    );
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
    Ok(GuestAccount {
        username: user.username.clone(),
        ssh_keys,
    })
}

#[instrument(level = "debug", skip(account))]
pub(crate) fn cloud_init_user(account: &GuestAccount) -> CloudInitUser {
    CloudInitUser {
        name: account.username.clone(),
        plain_text_passwd: None,
        lock_passwd: true,
        homedir: Some(format!("/home/{}", account.username)),
        ssh_authorized_keys: account.ssh_keys.clone(),
        sudo: "ALL=(ALL) NOPASSWD:ALL".to_owned(),
        groups: vec![],
        shell: "/bin/bash".to_owned(),
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
        contents: IgnitionFileContents::new(&format!("data:;base64,{}", base64_encode(&content))),
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use jiff::Timestamp;
    use shared::konst::SHERPA_PASSWORD_HASH;
    use shared::util::base64_decode;
    use template::{CloudInitConfig, IgnitionConfig};

    const KEY: &str =
        "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIJbRB5JhR5JhR5JhR5JhR5JhR5JhR5JhR5JhR5JhR5Jh";

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
        let account = from_user(&user).unwrap();
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
    fn validates_owner_username() {
        for name in ["root", SHERPA_USERNAME, "Alice", "alice@example.com"] {
            assert!(from_user(&owner(name, vec![KEY.to_owned()])).is_err());
        }
    }

    #[test]
    fn only_supported_linux_vm_generators_receive_accounts() {
        for model in [
            NodeModel::UbuntuLinux,
            NodeModel::OmarchyLinux,
            NodeModel::AlpineLinux,
            NodeModel::FlatcarLinux,
        ] {
            let mut image = NodeConfig::get_model(model);
            assert!(supports_guest_account(&image), "{model}");
            image.kind = NodeKind::Container;
            assert!(!supports_guest_account(&image));
        }
        for model in [
            NodeModel::FreeBsd,
            NodeModel::OpenBsd,
            NodeModel::WindowsServer,
            NodeModel::NokiaSrlinux,
            NodeModel::CiscoIosv,
            NodeModel::CumulusLinux,
            NodeModel::TalosLinux,
        ] {
            assert!(
                !supports_guest_account(&NodeConfig::get_model(model)),
                "{model}"
            );
        }
        let mut image = NodeConfig::get_model(NodeModel::UbuntuLinux);
        image.ztp_enable = false;
        assert!(!supports_guest_account(&image));
    }

    #[test]
    fn custom_ztp_rejected_before_provisioning() {
        let node = topology::Node {
            name: "vm".to_owned(),
            model: NodeModel::UbuntuLinux,
            ztp_config: Some("opaque".to_owned()),
            ..Default::default()
        };
        let nodes = crate::services::up::process_manifest_nodes(&[node]);
        assert!(validate_node(&nodes[0], &NodeConfig::get_model(NodeModel::UbuntuLinux)).is_err());
        assert!(validate_node(&nodes[0], &NodeConfig::get_model(NodeModel::CiscoIosv)).is_ok());
    }

    #[test]
    fn personal_cloud_init_user_has_keys_home_shell_and_sudo_without_password() {
        let account = from_user(&owner("alice", vec![KEY.to_owned()])).unwrap();
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
    fn personal_ignition_user_has_native_fields_and_dedicated_sudoers() {
        let account = from_user(&owner("alice", vec![KEY.to_owned()])).unwrap();
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
