use anyhow::Result;
use db::{
    count_links, count_links_by_lab, count_links_by_node, create_lab, create_link, create_node,
    create_node_config, create_user, get_link_by_id, get_link_by_peers, list_links,
    list_links_by_lab, list_links_by_node,
};
use shared::data::{BridgeKind, NodeConfig, NodeModel};

use crate::helper::{setup_db, teardown_db};

#[tokio::test]
#[ignore]
async fn test_get_link_by_id_success() -> Result<()> {
    let db = setup_db("link_read_get_by_id_success").await?;

    let user = create_user(&db, "testuser".to_string(), "TestPass123!", false, vec![]).await?;
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
        "br-node1-eth0".to_string(),
        "br-node2-eth0".to_string(),
        "veth-node1".to_string(),
        "veth-node2".to_string(),
        lab.id.clone().unwrap(),
    )
    .await?;

    let retrieved = get_link_by_id(&db, link.id.clone().unwrap()).await?;

    assert_eq!(retrieved.id, link.id);
    assert_eq!(retrieved.index, 1);
    assert_eq!(retrieved.node_a, node1.id.unwrap());
    assert_eq!(retrieved.node_b, node2.id.unwrap());

    teardown_db(&db).await?;
    Ok(())
}

#[tokio::test]
#[ignore]
async fn test_get_link_not_found() -> Result<()> {
    let db = setup_db("link_read_get_not_found").await?;

    let user = create_user(&db, "testuser".to_string(), "TestPass123!", false, vec![]).await?;
    let _lab = create_lab(&db, "Test Lab", "lab-0001", &user).await?;

    let fake_id = ("link", "nonexistent").into();

    let result = get_link_by_id(&db, fake_id).await;

    assert!(result.is_err());
    let err_msg = result.unwrap_err().to_string();
    assert!(err_msg.contains("not found") || err_msg.contains("No link found"));

    teardown_db(&db).await?;
    Ok(())
}

#[tokio::test]
#[ignore]
async fn test_get_link_by_peers_success() -> Result<()> {
    let db = setup_db("link_read_get_by_peers_success").await?;

    let user = create_user(&db, "testuser".to_string(), "TestPass123!", false, vec![]).await?;
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
        "eth1".to_string(),
        "br-node1-eth0".to_string(),
        "br-node2-eth1".to_string(),
        "veth-node1".to_string(),
        "veth-node2".to_string(),
        lab.id.clone().unwrap(),
    )
    .await?;

    let retrieved = get_link_by_peers(
        &db,
        node1.id.clone().unwrap(),
        node2.id.clone().unwrap(),
        "eth0",
        "eth1",
    )
    .await?;

    assert_eq!(retrieved.id, link.id);
    assert_eq!(retrieved.int_a, "eth0");
    assert_eq!(retrieved.int_b, "eth1");

    teardown_db(&db).await?;
    Ok(())
}

#[tokio::test]
#[ignore]
async fn test_get_link_by_peers_not_found() -> Result<()> {
    let db = setup_db("link_read_get_by_peers_not_found").await?;

    let user = create_user(&db, "testuser".to_string(), "TestPass123!", false, vec![]).await?;
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

    let result = get_link_by_peers(
        &db,
        node1.id.clone().unwrap(),
        node2.id.clone().unwrap(),
        "eth0",
        "eth1",
    )
    .await;

    assert!(result.is_err());
    let err_msg = result.unwrap_err().to_string();
    assert!(err_msg.contains("not found") || err_msg.contains("No link found"));

    teardown_db(&db).await?;
    Ok(())
}

#[tokio::test]
#[ignore]
async fn test_list_links_empty() -> Result<()> {
    let db = setup_db("link_read_list_empty").await?;

    let links = list_links(&db).await?;

    assert_eq!(links.len(), 0);

    teardown_db(&db).await?;
    Ok(())
}

#[tokio::test]
#[ignore]
async fn test_list_links_returns_all() -> Result<()> {
    let db = setup_db("link_read_list_returns_all").await?;

    let user = create_user(&db, "testuser".to_string(), "TestPass123!", false, vec![]).await?;
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
        "eth1".to_string(),
        "eth1".to_string(),
        "br-3".to_string(),
        "br-4".to_string(),
        "veth-3".to_string(),
        "veth-4".to_string(),
        lab.id.clone().unwrap(),
    )
    .await?;

    let links = list_links(&db).await?;

    assert_eq!(links.len(), 2);

    teardown_db(&db).await?;
    Ok(())
}

#[tokio::test]
#[ignore]
async fn test_list_links_by_lab_empty() -> Result<()> {
    let db = setup_db("link_read_list_by_lab_empty").await?;

    let user = create_user(&db, "testuser".to_string(), "TestPass123!", false, vec![]).await?;
    let lab = create_lab(&db, "Test Lab", "lab-0001", &user).await?;

    let links = list_links_by_lab(&db, lab.id.clone().unwrap()).await?;

    assert_eq!(links.len(), 0);

    teardown_db(&db).await?;
    Ok(())
}

#[tokio::test]
#[ignore]
async fn test_list_links_by_lab_filters_correctly() -> Result<()> {
    let db = setup_db("link_read_list_by_lab_filters").await?;

    let user = create_user(&db, "testuser".to_string(), "TestPass123!", false, vec![]).await?;
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

    // Create link in lab1
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

    // Create links in lab2
    create_link(
        &db,
        1,
        BridgeKind::P2pVeth,
        node1_lab2.id.clone().unwrap(),
        node2_lab2.id.clone().unwrap(),
        "eth0".to_string(),
        "eth0".to_string(),
        "br-3".to_string(),
        "br-4".to_string(),
        "veth-3".to_string(),
        "veth-4".to_string(),
        lab2.id.clone().unwrap(),
    )
    .await?;

    create_link(
        &db,
        2,
        BridgeKind::P2pUdp,
        node1_lab2.id.clone().unwrap(),
        node2_lab2.id.clone().unwrap(),
        "eth1".to_string(),
        "eth1".to_string(),
        "br-5".to_string(),
        "br-6".to_string(),
        "veth-5".to_string(),
        "veth-6".to_string(),
        lab2.id.clone().unwrap(),
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

    teardown_db(&db).await?;
    Ok(())
}

#[tokio::test]
#[ignore]
async fn test_list_links_by_node_returns_all_connections() -> Result<()> {
    let db = setup_db("link_read_list_by_node").await?;

    let user = create_user(&db, "testuser".to_string(), "TestPass123!", false, vec![]).await?;
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

    let links_node1 = list_links_by_node(&db, node1.id.clone().unwrap()).await?;
    let links_node2 = list_links_by_node(&db, node2.id.clone().unwrap()).await?;
    let links_node3 = list_links_by_node(&db, node3.id.clone().unwrap()).await?;

    // node1 should have 2 links (to node2 and node3)
    assert_eq!(links_node1.len(), 2);

    // node2 should have 2 links (to node1 and node3)
    assert_eq!(links_node2.len(), 2);

    // node3 should have 2 links (to node1 and node2)
    assert_eq!(links_node3.len(), 2);

    teardown_db(&db).await?;
    Ok(())
}

#[tokio::test]
#[ignore]
async fn test_count_links_zero() -> Result<()> {
    let db = setup_db("link_read_count_zero").await?;

    let count = count_links(&db).await?;

    assert_eq!(count, 0);

    teardown_db(&db).await?;
    Ok(())
}

#[tokio::test]
#[ignore]
async fn test_count_links_accurate() -> Result<()> {
    let db = setup_db("link_read_count_accurate").await?;

    let user = create_user(&db, "testuser".to_string(), "TestPass123!", false, vec![]).await?;
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

    let count_before = count_links(&db).await?;
    assert_eq!(count_before, 0);

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

    let count_after = count_links(&db).await?;
    assert_eq!(count_after, 1);

    teardown_db(&db).await?;
    Ok(())
}

#[tokio::test]
#[ignore]
async fn test_count_links_by_lab_accurate() -> Result<()> {
    let db = setup_db("link_read_count_by_lab").await?;

    let user = create_user(&db, "testuser".to_string(), "TestPass123!", false, vec![]).await?;
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

    // Create 1 link in lab1
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

    // Create 3 links in lab2
    create_link(
        &db,
        1,
        BridgeKind::P2pVeth,
        node1_lab2.id.clone().unwrap(),
        node2_lab2.id.clone().unwrap(),
        "eth0".to_string(),
        "eth0".to_string(),
        "br-3".to_string(),
        "br-4".to_string(),
        "veth-3".to_string(),
        "veth-4".to_string(),
        lab2.id.clone().unwrap(),
    )
    .await?;

    create_link(
        &db,
        2,
        BridgeKind::P2pUdp,
        node1_lab2.id.clone().unwrap(),
        node2_lab2.id.clone().unwrap(),
        "eth1".to_string(),
        "eth1".to_string(),
        "br-5".to_string(),
        "br-6".to_string(),
        "veth-5".to_string(),
        "veth-6".to_string(),
        lab2.id.clone().unwrap(),
    )
    .await?;

    create_link(
        &db,
        3,
        BridgeKind::P2pBridge,
        node1_lab2.id.clone().unwrap(),
        node2_lab2.id.clone().unwrap(),
        "eth2".to_string(),
        "eth2".to_string(),
        "br-7".to_string(),
        "br-8".to_string(),
        "veth-7".to_string(),
        "veth-8".to_string(),
        lab2.id.clone().unwrap(),
    )
    .await?;

    let count_lab1 = count_links_by_lab(&db, lab1.id.clone().unwrap()).await?;
    let count_lab2 = count_links_by_lab(&db, lab2.id.clone().unwrap()).await?;

    assert_eq!(count_lab1, 1);
    assert_eq!(count_lab2, 3);

    teardown_db(&db).await?;
    Ok(())
}

#[tokio::test]
#[ignore]
async fn test_count_links_by_node_accurate() -> Result<()> {
    let db = setup_db("link_read_count_by_node").await?;

    let user = create_user(&db, "testuser".to_string(), "TestPass123!", false, vec![]).await?;
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

    let count_node1 = count_links_by_node(&db, node1.id.clone().unwrap()).await?;
    let count_node2 = count_links_by_node(&db, node2.id.clone().unwrap()).await?;
    let count_node3 = count_links_by_node(&db, node3.id.clone().unwrap()).await?;

    assert_eq!(count_node1, 2);
    assert_eq!(count_node2, 2);
    assert_eq!(count_node3, 2);

    teardown_db(&db).await?;
    Ok(())
}
