use anyhow::Result;
use db::{
    count_links, count_links_by_lab, create_lab, create_link, create_node, create_node_image,
    create_user, get_link, get_link_by_peers,
};
use shared::data::{BridgeKind, NodeConfig, NodeModel};

use crate::helper::{setup_db, teardown_db};

#[tokio::test]
#[ignore]
async fn test_create_link_success() -> Result<()> {
    let db = setup_db("test_create_link").await?;

    // Setup dependencies
    let user = create_user(&db, "testuser".to_string(), "TestPass123!", false, vec![]).await?;
    let lab = create_lab(
        &db,
        "Test Lab",
        "lab-0001",
        &user,
        "127.127.1.0/24",
        "172.31.1.0/24",
    )
    .await?;
    let config = create_node_image(&db, NodeConfig::get_model(NodeModel::UbuntuLinux)).await?;

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
        0,
        BridgeKind::P2pBridge,
        node1.id.clone().unwrap(),
        node2.id.clone().unwrap(),
        "eth0".to_string(),
        "eth0".to_string(),
        "br0".to_string(),
        "br1".to_string(),
        "veth0".to_string(),
        "veth1".to_string(),
        lab.id.clone().unwrap(),
    )
    .await?;

    assert_eq!(link.index, 0);
    assert_eq!(link.node_a, node1.id.unwrap());
    assert_eq!(link.node_b, node2.id.unwrap());
    assert_eq!(link.int_a, "eth0");
    assert_eq!(link.int_b, "eth0");
    assert_eq!(link.lab, lab.id.unwrap());
    assert!(link.id.is_some(), "Link should have an ID");

    teardown_db(&db).await?;
    Ok(())
}

#[tokio::test]
#[ignore]
async fn test_create_link_with_lab() -> Result<()> {
    let db = setup_db("test_create_link_lab").await?;

    let user = create_user(&db, "bob".to_string(), "TestPass123!", false, vec![]).await?;
    let lab = create_lab(
        &db,
        "Bob's Lab",
        "lab-0002",
        &user,
        "127.127.1.0/24",
        "172.31.1.0/24",
    )
    .await?;
    let config = create_node_image(&db, NodeConfig::get_model(NodeModel::CiscoIosv)).await?;

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
        0,
        BridgeKind::P2pVeth,
        node1.id.clone().unwrap(),
        node2.id.clone().unwrap(),
        "GigabitEthernet0/0".to_string(),
        "GigabitEthernet0/0".to_string(),
        "br0".to_string(),
        "br1".to_string(),
        "veth0".to_string(),
        "veth1".to_string(),
        lab.id.clone().unwrap(),
    )
    .await?;

    // Verify link is associated with lab
    assert_eq!(link.lab, lab.id.unwrap());

    teardown_db(&db).await?;
    Ok(())
}

#[tokio::test]
#[ignore]
async fn test_create_link_duplicate_peers_fails() -> Result<()> {
    let db = setup_db("test_create_link_dup_peers").await?;

    let user = create_user(&db, "charlie".to_string(), "TestPass123!", false, vec![]).await?;
    let lab = create_lab(
        &db,
        "Lab One",
        "lab-0003",
        &user,
        "127.127.1.0/24",
        "172.31.1.0/24",
    )
    .await?;
    let config = create_node_image(&db, NodeConfig::get_model(NodeModel::UbuntuLinux)).await?;

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
        0,
        BridgeKind::P2pBridge,
        node1.id.clone().unwrap(),
        node2.id.clone().unwrap(),
        "eth0".to_string(),
        "eth0".to_string(),
        "br0".to_string(),
        "br1".to_string(),
        "veth0".to_string(),
        "veth1".to_string(),
        lab.id.clone().unwrap(),
    )
    .await?;

    // Try to create link with same peer combination
    let result = create_link(
        &db,
        1,
        BridgeKind::P2pBridge,
        node1.id.clone().unwrap(),
        node2.id.clone().unwrap(),
        "eth0".to_string(),
        "eth0".to_string(),
        "br2".to_string(),
        "br3".to_string(),
        "veth2".to_string(),
        "veth3".to_string(),
        lab.id.clone().unwrap(),
    )
    .await;

    assert!(
        result.is_err(),
        "Should fail on duplicate (node_a, node_b, int_a, int_b) combination"
    );
    let error_msg = result.unwrap_err().to_string();
    println!("Duplicate peers error: {}", error_msg);
    assert!(
        error_msg.contains("Failed to create link"),
        "Error should mention failed link creation, got: {}",
        error_msg
    );

    teardown_db(&db).await?;
    Ok(())
}

#[tokio::test]
#[ignore]
async fn test_create_link_different_interfaces_succeeds() -> Result<()> {
    let db = setup_db("test_create_link_diff_ints").await?;

    let user = create_user(&db, "diana".to_string(), "TestPass123!", false, vec![]).await?;
    let lab = create_lab(
        &db,
        "Lab Two",
        "lab-0004",
        &user,
        "127.127.1.0/24",
        "172.31.1.0/24",
    )
    .await?;
    let config = create_node_image(&db, NodeConfig::get_model(NodeModel::CiscoIosv)).await?;

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
        0,
        BridgeKind::P2pBridge,
        node1.id.clone().unwrap(),
        node2.id.clone().unwrap(),
        "GigabitEthernet0/0".to_string(),
        "GigabitEthernet0/0".to_string(),
        "br0".to_string(),
        "br1".to_string(),
        "veth0".to_string(),
        "veth1".to_string(),
        lab.id.clone().unwrap(),
    )
    .await?;

    // Create second link between same nodes on eth1 - should succeed
    let link2 = create_link(
        &db,
        1,
        BridgeKind::P2pBridge,
        node1.id.clone().unwrap(),
        node2.id.clone().unwrap(),
        "GigabitEthernet0/1".to_string(),
        "GigabitEthernet0/1".to_string(),
        "br2".to_string(),
        "br3".to_string(),
        "veth2".to_string(),
        "veth3".to_string(),
        lab.id.clone().unwrap(),
    )
    .await?;

    assert_ne!(link1.id, link2.id, "Links should have different IDs");
    assert_eq!(link1.node_a, link2.node_a);
    assert_eq!(link1.node_b, link2.node_b);
    assert_ne!(link1.int_a, link2.int_a);

    teardown_db(&db).await?;
    Ok(())
}

#[tokio::test]
#[ignore]
async fn test_create_link_with_bridge_kinds() -> Result<()> {
    let db = setup_db("test_create_link_bridge_kinds").await?;

    let user = create_user(&db, "emily".to_string(), "TestPass123!", false, vec![]).await?;
    let lab = create_lab(
        &db,
        "Lab Test",
        "lab-0005",
        &user,
        "127.127.1.0/24",
        "172.31.1.0/24",
    )
    .await?;
    let config = create_node_image(&db, NodeConfig::get_model(NodeModel::UbuntuLinux)).await?;

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

    // Test P2pBridge
    let link1 = create_link(
        &db,
        0,
        BridgeKind::P2pBridge,
        node1.id.clone().unwrap(),
        node2.id.clone().unwrap(),
        "eth0".to_string(),
        "eth0".to_string(),
        "br0".to_string(),
        "br1".to_string(),
        "veth0".to_string(),
        "veth1".to_string(),
        lab.id.clone().unwrap(),
    )
    .await?;

    // Test P2pVeth
    let link2 = create_link(
        &db,
        1,
        BridgeKind::P2pVeth,
        node2.id.clone().unwrap(),
        node3.id.clone().unwrap(),
        "eth0".to_string(),
        "eth0".to_string(),
        "br2".to_string(),
        "br3".to_string(),
        "veth2".to_string(),
        "veth3".to_string(),
        lab.id.clone().unwrap(),
    )
    .await?;

    // Test P2pUdp
    let link3 = create_link(
        &db,
        2,
        BridgeKind::P2pUdp,
        node1.id.clone().unwrap(),
        node3.id.clone().unwrap(),
        "eth1".to_string(),
        "eth1".to_string(),
        "br4".to_string(),
        "br5".to_string(),
        "veth4".to_string(),
        "veth5".to_string(),
        lab.id.clone().unwrap(),
    )
    .await?;

    assert!(matches!(link1.kind, BridgeKind::P2pBridge));
    assert!(matches!(link2.kind, BridgeKind::P2pVeth));
    assert!(matches!(link3.kind, BridgeKind::P2pUdp));

    teardown_db(&db).await?;
    Ok(())
}

#[tokio::test]
#[ignore]
async fn test_create_link_increments_count() -> Result<()> {
    let db = setup_db("test_create_link_count").await?;

    let user = create_user(&db, "frank".to_string(), "TestPass123!", false, vec![]).await?;
    let lab = create_lab(
        &db,
        "Lab Count",
        "lab-0006",
        &user,
        "127.127.1.0/24",
        "172.31.1.0/24",
    )
    .await?;
    let config = create_node_image(&db, NodeConfig::get_model(NodeModel::UbuntuLinux)).await?;

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

    let initial_count = count_links(&db).await?;
    let initial_lab_count = count_links_by_lab(&db, lab.id.clone().unwrap()).await?;

    // Create links
    create_link(
        &db,
        0,
        BridgeKind::P2pBridge,
        node1.id.clone().unwrap(),
        node2.id.clone().unwrap(),
        "eth0".to_string(),
        "eth0".to_string(),
        "br0".to_string(),
        "br1".to_string(),
        "veth0".to_string(),
        "veth1".to_string(),
        lab.id.clone().unwrap(),
    )
    .await?;
    create_link(
        &db,
        1,
        BridgeKind::P2pVeth,
        node1.id.clone().unwrap(),
        node2.id.clone().unwrap(),
        "eth1".to_string(),
        "eth1".to_string(),
        "br2".to_string(),
        "br3".to_string(),
        "veth2".to_string(),
        "veth3".to_string(),
        lab.id.clone().unwrap(),
    )
    .await?;

    let final_count = count_links(&db).await?;
    let final_lab_count = count_links_by_lab(&db, lab.id.clone().unwrap()).await?;

    assert_eq!(final_count, initial_count + 2);
    assert_eq!(final_lab_count, initial_lab_count + 2);

    teardown_db(&db).await?;
    Ok(())
}

#[tokio::test]
#[ignore]
async fn test_create_link_can_retrieve_by_peers() -> Result<()> {
    let db = setup_db("test_create_link_retrieve").await?;

    let user = create_user(&db, "grace".to_string(), "TestPass123!", false, vec![]).await?;
    let lab = create_lab(
        &db,
        "Lab Retrieve",
        "lab-0007",
        &user,
        "127.127.1.0/24",
        "172.31.1.0/24",
    )
    .await?;
    let config = create_node_image(&db, NodeConfig::get_model(NodeModel::CiscoIosv)).await?;

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

    // Create link
    let created_link = create_link(
        &db,
        0,
        BridgeKind::P2pBridge,
        node1.id.clone().unwrap(),
        node2.id.clone().unwrap(),
        "GigabitEthernet0/0".to_string(),
        "GigabitEthernet0/0".to_string(),
        "br0".to_string(),
        "br1".to_string(),
        "veth0".to_string(),
        "veth1".to_string(),
        lab.id.clone().unwrap(),
    )
    .await?;

    // Retrieve by peers
    let retrieved_link = get_link_by_peers(
        &db,
        node1.id.clone().unwrap(),
        node2.id.clone().unwrap(),
        "GigabitEthernet0/0",
        "GigabitEthernet0/0",
    )
    .await?;

    assert_eq!(created_link.id, retrieved_link.id);
    assert_eq!(created_link.index, retrieved_link.index);

    teardown_db(&db).await?;
    Ok(())
}

#[tokio::test]
#[ignore]
async fn test_create_link_can_retrieve_by_id() -> Result<()> {
    let db = setup_db("test_create_link_retrieve_id").await?;

    let user = create_user(&db, "hannah".to_string(), "TestPass123!", false, vec![]).await?;
    let lab = create_lab(
        &db,
        "Lab Retrieve ID",
        "lab-0008",
        &user,
        "127.127.1.0/24",
        "172.31.1.0/24",
    )
    .await?;
    let config = create_node_image(&db, NodeConfig::get_model(NodeModel::UbuntuLinux)).await?;

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
    let created_link = create_link(
        &db,
        0,
        BridgeKind::P2pVeth,
        node1.id.clone().unwrap(),
        node2.id.clone().unwrap(),
        "eth0".to_string(),
        "eth0".to_string(),
        "br0".to_string(),
        "br1".to_string(),
        "veth0".to_string(),
        "veth1".to_string(),
        lab.id.clone().unwrap(),
    )
    .await?;

    // Retrieve by ID
    let link_id = created_link.id.clone().unwrap();
    let retrieved_link = get_link(&db, link_id).await?;

    assert_eq!(created_link.id, retrieved_link.id);
    assert_eq!(created_link.index, retrieved_link.index);
    assert_eq!(created_link.node_a, retrieved_link.node_a);
    assert_eq!(created_link.node_b, retrieved_link.node_b);

    teardown_db(&db).await?;
    Ok(())
}
