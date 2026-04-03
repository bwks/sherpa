use anyhow::Result;

use db::{
    count_links_by_lab, count_nodes_by_lab, create_bridge, create_lab, create_link,
    create_node_image, create_user, delete_lab_cascade, delete_user, get_lab, list_bridges,
    list_links_by_lab, list_nodes_by_lab,
};
use shared::data::{BridgeKind, NodeConfig, NodeModel};

use crate::{create_test_node_with_model, setup_db, teardown_db};

// ============================================================================
// Cascade deletes — lab
// ============================================================================

/// Deleting a lab cascades to its nodes, links, AND bridges.
#[tokio::test]
#[ignore]
async fn test_delete_lab_cascade_removes_nodes_links_bridges() -> Result<()> {
    let db = setup_db("test_cascade_full_topology").await?;

    let user = create_user(
        &db,
        "cascade_user".to_string(),
        "TestPass123!",
        false,
        vec![],
    )
    .await?;
    create_node_image(&db, NodeConfig::get_model(NodeModel::UbuntuLinux)).await?;

    let lab = create_lab(
        &db,
        "Cascade Lab",
        "casc0001",
        &user,
        "127.127.1.0/24",
        "172.31.1.0/24",
        "172.31.1.1",
        "172.31.1.2",
    )
    .await?;
    let lab_rid = lab.id.clone().expect("lab has id");

    // Create 2 nodes
    let node1 = create_test_node_with_model(&db, "n1", 1, NodeModel::UbuntuLinux, &lab).await?;
    let node2 = create_test_node_with_model(&db, "n2", 2, NodeModel::UbuntuLinux, &lab).await?;
    let n1_id = node1.id.clone().expect("n1 has id");
    let n2_id = node2.id.clone().expect("n2 has id");

    // Create a link between them
    create_link(
        &db,
        0,
        BridgeKind::default(),
        n1_id.clone(),
        n2_id.clone(),
        "eth1".to_string(),
        "eth1".to_string(),
        "br-0".to_string(),
        "br-0".to_string(),
        "veth-a".to_string(),
        "veth-b".to_string(),
        String::new(),
        String::new(),
        lab_rid.clone(),
    )
    .await?;

    // Create a bridge
    create_bridge(
        &db,
        0,
        "test-br".to_string(),
        "test-net".to_string(),
        lab_rid.clone(),
        vec![n1_id, n2_id],
    )
    .await?;

    // Verify everything exists
    assert_eq!(count_nodes_by_lab(&db, lab_rid.clone()).await?, 2);
    assert_eq!(count_links_by_lab(&db, lab_rid.clone()).await?, 1);
    let bridges = list_bridges(&db, &lab_rid).await?;
    assert_eq!(bridges.len(), 1);

    // Delete lab with cascade
    delete_lab_cascade(&db, &lab.lab_id).await?;

    // Verify lab is gone
    assert!(get_lab(&db, "casc0001").await.is_err());

    // Verify nodes are gone
    assert_eq!(count_nodes_by_lab(&db, lab_rid.clone()).await?, 0);

    // Verify links are gone
    assert_eq!(count_links_by_lab(&db, lab_rid.clone()).await?, 0);

    // Verify bridges are gone
    let bridges_after = list_bridges(&db, &lab_rid).await?;
    assert_eq!(bridges_after.len(), 0);

    teardown_db(&db).await?;
    Ok(())
}

// ============================================================================
// Cascade deletes — user to labs (transitive)
// ============================================================================

/// Deleting a user cascades to their labs, which transitively removes nodes.
#[tokio::test]
#[ignore]
async fn test_delete_user_cascades_to_labs_and_nodes() -> Result<()> {
    let db = setup_db("test_user_cascade_transitive").await?;

    let user = create_user(
        &db,
        "doomed_user".to_string(),
        "TestPass123!",
        false,
        vec![],
    )
    .await?;
    let user_id = user.id.clone().expect("user has id");
    create_node_image(&db, NodeConfig::get_model(NodeModel::UbuntuLinux)).await?;

    let lab = create_lab(
        &db,
        "Doomed Lab",
        "doom0001",
        &user,
        "127.127.1.0/24",
        "172.31.1.0/24",
        "172.31.1.1",
        "172.31.1.2",
    )
    .await?;
    let lab_rid = lab.id.clone().expect("lab has id");

    create_test_node_with_model(&db, "node1", 1, NodeModel::UbuntuLinux, &lab).await?;
    create_test_node_with_model(&db, "node2", 2, NodeModel::UbuntuLinux, &lab).await?;

    // Verify setup
    assert_eq!(count_nodes_by_lab(&db, lab_rid.clone()).await?, 2);

    // Delete the user — should cascade to lab → nodes
    delete_user(&db, user_id).await?;

    // Lab should be gone
    assert!(get_lab(&db, "doom0001").await.is_err());

    // Nodes should be gone
    assert_eq!(count_nodes_by_lab(&db, lab_rid).await?, 0);

    teardown_db(&db).await?;
    Ok(())
}

// ============================================================================
// Cross-table query isolation
// ============================================================================

/// Nodes from one lab don't appear when querying another lab.
#[tokio::test]
#[ignore]
async fn test_list_nodes_by_lab_isolation() -> Result<()> {
    let db = setup_db("test_node_lab_isolation").await?;

    let user = create_user(&db, "iso_user".to_string(), "TestPass123!", false, vec![]).await?;
    create_node_image(&db, NodeConfig::get_model(NodeModel::UbuntuLinux)).await?;

    let lab_a = create_lab(
        &db,
        "Lab A",
        "laba0001",
        &user,
        "127.127.1.0/24",
        "172.31.1.0/24",
        "172.31.1.1",
        "172.31.1.2",
    )
    .await?;
    let lab_b = create_lab(
        &db,
        "Lab B",
        "labb0001",
        &user,
        "127.127.2.0/24",
        "172.31.2.0/24",
        "172.31.2.1",
        "172.31.2.2",
    )
    .await?;

    create_test_node_with_model(&db, "a-node1", 1, NodeModel::UbuntuLinux, &lab_a).await?;
    create_test_node_with_model(&db, "a-node2", 2, NodeModel::UbuntuLinux, &lab_a).await?;
    create_test_node_with_model(&db, "b-node1", 1, NodeModel::UbuntuLinux, &lab_b).await?;

    let nodes_a = list_nodes_by_lab(&db, lab_a.id.clone().unwrap()).await?;
    let nodes_b = list_nodes_by_lab(&db, lab_b.id.clone().unwrap()).await?;

    assert_eq!(nodes_a.len(), 2);
    assert_eq!(nodes_b.len(), 1);

    let names_a: Vec<&str> = nodes_a.iter().map(|n| n.name.as_str()).collect();
    assert!(names_a.contains(&"a-node1"));
    assert!(names_a.contains(&"a-node2"));
    assert_eq!(nodes_b[0].name, "b-node1");

    teardown_db(&db).await?;
    Ok(())
}

/// Links from one lab don't appear when querying another lab.
#[tokio::test]
#[ignore]
async fn test_list_links_by_lab_isolation() -> Result<()> {
    let db = setup_db("test_link_lab_isolation").await?;

    let user = create_user(
        &db,
        "link_iso_user".to_string(),
        "TestPass123!",
        false,
        vec![],
    )
    .await?;
    create_node_image(&db, NodeConfig::get_model(NodeModel::UbuntuLinux)).await?;

    let lab_a = create_lab(
        &db,
        "Link Lab A",
        "llka0001",
        &user,
        "127.127.1.0/24",
        "172.31.1.0/24",
        "172.31.1.1",
        "172.31.1.2",
    )
    .await?;
    let lab_b = create_lab(
        &db,
        "Link Lab B",
        "llkb0001",
        &user,
        "127.127.2.0/24",
        "172.31.2.0/24",
        "172.31.2.1",
        "172.31.2.2",
    )
    .await?;

    let a1 = create_test_node_with_model(&db, "a1", 1, NodeModel::UbuntuLinux, &lab_a).await?;
    let a2 = create_test_node_with_model(&db, "a2", 2, NodeModel::UbuntuLinux, &lab_a).await?;
    create_link(
        &db,
        0,
        BridgeKind::default(),
        a1.id.clone().unwrap(),
        a2.id.clone().unwrap(),
        "eth1".to_string(),
        "eth1".to_string(),
        "br-0".to_string(),
        "br-0".to_string(),
        "veth-a".to_string(),
        "veth-b".to_string(),
        String::new(),
        String::new(),
        lab_a.id.clone().unwrap(),
    )
    .await?;

    let b1 = create_test_node_with_model(&db, "b1", 1, NodeModel::UbuntuLinux, &lab_b).await?;
    let b2 = create_test_node_with_model(&db, "b2", 2, NodeModel::UbuntuLinux, &lab_b).await?;
    create_link(
        &db,
        0,
        BridgeKind::default(),
        b1.id.clone().unwrap(),
        b2.id.clone().unwrap(),
        "eth1".to_string(),
        "eth1".to_string(),
        "br-0".to_string(),
        "br-0".to_string(),
        "veth-a".to_string(),
        "veth-b".to_string(),
        String::new(),
        String::new(),
        lab_b.id.clone().unwrap(),
    )
    .await?;

    let links_a = list_links_by_lab(&db, lab_a.id.clone().unwrap()).await?;
    let links_b = list_links_by_lab(&db, lab_b.id.clone().unwrap()).await?;

    assert_eq!(links_a.len(), 1);
    assert_eq!(links_b.len(), 1);

    teardown_db(&db).await?;
    Ok(())
}

// ============================================================================
// Labs by user isolation
// ============================================================================

/// Each user only sees their own labs.
#[tokio::test]
#[ignore]
async fn test_list_labs_by_user_isolation() -> Result<()> {
    let db = setup_db("test_labs_by_user_iso").await?;

    let user_a = create_user(&db, "alice".to_string(), "TestPass123!", false, vec![]).await?;
    let user_b = create_user(&db, "bob".to_string(), "TestPass123!", false, vec![]).await?;

    create_lab(
        &db,
        "Alice Lab 1",
        "alab0001",
        &user_a,
        "127.127.1.0/24",
        "172.31.1.0/24",
        "172.31.1.1",
        "172.31.1.2",
    )
    .await?;
    create_lab(
        &db,
        "Alice Lab 2",
        "alab0002",
        &user_a,
        "127.127.2.0/24",
        "172.31.2.0/24",
        "172.31.2.1",
        "172.31.2.2",
    )
    .await?;
    create_lab(
        &db,
        "Bob Lab 1",
        "blab0001",
        &user_b,
        "127.127.3.0/24",
        "172.31.3.0/24",
        "172.31.3.1",
        "172.31.3.2",
    )
    .await?;

    let alice_labs = db::list_labs_by_user(&db, user_a.id.clone().unwrap()).await?;
    let bob_labs = db::list_labs_by_user(&db, user_b.id.clone().unwrap()).await?;

    assert_eq!(alice_labs.len(), 2);
    assert_eq!(bob_labs.len(), 1);
    assert_eq!(bob_labs[0].name, "Bob Lab 1");

    teardown_db(&db).await?;
    Ok(())
}
