use anyhow::Result;
use db::{
    create_lab, create_node, create_node_image, create_user, get_node, list_vnc_ports,
    update_node_resources,
};
use shared::data::{NodeConfig, NodeModel, NodeResources};

use crate::helper::{ipv4_lab_networks, setup_db, teardown_db};

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
#[ignore]
async fn test_node_resources_are_stored() -> Result<()> {
    let db = setup_db("test_node_resources_stored").await?;
    let nodes = lab_with_nodes(&db, 1).await?;
    let id = nodes[0].id.clone().unwrap();
    assert_eq!(nodes[0].cpu_count, None);
    assert_eq!(nodes[0].vnc_port, None);

    let resources = NodeResources {
        cpu_count: Some(16),
        memory: Some(32768),
        vnc_port: Some(5901),
    };
    let updated = update_node_resources(&db, id.clone(), &resources).await?;
    assert_eq!(updated.cpu_count, Some(16));
    assert_eq!(updated.memory, Some(32768));
    assert_eq!(updated.vnc_port, Some(5901));

    let read = get_node(&db, id).await?;
    assert_eq!(read.cpu_count, Some(16));
    assert_eq!(read.memory, Some(32768));
    assert_eq!(read.vnc_port, Some(5901));

    teardown_db(&db).await?;
    Ok(())
}

#[tokio::test]
#[ignore]
async fn test_vnc_port_belongs_to_one_node() -> Result<()> {
    let db = setup_db("test_vnc_port_unique").await?;
    let nodes = lab_with_nodes(&db, 3).await?;
    update_node_resources(&db, nodes[0].id.clone().unwrap(), &vnc(Some(5900))).await?;
    let taken = update_node_resources(&db, nodes[1].id.clone().unwrap(), &vnc(Some(5900))).await;
    assert!(taken.is_err(), "a VNC port must belong to one node");

    // Nodes without a VNC port, such as containers, do not conflict.
    update_node_resources(&db, nodes[1].id.clone().unwrap(), &vnc(None)).await?;
    update_node_resources(&db, nodes[2].id.clone().unwrap(), &vnc(None)).await?;

    assert_eq!(list_vnc_ports(&db).await?, vec![5900]);

    teardown_db(&db).await?;
    Ok(())
}
