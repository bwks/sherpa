use anyhow::Result;
use db::{
    create_lab, create_node, create_node_image, create_user, get_node, list_pinned_nodes,
    list_vnc_ports, update_node_host_key, update_node_resources, update_node_state,
};
use shared::data::{NodeConfig, NodeModel, NodeResources, NodeState};

use crate::helper::{ipv4_lab_networks, setup_db};

async fn lab_with_nodes(db: &db::Database, count: u16) -> Result<Vec<shared::data::DbNode>> {
    let user = create_user(db, "alice".to_string(), "TestPass123!", false, vec![]).await?;
    let lab = create_lab(
        db,
        "Test Lab",
        "lab-0001",
        &user,
        &ipv4_lab_networks(
            "127.127.1.0/24",
            "172.31.1.0/24",
            "172.31.1.1",
            "172.31.1.2",
        ),
    )
    .await?;
    let image = create_node_image(db, NodeConfig::get_model(NodeModel::UbuntuLinux)).await?;
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

fn vnc(vnc_port: Option<u16>) -> NodeResources {
    NodeResources {
        vnc_port,
        ..NodeResources::default()
    }
}

#[tokio::test]
async fn test_node_resources_are_stored() -> Result<()> {
    let db = setup_db().await?;
    let nodes = lab_with_nodes(&db, 1).await?;
    let id = nodes[0].id.clone().unwrap();
    assert_eq!(nodes[0].cpu_count, None);
    assert_eq!(nodes[0].vnc_port, None);

    let resources = NodeResources {
        cpu_count: Some(16),
        memory: Some(32768),
        vnc_port: Some(5901),
        ..NodeResources::default()
    };
    let updated = update_node_resources(&db, id.clone(), &resources).await?;
    assert_eq!(updated.cpu_count, Some(16));
    assert_eq!(updated.memory, Some(32768));
    assert_eq!(updated.vnc_port, Some(5901));

    let read = get_node(&db, id).await?;
    assert_eq!(read.cpu_count, Some(16));
    assert_eq!(read.memory, Some(32768));
    assert_eq!(read.vnc_port, Some(5901));

    Ok(())
}

#[tokio::test]
async fn test_pinned_cpus_are_stored_and_listed() -> Result<()> {
    let db = setup_db().await?;
    let nodes = lab_with_nodes(&db, 2).await?;
    let id = nodes[0].id.clone().unwrap();
    assert!(list_pinned_nodes(&db).await?.is_empty());

    let resources = NodeResources {
        cpu_count: Some(2),
        cpu_pinning: Some(vec![9, 25]),
        qemu_thread_cpus: Some(vec![8, 24]),
        ..NodeResources::default()
    };
    update_node_resources(&db, id.clone(), &resources).await?;
    let read = get_node(&db, id).await?;
    assert_eq!(read.cpu_pinning, Some(vec![9, 25]));
    assert_eq!(read.qemu_thread_cpus, Some(vec![8, 24]));

    // Only nodes with pins are listed.
    let pinned = list_pinned_nodes(&db).await?;
    assert_eq!(pinned.len(), 1);
    assert_eq!(pinned[0].name, nodes[0].name);

    Ok(())
}

#[tokio::test]
async fn test_vnc_port_belongs_to_one_node() -> Result<()> {
    let db = setup_db().await?;
    let nodes = lab_with_nodes(&db, 3).await?;
    update_node_resources(&db, nodes[0].id.clone().unwrap(), &vnc(Some(5900))).await?;
    let taken = update_node_resources(&db, nodes[1].id.clone().unwrap(), &vnc(Some(5900))).await;
    assert!(taken.is_err(), "a VNC port must belong to one node");

    // Nodes without a VNC port, such as containers, do not conflict.
    update_node_resources(&db, nodes[1].id.clone().unwrap(), &vnc(None)).await?;
    update_node_resources(&db, nodes[2].id.clone().unwrap(), &vnc(None)).await?;

    assert_eq!(list_vnc_ports(&db).await?, vec![5900]);

    Ok(())
}

#[tokio::test]
async fn test_node_host_key_is_stored_and_cleared() -> Result<()> {
    let db = setup_db().await?;
    let nodes = lab_with_nodes(&db, 2).await?;
    let id = nodes[0].id.clone().unwrap();
    assert_eq!(nodes[0].ssh_host_key, None);

    let key = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIJdD7y3aLq454yWBdwLWbieU1ebz9/cu7/QEXn9OIeZJ";
    let updated = update_node_host_key(&db, id.clone(), Some(key)).await?;
    assert_eq!(updated.ssh_host_key.as_deref(), Some(key));

    // Other updates keep the key.
    update_node_state(&db, id.clone(), NodeState::Running).await?;
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
