use anyhow::Result;
use db::{
    count_nodes, count_nodes_by_lab, create_lab, create_link, create_node, create_node_image,
    create_user, delete_node, delete_node_by_id, delete_node_cascade, delete_node_safe,
    delete_nodes_by_lab, get_node,
};
use shared::data::{BridgeKind, NodeConfig, NodeModel, RecordId};

use crate::helper::{setup_db, teardown_db};

#[tokio::test]
#[ignore]
async fn test_delete_node_success() -> Result<()> {
    let db = setup_db("test_delete_node").await?;

    let user = create_user(&db, "alice".to_string(), "TestPass123!", false, vec![]).await?;
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

    let node = create_node(&db, "node1", 1, config.id.unwrap(), lab.id.clone().unwrap()).await?;

    let node_id = node.id.clone().unwrap();

    // Delete node
    delete_node(&db, node_id.clone()).await?;

    // Verify node is gone
    let result = get_node(&db, node_id).await;
    assert!(result.is_err(), "Node should not exist after deletion");

    teardown_db(&db).await?;
    Ok(())
}

#[tokio::test]
#[ignore]
async fn test_delete_node_by_id_success() -> Result<()> {
    let db = setup_db("test_delete_node_by_id").await?;

    let user = create_user(&db, "bob".to_string(), "TestPass123!", false, vec![]).await?;
    let lab = create_lab(
        &db,
        "Lab",
        "lab-0002",
        &user,
        "127.127.1.0/24",
        "172.31.1.0/24",
    )
    .await?;
    let config = create_node_image(&db, NodeConfig::get_model(NodeModel::CiscoIosv)).await?;

    let node = create_node(
        &db,
        "router1",
        1,
        config.id.unwrap(),
        lab.id.clone().unwrap(),
    )
    .await?;

    let node_id = node.id.clone().unwrap();

    // Delete node by ID
    delete_node_by_id(&db, node_id.clone()).await?;

    // Verify node is gone
    let result = get_node(&db, node_id).await;
    assert!(result.is_err(), "Node should not exist after deletion");

    teardown_db(&db).await?;
    Ok(())
}

#[tokio::test]
#[ignore]
async fn test_delete_node_nonexistent_fails() -> Result<()> {
    let db = setup_db("test_delete_node_nonexistent").await?;

    let fake_id = RecordId::new("node", "nonexistent");
    let result = delete_node(&db, fake_id).await;

    assert!(result.is_err(), "Should fail when node doesn't exist");
    let error_msg = result.unwrap_err().to_string();
    assert!(
        error_msg.contains("not found"),
        "Error should mention node not found, got: {}",
        error_msg
    );

    teardown_db(&db).await?;
    Ok(())
}

#[tokio::test]
#[ignore]
async fn test_delete_node_decrements_count() -> Result<()> {
    let db = setup_db("test_delete_node_count").await?;

    let user = create_user(&db, "charlie".to_string(), "TestPass123!", false, vec![]).await?;
    let lab = create_lab(
        &db,
        "Lab",
        "lab-0003",
        &user,
        "127.127.1.0/24",
        "172.31.1.0/24",
    )
    .await?;
    let config = create_node_image(&db, NodeConfig::get_model(NodeModel::UbuntuLinux)).await?;

    // Create nodes
    let node1 = create_node(
        &db,
        "node1",
        1,
        config.id.clone().unwrap(),
        lab.id.clone().unwrap(),
    )
    .await?;
    create_node(
        &db,
        "node2",
        2,
        config.id.clone().unwrap(),
        lab.id.clone().unwrap(),
    )
    .await?;

    let count_before = count_nodes(&db).await?;

    // Delete one node
    delete_node(&db, node1.id.unwrap()).await?;

    let count_after = count_nodes(&db).await?;

    assert_eq!(count_after, count_before - 1);

    teardown_db(&db).await?;
    Ok(())
}

#[tokio::test]
#[ignore]
async fn test_delete_nodes_by_lab_success() -> Result<()> {
    let db = setup_db("test_delete_nodes_by_lab").await?;

    let user = create_user(&db, "diana".to_string(), "TestPass123!", false, vec![]).await?;
    let lab = create_lab(
        &db,
        "Lab",
        "lab-0004",
        &user,
        "127.127.1.0/24",
        "172.31.1.0/24",
    )
    .await?;
    let config = create_node_image(&db, NodeConfig::get_model(NodeModel::CiscoIosv)).await?;

    // Create multiple nodes in lab
    create_node(
        &db,
        "node1",
        1,
        config.id.clone().unwrap(),
        lab.id.clone().unwrap(),
    )
    .await?;
    create_node(
        &db,
        "node2",
        2,
        config.id.clone().unwrap(),
        lab.id.clone().unwrap(),
    )
    .await?;
    create_node(
        &db,
        "node3",
        3,
        config.id.clone().unwrap(),
        lab.id.clone().unwrap(),
    )
    .await?;

    let count_before = count_nodes_by_lab(&db, lab.id.clone().unwrap()).await?;
    assert_eq!(count_before, 3);

    // Delete all nodes in lab
    delete_nodes_by_lab(&db, lab.id.clone().unwrap()).await?;

    let count_after = count_nodes_by_lab(&db, lab.id.unwrap()).await?;
    assert_eq!(count_after, 0, "Lab should have no nodes after deletion");

    teardown_db(&db).await?;
    Ok(())
}

#[tokio::test]
#[ignore]
async fn test_delete_nodes_by_lab_only_affects_one_lab() -> Result<()> {
    let db = setup_db("test_delete_nodes_lab_isolation").await?;

    let user = create_user(&db, "emily".to_string(), "TestPass123!", false, vec![]).await?;
    let lab1 = create_lab(
        &db,
        "Lab 1",
        "lab-0005",
        &user,
        "127.127.1.0/24",
        "172.31.1.0/24",
    )
    .await?;
    let lab2 = create_lab(
        &db,
        "Lab 2",
        "lab-0006",
        &user,
        "127.127.2.0/24",
        "172.31.2.0/24",
    )
    .await?;
    let config = create_node_image(&db, NodeConfig::get_model(NodeModel::UbuntuLinux)).await?;

    // Create nodes in both labs
    create_node(
        &db,
        "node1",
        1,
        config.id.clone().unwrap(),
        lab1.id.clone().unwrap(),
    )
    .await?;
    create_node(
        &db,
        "node2",
        2,
        config.id.clone().unwrap(),
        lab1.id.clone().unwrap(),
    )
    .await?;
    create_node(
        &db,
        "node3",
        1,
        config.id.clone().unwrap(),
        lab2.id.clone().unwrap(),
    )
    .await?;

    // Delete nodes in lab1 only
    delete_nodes_by_lab(&db, lab1.id.clone().unwrap()).await?;

    let lab1_count = count_nodes_by_lab(&db, lab1.id.unwrap()).await?;
    let lab2_count = count_nodes_by_lab(&db, lab2.id.unwrap()).await?;

    assert_eq!(lab1_count, 0, "Lab1 should have no nodes");
    assert_eq!(lab2_count, 1, "Lab2 should still have its node");

    teardown_db(&db).await?;
    Ok(())
}

#[tokio::test]
#[ignore]
async fn test_delete_node_safe_with_no_links_succeeds() -> Result<()> {
    let db = setup_db("test_delete_node_safe_ok").await?;

    let user = create_user(&db, "frank".to_string(), "TestPass123!", false, vec![]).await?;
    let lab = create_lab(
        &db,
        "Lab",
        "lab-0007",
        &user,
        "127.127.1.0/24",
        "172.31.1.0/24",
    )
    .await?;
    let config = create_node_image(&db, NodeConfig::get_model(NodeModel::CiscoIosv)).await?;

    let node = create_node(
        &db,
        "router1",
        1,
        config.id.unwrap(),
        lab.id.clone().unwrap(),
    )
    .await?;

    let node_id = node.id.clone().unwrap();

    // Delete node safely (no links)
    delete_node_safe(&db, node_id.clone()).await?;

    // Verify node is gone
    let result = get_node(&db, node_id).await;
    assert!(result.is_err(), "Node should not exist after deletion");

    teardown_db(&db).await?;
    Ok(())
}

#[tokio::test]
#[ignore]
async fn test_delete_node_safe_with_links_fails() -> Result<()> {
    let db = setup_db("test_delete_node_safe_fail").await?;

    let user = create_user(&db, "grace".to_string(), "TestPass123!", false, vec![]).await?;
    let lab = create_lab(
        &db,
        "Lab",
        "lab-0008",
        &user,
        "127.127.1.0/24",
        "172.31.1.0/24",
    )
    .await?;
    let config = create_node_image(&db, NodeConfig::get_model(NodeModel::UbuntuLinux)).await?;

    // Create two nodes
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

    // Create a link between them
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

    // Try to delete node1 safely (has a link)
    let result = delete_node_safe(&db, node1.id.unwrap()).await;

    assert!(
        result.is_err(),
        "Should fail when node has links (safe delete)"
    );
    let error_msg = result.unwrap_err().to_string();
    assert!(
        error_msg.contains("link") && error_msg.contains("Cannot delete"),
        "Error should mention links blocking deletion, got: {}",
        error_msg
    );

    teardown_db(&db).await?;
    Ok(())
}

#[tokio::test]
#[ignore]
async fn test_delete_node_cascade_with_links_succeeds() -> Result<()> {
    let db = setup_db("test_delete_node_cascade").await?;

    let user = create_user(&db, "hannah".to_string(), "TestPass123!", false, vec![]).await?;
    let lab = create_lab(
        &db,
        "Lab",
        "lab-0009",
        &user,
        "127.127.1.0/24",
        "172.31.1.0/24",
    )
    .await?;
    let config = create_node_image(&db, NodeConfig::get_model(NodeModel::CiscoIosv)).await?;

    // Create two nodes
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

    // Create a link between them
    create_link(
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

    let node1_id = node1.id.clone().unwrap();

    // Delete node1 with cascade (should delete link too)
    delete_node_cascade(&db, node1_id.clone()).await?;

    // Verify node is gone
    let result = get_node(&db, node1_id).await;
    assert!(
        result.is_err(),
        "Node should not exist after cascade delete"
    );

    teardown_db(&db).await?;
    Ok(())
}

#[tokio::test]
#[ignore]
async fn test_delete_node_cascade_deletes_multiple_links() -> Result<()> {
    let db = setup_db("test_delete_node_cascade_multi").await?;

    let user = create_user(&db, "ian".to_string(), "TestPass123!", false, vec![]).await?;
    let lab = create_lab(
        &db,
        "Lab",
        "lab-0010",
        &user,
        "127.127.1.0/24",
        "172.31.1.0/24",
    )
    .await?;
    let config = create_node_image(&db, NodeConfig::get_model(NodeModel::UbuntuLinux)).await?;

    // Create three nodes
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

    // Create links: node1 <-> node2 and node1 <-> node3
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
        BridgeKind::P2pBridge,
        node1.id.clone().unwrap(),
        node3.id.clone().unwrap(),
        "eth1".to_string(),
        "eth0".to_string(),
        "br2".to_string(),
        "br3".to_string(),
        "veth2".to_string(),
        "veth3".to_string(),
        lab.id.clone().unwrap(),
    )
    .await?;

    let node1_id = node1.id.clone().unwrap();

    // Delete node1 with cascade (should delete both links)
    delete_node_cascade(&db, node1_id.clone()).await?;

    // Verify node is gone
    let result = get_node(&db, node1_id).await;
    assert!(
        result.is_err(),
        "Node should not exist after cascade delete"
    );

    // Verify other nodes still exist
    let node2_result = get_node(&db, node2.id.unwrap()).await;
    let node3_result = get_node(&db, node3.id.unwrap()).await;
    assert!(node2_result.is_ok(), "Node2 should still exist");
    assert!(node3_result.is_ok(), "Node3 should still exist");

    teardown_db(&db).await?;
    Ok(())
}
