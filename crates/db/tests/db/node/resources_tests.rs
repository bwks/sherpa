use anyhow::Result;
use db::{
    DefaultRequest, LabDetails, create_lab, create_node, create_user, get_node,
    update_node_host_key, update_node_mgmt_ipv4, update_node_mgmt_ipv6, update_node_mgmt_mac,
    update_node_resources, update_node_state_from, upsert_node_image,
};
use shared::data::{ModelKind, NodeModel, NodeResources, NodeState};

use crate::helper::{ipv4_lab_networks, setup_db};

async fn lab_with_nodes(db: &db::Database, count: u16) -> Result<Vec<shared::data::DbNode>> {
    let user = create_user(db, "alice".to_string(), "TestPass123!", false, vec![]).await?;
    let lab = create_lab(
        db,
        "Test Lab",
        "lab-0001",
        &user,
        &ipv4_lab_networks("172.31.1.0/24", "172.31.1.1", "172.31.1.2"),
        &LabDetails::default(),
    )
    .await?;
    let image = upsert_node_image(
        db,
        ModelKind::default_for(NodeModel::UbuntuLinux).node_config(),
        DefaultRequest::Keep,
    )
    .await?;
    let mut nodes = Vec::new();
    for index in 1..=count {
        nodes.push(
            create_node(
                db,
                &format!("node{index}"),
                index,
                image.id.clone().unwrap(),
                lab.id.clone().unwrap(),
            )
            .await?,
        );
    }
    Ok(nodes)
}

#[tokio::test]
async fn test_node_resources_are_stored() -> Result<()> {
    let db = setup_db().await?;
    let nodes = lab_with_nodes(&db, 1).await?;
    let id = nodes[0].id.clone().unwrap();
    assert_eq!(nodes[0].cpu_count, None);

    let resources = NodeResources {
        cpu_count: Some(16),
        memory: Some(32768),
    };
    let updated = update_node_resources(&db, id.clone(), &resources).await?;
    assert_eq!(updated.cpu_count, Some(16));
    assert_eq!(updated.memory, Some(32768));

    let read = get_node(&db, id).await?;
    assert_eq!(read.cpu_count, Some(16));
    assert_eq!(read.memory, Some(32768));

    Ok(())
}

#[tokio::test]
async fn node_host_keys_014_a_recorded_key_is_kept_through_state_changes_and_can_be_cleared()
-> Result<()> {
    let db = setup_db().await?;
    let nodes = lab_with_nodes(&db, 2).await?;
    let id = nodes[0].id.clone().unwrap();
    assert_eq!(nodes[0].ssh_host_key, None);

    let key = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIJdD7y3aLq454yWBdwLWbieU1ebz9/cu7/QEXn9OIeZJ";
    let updated = update_node_host_key(&db, id.clone(), Some(key)).await?;
    assert_eq!(updated.ssh_host_key.as_deref(), Some(key));

    // Other updates keep the key.
    update_node_state_from(&db, id.clone(), NodeState::Building, NodeState::Starting).await?;
    assert_eq!(
        get_node(&db, id.clone()).await?.ssh_host_key.as_deref(),
        Some(key)
    );
    let other = get_node(&db, nodes[1].id.clone().unwrap()).await?;
    assert_eq!(other.ssh_host_key, None);

    update_node_host_key(&db, id.clone(), None).await?;
    assert_eq!(get_node(&db, id).await?.ssh_host_key, None);

    Ok(())
}

#[tokio::test]
async fn management_addresses_are_stored() -> Result<()> {
    let db = setup_db().await?;
    let nodes = lab_with_nodes(&db, 1).await?;
    let id = nodes[0].id.clone().unwrap();

    update_node_mgmt_ipv4(&db, id.clone(), "172.31.1.21").await?;
    update_node_mgmt_ipv6(&db, id.clone(), "fd00:b00b:0:1::21").await?;
    update_node_mgmt_mac(&db, id.clone(), "52:54:00:12:34:56").await?;

    let node = get_node(&db, id).await?;
    assert_eq!(node.mgmt_ipv4.as_deref(), Some("172.31.1.21"));
    assert_eq!(node.mgmt_ipv6.as_deref(), Some("fd00:b00b:0:1::21"));
    assert_eq!(node.mgmt_mac.as_deref(), Some("52:54:00:12:34:56"));
    Ok(())
}
