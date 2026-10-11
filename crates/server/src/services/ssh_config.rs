use std::net::Ipv4Addr;

use tracing::instrument;

use shared::data::{NodeSshUsername, ZtpRecord};
use shared::konst::{SHERPA_DOMAIN_NAME, SHERPA_SSH_PRIVATE_KEY_FILE, SHERPA_USERNAME};
use template::{SshConfigTemplate, SshHostConfig};

/// Resolve SSH identities and aliases before passing data to the renderer.
#[instrument(level = "debug", skip(records, ssh_usernames))]
pub(crate) fn build_ssh_config(
    records: &[ZtpRecord],
    ssh_usernames: &[NodeSshUsername],
    lab_id: &str,
    proxy_user: &str,
    server_ipv4: Ipv4Addr,
) -> SshConfigTemplate {
    let hosts = records
        .iter()
        .map(|record| {
            let personal_username = ssh_usernames
                .iter()
                .find(|entry| entry.node_name == record.node_name)
                .map(|entry| entry.username.as_str());
            SshHostConfig {
                aliases: format!(
                    "{} {}.{lab_id} {}.{lab_id}.{SHERPA_DOMAIN_NAME}",
                    record.ipv4_address, record.node_name, record.node_name
                ),
                hostname: record.ipv4_address,
                port: record.ssh_port,
                username: personal_username.unwrap_or(SHERPA_USERNAME).to_owned(),
                identity_file: personal_username
                    .is_none()
                    .then(|| SHERPA_SSH_PRIVATE_KEY_FILE.to_owned()),
            }
        })
        .collect();
    SshConfigTemplate {
        hosts,
        proxy_jump: format!("{proxy_user}@{server_ipv4}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use askama::Template;
    use shared::data::ZtpMethod;

    fn records() -> Vec<ZtpRecord> {
        ["vm", "router"]
            .into_iter()
            .enumerate()
            .map(|(index, name)| ZtpRecord {
                node_name: name.to_owned(),
                ipv4_address: format!("192.0.2.{}", index + 10).parse().unwrap(),
                ipv6_address: None,
                config_file: String::new(),
                mac_address: String::new(),
                ztp_method: ZtpMethod::None,
                ssh_port: if index == 0 { 22 } else { 830 },
            })
            .collect()
    }

    #[test]
    fn mixed_lab_selects_personal_identity_and_preserves_legacy_options() {
        let usernames = [NodeSshUsername {
            node_name: "vm".to_owned(),
            username: "alice".to_owned(),
        }];
        let config = build_ssh_config(
            &records(),
            &usernames,
            "abcd1234",
            "owner",
            "192.0.2.1".parse().unwrap(),
        );
        assert_eq!(config.hosts[0].username, "alice");
        assert!(config.hosts[0].identity_file.is_none());
        assert_eq!(config.hosts[1].username, SHERPA_USERNAME);
        assert_eq!(
            config.hosts[1].identity_file.as_deref(),
            Some(SHERPA_SSH_PRIVATE_KEY_FILE)
        );
        let output = config.render().unwrap();
        let (personal, legacy) = output.split_once("Host 192.0.2.11").unwrap();
        assert!(personal.contains("User alice"));
        assert!(personal.contains("vm.abcd1234.sherpa.lab.local"));
        assert!(personal.contains("ProxyJump owner@192.0.2.1"));
        assert!(!personal.contains("IdentityFile"));
        assert!(!personal.contains("IdentitiesOnly"));
        assert!(legacy.contains("User sherpa"));
        assert!(legacy.contains("Port 830"));
        assert!(legacy.contains("IdentityFile sherpa_ssh_key"));
        assert!(legacy.contains("IdentitiesOnly yes"));
    }

    #[test]
    fn unrelated_metadata_does_not_change_legacy_nodes() {
        let usernames = [NodeSshUsername {
            node_name: "other-vm".to_owned(),
            username: "alice".to_owned(),
        }];
        for usernames in [&[][..], &usernames[..]] {
            let config = build_ssh_config(
                &records(),
                usernames,
                "abcd1234",
                "owner",
                "192.0.2.1".parse().unwrap(),
            );
            assert!(
                config
                    .hosts
                    .iter()
                    .all(|host| host.username == SHERPA_USERNAME
                        && host.identity_file.as_deref() == Some(SHERPA_SSH_PRIVATE_KEY_FILE))
            );
            assert_eq!(
                config.hosts[0].aliases,
                "192.0.2.10 vm.abcd1234 vm.abcd1234.sherpa.lab.local"
            );
            assert_eq!(config.proxy_jump, "owner@192.0.2.1");
        }
    }
}
