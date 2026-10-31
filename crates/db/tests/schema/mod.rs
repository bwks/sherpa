use anyhow::Result;

use crate::{create_test_node_with_model, ipv4_lab_networks, setup_db, teardown_db};
use db::{
    apply_schema, count_nodes_by_lab, count_users, create_bridge, create_lab, create_link,
    create_node_image, create_user, delete_lab_nodes, list_bridges,
};
use shared::data::{BridgeKind, BridgeMember, NodeConfig, NodeModel};

/// The bridge `nodes` field as it was defined before `members` replaced it.
const OLD_BRIDGE_SCHEMA: &str = "DEFINE FIELD OVERWRITE nodes ON TABLE bridge \
    TYPE array<record<node>> REFERENCE ON DELETE UNSET;";
/// A bridge record written under the old schema, listing every node of lab `migr0001`.
const OLD_BRIDGE_RECORD: &str = "LET $lab = (SELECT VALUE id FROM lab WHERE lab_id = 'migr0001')[0]; \
    CREATE bridge SET index = 0, bridge_name = 'brs0-migr0001', \
    network_name = 'sherpa-bridge0-old-migr0001', lab = $lab, members = [], \
    nodes = (SELECT VALUE id FROM node WHERE lab = $lab);";

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
        &ipv4_lab_networks(
            "127.127.1.0/24",
            "172.31.1.0/24",
            "172.31.1.1",
            "172.31.1.2",
        ),
    )
    .await?;

    // node (via helper)
    let node = create_test_node_with_model(&db, "node1", 1, NodeModel::UbuntuLinux, &lab).await?;

    // link
    let node2 = create_test_node_with_model(&db, "node2", 2, NodeModel::UbuntuLinux, &lab).await?;
    let node_a_id = node.id.clone().expect("node has id");
    let node_b_id = node2.id.clone().expect("node2 has id");
    let lab_id = lab.id.clone().expect("lab has id");
    create_link(
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
        String::new(),
        String::new(),
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
        vec![
            BridgeMember {
                node: node_a_id,
                interface: "eth2".to_string(),
            },
            BridgeMember {
                node: node_b_id,
                interface: "eth2".to_string(),
            },
        ],
    )
    .await?;

    teardown_db(&db).await?;
    Ok(())
}

/// A bridge stored before `members` replaced `nodes` must not stop its lab's nodes from being
/// deleted once the schema is applied again.
#[tokio::test]
#[ignore]
async fn test_schema_migrates_bridge_nodes_to_members() -> Result<()> {
    let db = setup_db("test_schema_bridge_migration").await?;

    let user = create_user(
        &db,
        "migrate_user".to_string(),
        "TestPass123!",
        false,
        vec![],
    )
    .await?;
    create_node_image(&db, NodeConfig::get_model(NodeModel::UbuntuLinux)).await?;
    let lab = create_lab(
        &db,
        "Migrate Lab",
        "migr0001",
        &user,
        &ipv4_lab_networks(
            "127.127.3.0/24",
            "172.31.3.0/24",
            "172.31.3.1",
            "172.31.3.2",
        ),
    )
    .await?;
    let lab_rid = lab.id.clone().expect("lab has id");
    create_test_node_with_model(&db, "node1", 1, NodeModel::UbuntuLinux, &lab).await?;
    create_test_node_with_model(&db, "node2", 2, NodeModel::UbuntuLinux, &lab).await?;

    // The bridge schema and record as they were before `members`.
    db.query(OLD_BRIDGE_SCHEMA).await?.check()?;
    db.query(OLD_BRIDGE_RECORD).await?.check()?;

    apply_schema(&db).await?;

    let bridges = list_bridges(&db, &lab_rid).await?;
    assert_eq!(bridges.len(), 1);
    assert!(bridges[0].members.is_empty());

    delete_lab_nodes(&db, &lab.lab_id).await?;
    assert_eq!(count_nodes_by_lab(&db, lab_rid).await?, 0);

    teardown_db(&db).await?;
    Ok(())
}
