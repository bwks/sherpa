use anyhow::Result;
use db::{
    count_links, count_links_by_lab, count_links_by_node, create_lab, create_link, create_node,
    create_node_config, create_user, delete_link, delete_link_by_id, delete_links_by_lab,
    delete_links_by_node, get_link_by_id,
};
use shared::data::{BridgeKind, NodeConfig, NodeModel};

use crate::helper::{setup_db, teardown_db};

#[tokio::test]
#[ignore]
async fn test_delete_link_success() -> Result<()> {
    let db = setup_db("link_delete_success").await?;

    let user = create_user(&db, "testuser".to_string(), vec![]).await?;
    let lab = create_lab(&db, "Test Lab", "lab-0001", &user).await?;
    let config = create_node_config(&db, NodeConfig::get_model(NodeModel::UbuntuLinux)).await?;

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

    let link = create_link(
        &db,
        1,
        BridgeKind::P2pBridge,
        node1.id.clone().unwrap(),
        node2.id.clone().unwrap(),
        "eth0".to_string(),
        "eth0".to_string(),
        "br-1".to_string(),
        "br-2".to_string(),
        "veth-1".to_string(),
        "veth-2".to_string(),
        lab.id.clone().unwrap(),
    )
    .await?;

    let link_id = link.id.clone().unwrap();

    // Delete the link
    delete_link(&db, link_id.clone()).await?;

    // Verify it's deleted
    let result = get_link_by_id(&db, link_id).await;
    assert!(result.is_err());

    teardown_db(&db).await?;
    Ok(())
}

#[tokio::test]
#[ignore]
async fn test_delete_link_by_id_success() -> Result<()> {
    let db = setup_db("link_delete_by_id_success").await?;

    let user = create_user(&db, "testuser".to_string(), vec![]).await?;
    let lab = create_lab(&db, "Test Lab", "lab-0001", &user).await?;
    let config = create_node_config(&db, NodeConfig::get_model(NodeModel::UbuntuLinux)).await?;

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

    let link = create_link(
        &db,
        1,
        BridgeKind::P2pBridge,
        node1.id.clone().unwrap(),
        node2.id.clone().unwrap(),
        "eth0".to_string(),
        "eth0".to_string(),
        "br-1".to_string(),
        "br-2".to_string(),
        "veth-1".to_string(),
        "veth-2".to_string(),
        lab.id.clone().unwrap(),
    )
    .await?;

    let link_id = link.id.clone().unwrap();

    // Delete by ID
    delete_link_by_id(&db, link_id.clone()).await?;

    // Verify it's deleted
    let result = get_link_by_id(&db, link_id).await;
    assert!(result.is_err());

    teardown_db(&db).await?;
    Ok(())
}

#[tokio::test]
#[ignore]
async fn test_delete_link_nonexistent_fails() -> Result<()> {
    let db = setup_db("link_delete_nonexistent").await?;

    let user = create_user(&db, "testuser".to_string(), vec![]).await?;
    let lab = create_lab(&db, "Test Lab", "lab-0001", &user).await?;
    let config = create_node_config(&db, NodeConfig::get_model(NodeModel::UbuntuLinux)).await?;

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

    let link = create_link(
        &db,
        1,
        BridgeKind::P2pBridge,
        node1.id.clone().unwrap(),
        node2.id.clone().unwrap(),
        "eth0".to_string(),
        "eth0".to_string(),
        "br-1".to_string(),
        "br-2".to_string(),
        "veth-1".to_string(),
        "veth-2".to_string(),
        lab.id.clone().unwrap(),
    )
    .await?;

    let link_id = link.id.clone().unwrap();

    // Delete the link first
    delete_link(&db, link_id.clone()).await?;

    // Try to delete again (should fail)
    let result = delete_link(&db, link_id).await;

    assert!(result.is_err());
    let err_msg = result.unwrap_err().to_string();
    assert!(err_msg.contains("not found") || err_msg.contains("does not exist"));

    teardown_db(&db).await?;
    Ok(())
}

#[tokio::test]
#[ignore]
async fn test_delete_link_decrements_count() -> Result<()> {
    let db = setup_db("link_delete_decrements_count").await?;

    let user = create_user(&db, "testuser".to_string(), vec![]).await?;
    let lab = create_lab(&db, "Test Lab", "lab-0001", &user).await?;
    let config = create_node_config(&db, NodeConfig::get_model(NodeModel::UbuntuLinux)).await?;

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

    let link1 = create_link(
        &db,
        1,
        BridgeKind::P2pBridge,
        node1.id.clone().unwrap(),
        node2.id.clone().unwrap(),
        "eth0".to_string(),
        "eth0".to_string(),
        "br-1".to_string(),
        "br-2".to_string(),
        "veth-1".to_string(),
        "veth-2".to_string(),
        lab.id.clone().unwrap(),
    )
    .await?;

    let link2 = create_link(
        &db,
        2,
        BridgeKind::P2pVeth,
        node1.id.clone().unwrap(),
        node2.id.clone().unwrap(),
        "eth1".to_string(),
        "eth1".to_string(),
        "br-3".to_string(),
        "br-4".to_string(),
        "veth-3".to_string(),
        "veth-4".to_string(),
        lab.id.clone().unwrap(),
    )
    .await?;

    let count_before = count_links(&db).await?;
    assert_eq!(count_before, 2);

    // Delete one link
    delete_link(&db, link1.id.clone().unwrap()).await?;

    let count_after_one = count_links(&db).await?;
    assert_eq!(count_after_one, 1);

    // Delete the other link
    delete_link(&db, link2.id.clone().unwrap()).await?;

    let count_after_two = count_links(&db).await?;
    assert_eq!(count_after_two, 0);

    teardown_db(&db).await?;
    Ok(())
}

#[tokio::test]
#[ignore]
async fn test_delete_links_by_lab_success() -> Result<()> {
    let db = setup_db("link_delete_by_lab_success").await?;

    let user = create_user(&db, "testuser".to_string(), vec![]).await?;
    let lab = create_lab(&db, "Test Lab", "lab-0001", &user).await?;
    let config = create_node_config(&db, NodeConfig::get_model(NodeModel::UbuntuLinux)).await?;

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

    // Create multiple links in the lab
    create_link(
        &db,
        1,
        BridgeKind::P2pBridge,
        node1.id.clone().unwrap(),
        node2.id.clone().unwrap(),
        "eth0".to_string(),
        "eth0".to_string(),
        "br-1".to_string(),
        "br-2".to_string(),
        "veth-1".to_string(),
        "veth-2".to_string(),
        lab.id.clone().unwrap(),
    )
    .await?;

    create_link(
        &db,
        2,
        BridgeKind::P2pVeth,
        node2.id.clone().unwrap(),
        node3.id.clone().unwrap(),
        "eth0".to_string(),
        "eth0".to_string(),
        "br-3".to_string(),
        "br-4".to_string(),
        "veth-3".to_string(),
        "veth-4".to_string(),
        lab.id.clone().unwrap(),
    )
    .await?;

    create_link(
        &db,
        3,
        BridgeKind::P2pUdp,
        node1.id.clone().unwrap(),
        node3.id.clone().unwrap(),
        "eth1".to_string(),
        "eth1".to_string(),
        "br-5".to_string(),
        "br-6".to_string(),
        "veth-5".to_string(),
        "veth-6".to_string(),
        lab.id.clone().unwrap(),
    )
    .await?;

    let count_before = count_links_by_lab(&db, lab.id.clone().unwrap()).await?;
    assert_eq!(count_before, 3);

    // Delete all links by lab
    delete_links_by_lab(&db, lab.id.clone().unwrap()).await?;

    let count_after = count_links_by_lab(&db, lab.id.clone().unwrap()).await?;
    assert_eq!(count_after, 0);

    teardown_db(&db).await?;
    Ok(())
}

#[tokio::test]
#[ignore]
async fn test_delete_links_by_lab_only_affects_one_lab() -> Result<()> {
    let db = setup_db("link_delete_by_lab_isolation").await?;

    let user = create_user(&db, "testuser".to_string(), vec![]).await?;
    let lab1 = create_lab(&db, "Lab 1", "lab-0001", &user).await?;
    let lab2 = create_lab(&db, "Lab 2", "lab-0002", &user).await?;
    let config = create_node_config(&db, NodeConfig::get_model(NodeModel::UbuntuLinux)).await?;

    // Create nodes for lab1
    let node1_lab1 = create_node(
        &db,
        "node1",
        1,
        config.id.clone().unwrap(),
        lab1.id.clone().unwrap(),
    )
    .await?;
    let node2_lab1 = create_node(
        &db,
        "node2",
        2,
        config.id.clone().unwrap(),
        lab1.id.clone().unwrap(),
    )
    .await?;

    // Create nodes for lab2
    let node1_lab2 = create_node(
        &db,
        "node1",
        1,
        config.id.clone().unwrap(),
        lab2.id.clone().unwrap(),
    )
    .await?;
    let node2_lab2 = create_node(
        &db,
        "node2",
        2,
        config.id.clone().unwrap(),
        lab2.id.clone().unwrap(),
    )
    .await?;

    // Create links in lab1
    create_link(
        &db,
        1,
        BridgeKind::P2pBridge,
        node1_lab1.id.clone().unwrap(),
        node2_lab1.id.clone().unwrap(),
        "eth0".to_string(),
        "eth0".to_string(),
        "br-1".to_string(),
        "br-2".to_string(),
        "veth-1".to_string(),
        "veth-2".to_string(),
        lab1.id.clone().unwrap(),
    )
    .await?;

    create_link(
        &db,
        2,
        BridgeKind::P2pVeth,
        node1_lab1.id.clone().unwrap(),
        node2_lab1.id.clone().unwrap(),
        "eth1".to_string(),
        "eth1".to_string(),
        "br-3".to_string(),
        "br-4".to_string(),
        "veth-3".to_string(),
        "veth-4".to_string(),
        lab1.id.clone().unwrap(),
    )
    .await?;

    // Create links in lab2
    create_link(
        &db,
        1,
        BridgeKind::P2pUdp,
        node1_lab2.id.clone().unwrap(),
        node2_lab2.id.clone().unwrap(),
        "eth0".to_string(),
        "eth0".to_string(),
        "br-5".to_string(),
        "br-6".to_string(),
        "veth-5".to_string(),
        "veth-6".to_string(),
        lab2.id.clone().unwrap(),
    )
    .await?;

    let count_lab1_before = count_links_by_lab(&db, lab1.id.clone().unwrap()).await?;
    let count_lab2_before = count_links_by_lab(&db, lab2.id.clone().unwrap()).await?;
    assert_eq!(count_lab1_before, 2);
    assert_eq!(count_lab2_before, 1);

    // Delete lab1's links
    delete_links_by_lab(&db, lab1.id.clone().unwrap()).await?;

    let count_lab1_after = count_links_by_lab(&db, lab1.id.clone().unwrap()).await?;
    let count_lab2_after = count_links_by_lab(&db, lab2.id.clone().unwrap()).await?;

    // lab1 should have 0 links
    assert_eq!(count_lab1_after, 0);

    // lab2 should still have 1 link
    assert_eq!(count_lab2_after, 1);

    teardown_db(&db).await?;
    Ok(())
}

#[tokio::test]
#[ignore]
async fn test_delete_links_by_node_success() -> Result<()> {
    let db = setup_db("link_delete_by_node_success").await?;

    let user = create_user(&db, "testuser".to_string(), vec![]).await?;
    let lab = create_lab(&db, "Test Lab", "lab-0001", &user).await?;
    let config = create_node_config(&db, NodeConfig::get_model(NodeModel::UbuntuLinux)).await?;

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

    // Create links: node1 connected to node2 and node3
    create_link(
        &db,
        1,
        BridgeKind::P2pBridge,
        node1.id.clone().unwrap(),
        node2.id.clone().unwrap(),
        "eth0".to_string(),
        "eth0".to_string(),
        "br-1".to_string(),
        "br-2".to_string(),
        "veth-1".to_string(),
        "veth-2".to_string(),
        lab.id.clone().unwrap(),
    )
    .await?;

    create_link(
        &db,
        2,
        BridgeKind::P2pVeth,
        node1.id.clone().unwrap(),
        node3.id.clone().unwrap(),
        "eth1".to_string(),
        "eth0".to_string(),
        "br-3".to_string(),
        "br-4".to_string(),
        "veth-3".to_string(),
        "veth-4".to_string(),
        lab.id.clone().unwrap(),
    )
    .await?;

    // node2 <-> node3
    create_link(
        &db,
        3,
        BridgeKind::P2pUdp,
        node2.id.clone().unwrap(),
        node3.id.clone().unwrap(),
        "eth1".to_string(),
        "eth1".to_string(),
        "br-5".to_string(),
        "br-6".to_string(),
        "veth-5".to_string(),
        "veth-6".to_string(),
        lab.id.clone().unwrap(),
    )
    .await?;

    let count_node1_before = count_links_by_node(&db, node1.id.clone().unwrap()).await?;
    assert_eq!(count_node1_before, 2);

    // Delete all links connected to node1
    delete_links_by_node(&db, node1.id.clone().unwrap()).await?;

    let count_node1_after = count_links_by_node(&db, node1.id.clone().unwrap()).await?;
    assert_eq!(count_node1_after, 0);

    // node2-node3 link should still exist
    let count_node2 = count_links_by_node(&db, node2.id.clone().unwrap()).await?;
    let count_node3 = count_links_by_node(&db, node3.id.clone().unwrap()).await?;
    assert_eq!(count_node2, 1);
    assert_eq!(count_node3, 1);

    teardown_db(&db).await?;
    Ok(())
}

#[tokio::test]
#[ignore]
async fn test_delete_links_by_node_only_affects_one_node() -> Result<()> {
    let db = setup_db("link_delete_by_node_isolation").await?;

    let user = create_user(&db, "testuser".to_string(), vec![]).await?;
    let lab = create_lab(&db, "Test Lab", "lab-0001", &user).await?;
    let config = create_node_config(&db, NodeConfig::get_model(NodeModel::UbuntuLinux)).await?;

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
    let node4 = create_node(
        &db,
        "node4",
        4,
        config.id.clone().unwrap(),
        lab.id.clone().unwrap(),
    )
    .await?;

    // Create a mesh of links
    // node1 <-> node2
    create_link(
        &db,
        1,
        BridgeKind::P2pBridge,
        node1.id.clone().unwrap(),
        node2.id.clone().unwrap(),
        "eth0".to_string(),
        "eth0".to_string(),
        "br-1".to_string(),
        "br-2".to_string(),
        "veth-1".to_string(),
        "veth-2".to_string(),
        lab.id.clone().unwrap(),
    )
    .await?;

    // node1 <-> node3
    create_link(
        &db,
        2,
        BridgeKind::P2pVeth,
        node1.id.clone().unwrap(),
        node3.id.clone().unwrap(),
        "eth1".to_string(),
        "eth0".to_string(),
        "br-3".to_string(),
        "br-4".to_string(),
        "veth-3".to_string(),
        "veth-4".to_string(),
        lab.id.clone().unwrap(),
    )
    .await?;

    // node2 <-> node4
    create_link(
        &db,
        3,
        BridgeKind::P2pUdp,
        node2.id.clone().unwrap(),
        node4.id.clone().unwrap(),
        "eth1".to_string(),
        "eth0".to_string(),
        "br-5".to_string(),
        "br-6".to_string(),
        "veth-5".to_string(),
        "veth-6".to_string(),
        lab.id.clone().unwrap(),
    )
    .await?;

    // node3 <-> node4
    create_link(
        &db,
        4,
        BridgeKind::P2pBridge,
        node3.id.clone().unwrap(),
        node4.id.clone().unwrap(),
        "eth1".to_string(),
        "eth1".to_string(),
        "br-7".to_string(),
        "br-8".to_string(),
        "veth-7".to_string(),
        "veth-8".to_string(),
        lab.id.clone().unwrap(),
    )
    .await?;

    let total_before = count_links(&db).await?;
    assert_eq!(total_before, 4);

    // Delete all links connected to node1
    delete_links_by_node(&db, node1.id.clone().unwrap()).await?;

    // node1 should have 0 links
    let count_node1 = count_links_by_node(&db, node1.id.clone().unwrap()).await?;
    assert_eq!(count_node1, 0);

    // node2 should have 1 link (to node4)
    let count_node2 = count_links_by_node(&db, node2.id.clone().unwrap()).await?;
    assert_eq!(count_node2, 1);

    // node3 should have 1 link (to node4)
    let count_node3 = count_links_by_node(&db, node3.id.clone().unwrap()).await?;
    assert_eq!(count_node3, 1);

    // node4 should have 2 links (to node2 and node3)
    let count_node4 = count_links_by_node(&db, node4.id.clone().unwrap()).await?;
    assert_eq!(count_node4, 2);

    // Total links should be 2 (node2-node4 and node3-node4)
    let total_after = count_links(&db).await?;
    assert_eq!(total_after, 2);

    teardown_db(&db).await?;
    Ok(())
}
