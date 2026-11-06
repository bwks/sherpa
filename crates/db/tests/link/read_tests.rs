use anyhow::Result;
use db::{LabOrigin, create_lab, create_link, create_user, list_links_by_lab, upsert_node_image};
use shared::data::{BridgeKind, NodeConfig, NodeModel};

use crate::helper::{create_test_node_with_model, ipv4_lab_networks, setup_db, test_link};

#[tokio::test]
async fn test_list_links_by_lab_empty() -> Result<()> {
    let db = setup_db().await?;

    let user = create_user(&db, "testuser".to_string(), "TestPass123!", false, vec![]).await?;
    let lab = create_lab(
        &db,
        "Test Lab",
        "lab-0001",
        &user,
        &ipv4_lab_networks(
            "127.127.1.0/24",
            "172.31.1.0/24",
            "172.31.1.1",
            "172.31.1.2",
        ),
        &LabOrigin::default(),
    )
    .await?;

    let links = list_links_by_lab(&db, lab.id.clone().unwrap()).await?;

    assert_eq!(links.len(), 0);

    Ok(())
}

#[tokio::test]
async fn test_list_links_by_lab_filters_correctly() -> Result<()> {
    let db = setup_db().await?;

    let user = create_user(&db, "testuser".to_string(), "TestPass123!", false, vec![]).await?;
    let lab1 = create_lab(
        &db,
        "Lab 1",
        "lab-0001",
        &user,
        &ipv4_lab_networks(
            "127.127.1.0/24",
            "172.31.1.0/24",
            "172.31.1.1",
            "172.31.1.2",
        ),
        &LabOrigin::default(),
    )
    .await?;
    let lab2 = create_lab(
        &db,
        "Lab 2",
        "lab-0002",
        &user,
        &ipv4_lab_networks(
            "127.127.2.0/24",
            "172.31.2.0/24",
            "172.31.1.1",
            "172.31.1.2",
        ),
        &LabOrigin::default(),
    )
    .await?;
    let _config = upsert_node_image(&db, NodeConfig::get_model(NodeModel::UbuntuLinux)).await?;

    // Create nodes for lab1
    let node1_lab1 =
        create_test_node_with_model(&db, "node1", 1, NodeModel::UbuntuLinux, &lab1).await?;
    let node2_lab1 =
        create_test_node_with_model(&db, "node2", 2, NodeModel::UbuntuLinux, &lab1).await?;

    // Create nodes for lab2
    let node1_lab2 =
        create_test_node_with_model(&db, "node1", 1, NodeModel::UbuntuLinux, &lab2).await?;
    let node2_lab2 =
        create_test_node_with_model(&db, "node2", 2, NodeModel::UbuntuLinux, &lab2).await?;

    // Create link in lab1
    create_link(
        &db,
        test_link(
            &lab1,
            1,
            BridgeKind::P2p,
            &node1_lab1,
            "eth0",
            &node2_lab1,
            "eth0",
        ),
    )
    .await?;

    // Create links in lab2
    create_link(
        &db,
        test_link(
            &lab2,
            1,
            BridgeKind::P2p,
            &node1_lab2,
            "eth0",
            &node2_lab2,
            "eth0",
        ),
    )
    .await?;

    create_link(
        &db,
        test_link(
            &lab2,
            2,
            BridgeKind::SharedBridge,
            &node1_lab2,
            "eth1",
            &node2_lab2,
            "eth1",
        ),
    )
    .await?;

    let links_lab1 = list_links_by_lab(&db, lab1.id.clone().unwrap()).await?;
    let links_lab2 = list_links_by_lab(&db, lab2.id.clone().unwrap()).await?;

    assert_eq!(links_lab1.len(), 1);
    assert_eq!(links_lab2.len(), 2);

    // Verify lab1 link belongs to lab1
    assert_eq!(links_lab1[0].lab, lab1.id.unwrap());

    // Verify lab2 links belong to lab2
    for link in &links_lab2 {
        assert_eq!(link.lab, lab2.id.clone().unwrap());
    }

    Ok(())
}
