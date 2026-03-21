use anyhow::Result;

use crate::{setup_db, teardown_db};
use db::{apply_schema, count_users, create_user};

/// Schema application is idempotent — applying twice does not fail or corrupt data.
/// setup_db already applies schema once. We apply it again and verify no error.
#[tokio::test]
#[ignore]
async fn test_schema_idempotent() -> Result<()> {
    let db = setup_db("test_schema_idempotent").await?;

    // Schema was already applied in setup_db. Apply again.
    apply_schema(&db).await?;

    // Verify the database still works — create a user after double-apply
    create_user(
        &db,
        "idempotent_user".to_string(),
        "TestPass123!",
        false,
        vec![],
    )
    .await?;
    let count = count_users(&db).await?;
    assert!(count >= 1);

    teardown_db(&db).await?;
    Ok(())
}

/// Schema creates all 6 tables — verify we can insert into each one.
#[tokio::test]
#[ignore]
async fn test_schema_creates_all_tables() -> Result<()> {
    use db::{create_bridge, create_lab, create_node_image};
    use shared::data::{BridgeKind, NodeConfig, NodeModel};

    let db = setup_db("test_schema_all_tables").await?;

    // user
    let user = create_user(
        &db,
        "schema_user".to_string(),
        "TestPass123!",
        false,
        vec![],
    )
    .await?;

    // node_image
    create_node_image(&db, NodeConfig::get_model(NodeModel::UbuntuLinux)).await?;

    // lab
    let lab = create_lab(
        &db,
        "Schema Lab",
        "schm0001",
        &user,
        "127.127.1.0/24",
        "172.31.1.0/24",
        "172.31.1.1",
        "172.31.1.2",
    )
    .await?;

    // node (via helper)
    let node =
        crate::create_test_node_with_model(&db, "node1", 1, NodeModel::UbuntuLinux, &lab).await?;

    // link
    let node2 =
        crate::create_test_node_with_model(&db, "node2", 2, NodeModel::UbuntuLinux, &lab).await?;
    let node_a_id = node.id.clone().expect("node has id");
    let node_b_id = node2.id.clone().expect("node2 has id");
    let lab_id = lab.id.clone().expect("lab has id");
    db::create_link(
        &db,
        0,
        BridgeKind::default(),
        node_a_id.clone(),
        node_b_id.clone(),
        "eth1".to_string(),
        "eth1".to_string(),
        "br-0".to_string(),
        "br-0".to_string(),
        "veth-a".to_string(),
        "veth-b".to_string(),
        lab_id.clone(),
    )
    .await?;

    // bridge
    create_bridge(
        &db,
        0,
        "test-bridge".to_string(),
        "test-net".to_string(),
        lab_id,
        vec![node_a_id, node_b_id],
    )
    .await?;

    teardown_db(&db).await?;
    Ok(())
}
