//! A lab's details as the client's `lab-info.toml` holds them, built from the lab's records.
//!
//! The server keeps no `lab-info.toml` of its own: the lab record holds the lab's networks,
//! name and what the user said about it, the owner's user record the owner's name, and each
//! node record the account the lab's SSH config logs in to it with. `up` returns the details,
//! and `download` and the web UI's lab files build the file from them.

use anyhow::{Context, Result};
use tracing::instrument;

use db::Database;
use shared::data::{DbLab, DbNode, LabInfo, NodeSshUsername};

/// The owner named for a lab whose owner's record is gone.
const UNKNOWN_OWNER: &str = "unknown";

/// The details of `lab`, from its record, its owner's and its nodes'.
///
/// # Errors
///
/// Fails when the owner or the nodes cannot be read, or the lab's networks do not parse.
#[instrument(skip(db, lab), fields(lab_id = %lab.lab_id), level = "debug")]
pub(crate) async fn load_lab_info(db: &Database, lab: &DbLab) -> Result<LabInfo> {
    let owner = lab_owner(db, lab).await?;
    let nodes = db::list_nodes_by_lab(db, db::get_lab_id(lab)?)
        .await
        .context("Failed to list the lab's nodes")?;
    lab_info_from_records(lab, owner, &nodes)
}

/// The username of `lab`'s owner, or `unknown` when the owner's record is gone.
///
/// # Errors
///
/// Fails when the lookup fails.
#[instrument(skip(db, lab), fields(lab_id = %lab.lab_id), level = "debug")]
pub(crate) async fn lab_owner(db: &Database, lab: &DbLab) -> Result<String> {
    Ok(db::find_user_by_id(db, lab.user.clone())
        .await
        .context("Failed to look up the lab owner")?
        .map_or_else(|| UNKNOWN_OWNER.to_owned(), |user| user.username))
}

/// A lab's details from its record, its owner's username and its nodes: the networks it
/// reserved, what the user said about it, and the nodes that log in with an account of their
/// own.
///
/// # Errors
///
/// Fails when a network or address in the record does not parse.
pub(crate) fn lab_info_from_records(
    lab: &DbLab,
    owner: String,
    nodes: &[DbNode],
) -> Result<LabInfo> {
    let mut ssh_usernames = Vec::new();
    for node in nodes {
        if let Some(username) = &node.ssh_username {
            ssh_usernames.push(NodeSshUsername {
                node_name: node.name.clone(),
                username: username.clone(),
            });
        }
    }
    Ok(LabInfo {
        ssh_usernames,
        id: lab.lab_id.clone(),
        name: lab.name.clone(),
        user: owner,
        ipv4_network: lab
            .management_network
            .parse()
            .context("Invalid stored management network")?,
        ipv4_gateway: lab
            .gateway_ipv4
            .parse()
            .context("Invalid stored gateway address")?,
        ipv4_router: lab
            .router_ipv4
            .parse()
            .context("Invalid stored router address")?,
        ipv6_network: lab
            .management_network_v6
            .as_deref()
            .map(str::parse)
            .transpose()
            .context("Invalid stored IPv6 management network")?,
        ipv6_gateway: lab
            .gateway_ipv6
            .as_deref()
            .map(str::parse)
            .transpose()
            .context("Invalid stored IPv6 gateway address")?,
        ipv6_router: lab
            .router_ipv6
            .as_deref()
            .map(str::parse)
            .transpose()
            .context("Invalid stored IPv6 router address")?,
        description: lab.description.clone(),
        client_host: lab.client_host.clone(),
        client_folder: lab.client_folder.clone(),
    })
}

#[cfg(test)]
mod tests {
    use shared::data::{LabPhase, NodeModel, RecordId};

    use crate::services::lab_definition::tests::lab_with;

    use super::*;

    #[test]
    fn a_lab_is_described_from_its_record() {
        let lab = DbLab {
            id: Some(RecordId::new("lab", "l1")),
            lab_id: "ab12cd34".to_string(),
            name: "core".to_string(),
            user: RecordId::new("user", "alice"),
            management_network: "172.31.1.0/24".to_string(),
            gateway_ipv4: "172.31.1.1".to_string(),
            router_ipv4: "172.31.1.2".to_string(),
            tailscale_container_id: None,
            management_network_v6: Some("fd00:b00b:0:1::/64".to_string()),
            gateway_ipv6: Some("fd00:b00b:0:1::1".to_string()),
            router_ipv6: None,
            loopback_network_v6: None,
            phase: LabPhase::Interrupted,
            client_host: Some("dev01".to_string()),
            client_folder: None,
            description: Some("BGP".to_string()),
        };

        let info = lab_info_from_records(&lab, "alice".to_string(), &[]).unwrap();

        assert_eq!(info.id, "ab12cd34");
        assert_eq!(info.user, "alice");
        assert_eq!(info.ipv4_network.to_string(), "172.31.1.0/24");
        assert_eq!(info.ipv4_router.to_string(), "172.31.1.2");
        assert_eq!(
            info.ipv6_gateway
                .map(|gateway| gateway.to_string())
                .as_deref(),
            Some("fd00:b00b:0:1::1")
        );
        assert!(info.ipv6_router.is_none());
        assert_eq!(info.client_host.as_deref(), Some("dev01"));
        assert_eq!(info.description.as_deref(), Some("BGP"));
        assert_eq!(info.ssh_usernames, Vec::<NodeSshUsername>::new());
    }

    #[tokio::test]
    async fn saved_lab_files_004_the_lab_info_names_the_lab_and_each_recorded_login() {
        let (db, lab) = lab_with(&[
            ("vm1", NodeModel::AlpineLinux),
            ("vm2", NodeModel::UbuntuLinux),
            ("web", NodeModel::PostgresqlDb),
        ])
        .await;
        let nodes = db::list_nodes_by_lab(&db, lab.id.clone().unwrap())
            .await
            .unwrap();
        let vm1 = nodes.iter().find(|node| node.name == "vm1").unwrap();
        db::update_node_ssh_username(&db, vm1.id.clone().unwrap(), Some("alice"))
            .await
            .unwrap();

        let info = load_lab_info(&db, &lab).await.unwrap();

        let expected = r#"id = "lab-0001"
name = "core"
user = "alice"
ipv4_network = "172.31.1.0/24"
ipv4_gateway = "172.31.1.1"
ipv4_router = "172.31.1.2"

[[ssh_usernames]]
node_name = "vm1"
username = "alice"
"#;
        assert_eq!(info.to_string(), expected);
    }

    #[tokio::test]
    async fn a_lab_whose_owner_is_gone_names_an_unknown_owner() {
        let (db, mut lab) = lab_with(&[]).await;
        lab.user = RecordId::new("user", "gone");
        let info = load_lab_info(&db, &lab).await.unwrap();
        assert_eq!(info.user, "unknown");
    }
}
