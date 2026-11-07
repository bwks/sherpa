use anyhow::Result;
use db::{
    LabOrigin, create_lab, create_link, create_node, create_user, list_links_by_lab,
    upsert_node_image,
};
use shared::data::{BridgeKind, NodeConfig, NodeModel};

use crate::helper::{ipv4_lab_networks, setup_db, test_link};

#[tokio::test]
async fn test_create_link_success() -> Result<()> {
    let db = setup_db().await?;

    // Setup dependencies
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
    let config = upsert_node_image(&db, NodeConfig::get_model(NodeModel::UbuntuLinux)).await?;

    let node1 = create_node(
        &db,
        "node1",
        1,
        config.id.clone().unwrap(),
        lab.id.clone().unwrap(),
    )
    .await?;
    let node2 = create_node(
        &db,
        "node2",
        2,
        config.id.clone().unwrap(),
        lab.id.clone().unwrap(),
    )
    .await?;

    // Create link
    let link = create_link(
        &db,
        test_link(&lab, 0, BridgeKind::P2p, &node1, "eth0", &node2, "eth0"),
    )
    .await?;

    assert_eq!(link.index, 0);
    assert_eq!(link.node_a, node1.id.unwrap());
    assert_eq!(link.node_b, node2.id.unwrap());
    assert_eq!(link.int_a, "eth0");
    assert_eq!(link.int_b, "eth0");
    assert_eq!(link.lab, lab.id.unwrap());
    assert!(link.id.is_some(), "Link should have an ID");

    Ok(())
}

#[tokio::test]
async fn test_create_link_with_lab() -> Result<()> {
    let db = setup_db().await?;

    let user = create_user(&db, "bob".to_string(), "TestPass123!", false, vec![]).await?;
    let lab = create_lab(
        &db,
        "Bob's Lab",
        "lab-0002",
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
    let config = upsert_node_image(&db, NodeConfig::get_model(NodeModel::CiscoIosv)).await?;

    let node1 = create_node(
        &db,
        "router1",
        1,
        config.id.clone().unwrap(),
        lab.id.clone().unwrap(),
    )
    .await?;
    let node2 = create_node(
        &db,
        "router2",
        2,
        config.id.clone().unwrap(),
        lab.id.clone().unwrap(),
    )
    .await?;

    let link = create_link(
        &db,
        test_link(
            &lab,
            0,
            BridgeKind::P2p,
            &node1,
            "GigabitEthernet0/0",
            &node2,
            "GigabitEthernet0/0",
        ),
    )
    .await?;

    // Verify link is associated with lab
    assert_eq!(link.lab, lab.id.unwrap());

    Ok(())
}

#[tokio::test]
async fn test_create_link_duplicate_peers_fails() -> Result<()> {
    let db = setup_db().await?;

    let user = create_user(&db, "charlie".to_string(), "TestPass123!", false, vec![]).await?;
    let lab = create_lab(
        &db,
        "Lab One",
        "lab-0003",
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
    let config = upsert_node_image(&db, NodeConfig::get_model(NodeModel::UbuntuLinux)).await?;

    let node1 = create_node(
        &db,
        "node1",
        1,
        config.id.clone().unwrap(),
        lab.id.clone().unwrap(),
    )
    .await?;
    let node2 = create_node(
        &db,
        "node2",
        2,
        config.id.clone().unwrap(),
        lab.id.clone().unwrap(),
    )
    .await?;

    // Create first link
    create_link(
        &db,
        test_link(&lab, 0, BridgeKind::P2p, &node1, "eth0", &node2, "eth0"),
    )
    .await?;

    // Try to create link with same peer combination
    let result = create_link(
        &db,
        test_link(&lab, 1, BridgeKind::P2p, &node1, "eth0", &node2, "eth0"),
    )
    .await;

    assert!(
        result.is_err(),
        "Should fail on duplicate (node_a, node_b, int_a, int_b) combination"
    );
    let error_msg = result.unwrap_err().to_string();
    println!("Duplicate peers error: {error_msg}");
    assert!(
        error_msg.contains("Failed to create link"),
        "Error should mention failed link creation, got: {error_msg}"
    );

    Ok(())
}

#[tokio::test]
async fn test_create_link_different_interfaces_succeeds() -> Result<()> {
    let db = setup_db().await?;

    let user = create_user(&db, "diana".to_string(), "TestPass123!", false, vec![]).await?;
    let lab = create_lab(
        &db,
        "Lab Two",
        "lab-0004",
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
    let config = upsert_node_image(&db, NodeConfig::get_model(NodeModel::CiscoIosv)).await?;

    let node1 = create_node(
        &db,
        "router1",
        1,
        config.id.clone().unwrap(),
        lab.id.clone().unwrap(),
    )
    .await?;
    let node2 = create_node(
        &db,
        "router2",
        2,
        config.id.clone().unwrap(),
        lab.id.clone().unwrap(),
    )
    .await?;

    // Create first link between same nodes on eth0
    let link1 = create_link(
        &db,
        test_link(
            &lab,
            0,
            BridgeKind::P2p,
            &node1,
            "GigabitEthernet0/0",
            &node2,
            "GigabitEthernet0/0",
        ),
    )
    .await?;

    // Create second link between same nodes on eth1 - should succeed
    let link2 = create_link(
        &db,
        test_link(
            &lab,
            1,
            BridgeKind::P2p,
            &node1,
            "GigabitEthernet0/1",
            &node2,
            "GigabitEthernet0/1",
        ),
    )
    .await?;

    assert_ne!(link1.id, link2.id, "Links should have different IDs");
    assert_eq!(link1.node_a, link2.node_a);
    assert_eq!(link1.node_b, link2.node_b);
    assert_ne!(link1.int_a, link2.int_a);

    Ok(())
}

#[tokio::test]
async fn test_create_link_with_bridge_kinds() -> Result<()> {
    let db = setup_db().await?;

    let user = create_user(&db, "emily".to_string(), "TestPass123!", false, vec![]).await?;
    let lab = create_lab(
        &db,
        "Lab Test",
        "lab-0005",
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
    let config = upsert_node_image(&db, NodeConfig::get_model(NodeModel::UbuntuLinux)).await?;

    let node1 = create_node(
        &db,
        "node1",
        1,
        config.id.clone().unwrap(),
        lab.id.clone().unwrap(),
    )
    .await?;
    let node2 = create_node(
        &db,
        "node2",
        2,
        config.id.clone().unwrap(),
        lab.id.clone().unwrap(),
    )
    .await?;
    let node3 = create_node(
        &db,
        "node3",
        3,
        config.id.clone().unwrap(),
        lab.id.clone().unwrap(),
    )
    .await?;

    // Test P2p
    let link2 = create_link(
        &db,
        test_link(&lab, 1, BridgeKind::P2p, &node2, "eth0", &node3, "eth0"),
    )
    .await?;

    // Test SharedBridge
    let link3 = create_link(
        &db,
        test_link(
            &lab,
            2,
            BridgeKind::SharedBridge,
            &node1,
            "eth1",
            &node3,
            "eth1",
        ),
    )
    .await?;

    assert!(matches!(link2.kind, BridgeKind::P2p));
    assert!(matches!(link3.kind, BridgeKind::SharedBridge));

    Ok(())
}

#[tokio::test]
async fn test_create_link_increments_count() -> Result<()> {
    let db = setup_db().await?;

    let user = create_user(&db, "frank".to_string(), "TestPass123!", false, vec![]).await?;
    let lab = create_lab(
        &db,
        "Lab Count",
        "lab-0006",
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
    let config = upsert_node_image(&db, NodeConfig::get_model(NodeModel::UbuntuLinux)).await?;

    let node1 = create_node(
        &db,
        "node1",
        1,
        config.id.clone().unwrap(),
        lab.id.clone().unwrap(),
    )
    .await?;
    let node2 = create_node(
        &db,
        "node2",
        2,
        config.id.clone().unwrap(),
        lab.id.clone().unwrap(),
    )
    .await?;

    let initial_lab_count = list_links_by_lab(&db, lab.id.clone().unwrap()).await?.len();

    // Create links
    create_link(
        &db,
        test_link(&lab, 0, BridgeKind::P2p, &node1, "eth0", &node2, "eth0"),
    )
    .await?;
    create_link(
        &db,
        test_link(&lab, 1, BridgeKind::P2p, &node1, "eth1", &node2, "eth1"),
    )
    .await?;

    let final_lab_count = list_links_by_lab(&db, lab.id.clone().unwrap()).await?.len();

    assert_eq!(final_lab_count, initial_lab_count + 2);

    Ok(())
}
