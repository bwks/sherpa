use anyhow::Result;
use db::{
    create_lab, create_link, create_node, create_node_image, create_user, get_link_by_id,
    update_link,
};
use shared::data::{BridgeKind, LinkImpairment, NodeConfig, NodeModel, RecordId};

use crate::helper::{ipv4_lab_networks, setup_db, teardown_db};

#[tokio::test]
#[ignore]
async fn test_update_link_success() -> Result<()> {
    let db = setup_db("link_update_success").await?;

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

    let mut link = create_link(
        &db,
        1,
        BridgeKind::P2p,
        node1.id.clone().unwrap(),
        node2.id.clone().unwrap(),
        "eth0".to_string(),
        "eth0".to_string(),
        String::new(),
        String::new(),
        lab.id.clone().unwrap(),
    )
    .await?;

    // Update mutable fields
    link.index = 10;
    link.kind = BridgeKind::SharedBridge;
    link.int_a = "eth1".to_string();
    link.int_b = "eth1".to_string();
    link.tap_a = "new-tap-a".to_string();
    link.tap_b = "new-tap-b".to_string();
    // Each direction keeps its own impairment.
    let a_to_b = LinkImpairment {
        delay_us: 20000,
        jitter_us: 1000,
        loss_percent: 2.5,
        reorder_percent: 10.0,
        corrupt_percent: 0.5,
    };
    let b_to_a = LinkImpairment {
        delay_us: 5000,
        ..LinkImpairment::default()
    };
    link.impairment_a_to_b = a_to_b;
    link.impairment_b_to_a = b_to_a;

    let updated = update_link(&db, link).await?;

    assert_eq!(updated.index, 10);
    assert_eq!(updated.kind, BridgeKind::SharedBridge);
    assert_eq!(updated.int_a, "eth1");
    assert_eq!(updated.int_b, "eth1");
    assert_eq!(updated.tap_a, "new-tap-a");
    assert_eq!(updated.tap_b, "new-tap-b");
    assert_eq!(updated.impairment_a_to_b, a_to_b);
    assert_eq!(updated.impairment_b_to_a, b_to_a);
    let stored = get_link_by_id(&db, updated.id.clone().unwrap()).await?;
    assert_eq!(stored.impairment_a_to_b, a_to_b);
    assert_eq!(stored.impairment_b_to_a, b_to_a);

    teardown_db(&db).await?;
    Ok(())
}

#[tokio::test]
#[ignore]
async fn test_update_link_without_id_fails() -> Result<()> {
    let db = setup_db("link_update_without_id").await?;

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

    let mut link = create_link(
        &db,
        1,
        BridgeKind::P2p,
        node1.id.clone().unwrap(),
        node2.id.clone().unwrap(),
        "eth0".to_string(),
        "eth0".to_string(),
        String::new(),
        String::new(),
        lab.id.clone().unwrap(),
    )
    .await?;

    // Remove the ID
    link.id = None;
    link.index = 10;

    let result = update_link(&db, link).await;

    assert!(result.is_err());
    let err_msg = result.unwrap_err().to_string();
    assert!(err_msg.contains("ID") || err_msg.contains("id") || err_msg.contains("required"));

    teardown_db(&db).await?;
    Ok(())
}

#[tokio::test]
#[ignore]
async fn test_update_link_change_lab_fails() -> Result<()> {
    let db = setup_db("link_update_change_lab").await?;

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
    )
    .await?;
    let config = create_node_image(&db, NodeConfig::get_model(NodeModel::UbuntuLinux)).await?;

    let node1 = create_node(
        &db,
        "node1",
        1,
        config.id.clone().unwrap(),
        lab1.id.clone().unwrap(),
    )
    .await?;
    let node2 = create_node(
        &db,
        "node2",
        2,
        config.id.clone().unwrap(),
        lab1.id.clone().unwrap(),
    )
    .await?;

    let mut link = create_link(
        &db,
        1,
        BridgeKind::P2p,
        node1.id.clone().unwrap(),
        node2.id.clone().unwrap(),
        "eth0".to_string(),
        "eth0".to_string(),
        String::new(),
        String::new(),
        lab1.id.clone().unwrap(),
    )
    .await?;

    // Try to change the lab (immutable field)
    link.lab = lab2.id.clone().unwrap();

    let result = update_link(&db, link).await;

    assert!(result.is_err());
    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.contains("immutable")
            || err_msg.contains("cannot change")
            || err_msg.contains("lab")
    );

    teardown_db(&db).await?;
    Ok(())
}

#[tokio::test]
#[ignore]
async fn test_update_link_change_node_a_fails() -> Result<()> {
    let db = setup_db("link_update_change_node_a").await?;

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

    let mut link = create_link(
        &db,
        1,
        BridgeKind::P2p,
        node1.id.clone().unwrap(),
        node2.id.clone().unwrap(),
        "eth0".to_string(),
        "eth0".to_string(),
        String::new(),
        String::new(),
        lab.id.clone().unwrap(),
    )
    .await?;

    // Try to change node_a (immutable field)
    link.node_a = node3.id.clone().unwrap();

    let result = update_link(&db, link).await;

    assert!(result.is_err());
    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.contains("immutable")
            || err_msg.contains("cannot change")
            || err_msg.contains("node_a")
            || err_msg.contains("endpoint")
    );

    teardown_db(&db).await?;
    Ok(())
}

#[tokio::test]
#[ignore]
async fn test_update_link_change_node_b_fails() -> Result<()> {
    let db = setup_db("link_update_change_node_b").await?;

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

    let mut link = create_link(
        &db,
        1,
        BridgeKind::P2p,
        node1.id.clone().unwrap(),
        node2.id.clone().unwrap(),
        "eth0".to_string(),
        "eth0".to_string(),
        String::new(),
        String::new(),
        lab.id.clone().unwrap(),
    )
    .await?;

    // Try to change node_b (immutable field)
    link.node_b = node3.id.clone().unwrap();

    let result = update_link(&db, link).await;

    assert!(result.is_err());
    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.contains("immutable")
            || err_msg.contains("cannot change")
            || err_msg.contains("node_b")
            || err_msg.contains("endpoint")
    );

    teardown_db(&db).await?;
    Ok(())
}

#[tokio::test]
#[ignore]
async fn test_update_link_duplicate_peers_fails() -> Result<()> {
    let db = setup_db("link_update_duplicate_peers").await?;

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

    // Create first link: node1:eth0 <-> node2:eth0
    create_link(
        &db,
        1,
        BridgeKind::P2p,
        node1.id.clone().unwrap(),
        node2.id.clone().unwrap(),
        "eth0".to_string(),
        "eth0".to_string(),
        String::new(),
        String::new(),
        lab.id.clone().unwrap(),
    )
    .await?;

    // Create second link: node1:eth1 <-> node2:eth1
    let mut link2 = create_link(
        &db,
        2,
        BridgeKind::P2p,
        node1.id.clone().unwrap(),
        node2.id.clone().unwrap(),
        "eth1".to_string(),
        "eth1".to_string(),
        String::new(),
        String::new(),
        lab.id.clone().unwrap(),
    )
    .await?;

    // Try to update link2 to use the same interfaces as link1 (should fail unique constraint)
    link2.int_a = "eth0".to_string();
    link2.int_b = "eth0".to_string();

    let result = update_link(&db, link2).await;

    // The update should fail due to unique constraint violation
    // However, SurrealDB might not return a specific error message
    // Let's check if it actually succeeds (which would be wrong)
    if result.is_ok() {
        panic!("Update should have failed due to unique constraint violation, but it succeeded!");
    }

    // If it fails, that's expected - the error message format may vary
    eprintln!("Update correctly failed with: {}", result.unwrap_err());

    teardown_db(&db).await?;
    Ok(())
}

#[tokio::test]
#[ignore]
async fn test_update_link_preserves_id() -> Result<()> {
    let db = setup_db("link_update_preserves_id").await?;

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

    let link = create_link(
        &db,
        1,
        BridgeKind::P2p,
        node1.id.clone().unwrap(),
        node2.id.clone().unwrap(),
        "eth0".to_string(),
        "eth0".to_string(),
        String::new(),
        String::new(),
        lab.id.clone().unwrap(),
    )
    .await?;

    let original_id = link.id.clone();

    let mut updated_link = link.clone();
    updated_link.index = 10;
    updated_link.int_a = "eth1".to_string();

    let result = update_link(&db, updated_link).await?;

    assert_eq!(result.id, original_id);

    teardown_db(&db).await?;
    Ok(())
}

#[tokio::test]
#[ignore]
async fn test_update_link_can_change_interfaces() -> Result<()> {
    let db = setup_db("link_update_change_interfaces").await?;

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

    let mut link = create_link(
        &db,
        1,
        BridgeKind::P2p,
        node1.id.clone().unwrap(),
        node2.id.clone().unwrap(),
        "eth0".to_string(),
        "eth0".to_string(),
        String::new(),
        String::new(),
        lab.id.clone().unwrap(),
    )
    .await?;

    // Change interface names
    link.int_a = "eth10".to_string();
    link.int_b = "eth20".to_string();

    let updated = update_link(&db, link.clone()).await?;

    assert_eq!(updated.int_a, "eth10");
    assert_eq!(updated.int_b, "eth20");

    // Verify the change persisted
    let retrieved = get_link_by_id(&db, link.id.unwrap()).await?;
    assert_eq!(retrieved.int_a, "eth10");
    assert_eq!(retrieved.int_b, "eth20");

    teardown_db(&db).await?;
    Ok(())
}

#[tokio::test]
#[ignore]
async fn test_update_link_nonexistent_fails() -> Result<()> {
    let db = setup_db("link_update_nonexistent").await?;

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

    let mut link = create_link(
        &db,
        1,
        BridgeKind::P2p,
        node1.id.clone().unwrap(),
        node2.id.clone().unwrap(),
        "eth0".to_string(),
        "eth0".to_string(),
        String::new(),
        String::new(),
        lab.id.clone().unwrap(),
    )
    .await?;

    // Create a fake ID
    link.id = Some(RecordId::new("link", "nonexistent"));
    link.index = 999;

    let result = update_link(&db, link).await;

    assert!(result.is_err());
    let err_msg = result.unwrap_err().to_string();
    assert!(err_msg.contains("not found") || err_msg.contains("does not exist"));

    teardown_db(&db).await?;
    Ok(())
}
