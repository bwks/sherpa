use anyhow::Result;

use crate::{create_test_node_with_model, ipv4_lab_networks, setup_db};
use db::{
    apply_schema, count_users, create_bridge, create_lab, create_link, create_node_image,
    create_user, list_links_by_lab, update_link,
};
use shared::data::{BridgeKind, BridgeMember, LinkImpairment, NodeConfig, NodeModel};

/// Schema application is idempotent — applying twice does not fail or corrupt data.
/// setup_db already applies schema once. We apply it again and verify no error.
#[tokio::test]
async fn test_schema_idempotent() -> Result<()> {
    let db = setup_db().await?;

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

    Ok(())
}

/// Schema creates all 6 tables — verify we can insert into each one.
#[tokio::test]
async fn test_schema_creates_all_tables() -> Result<()> {
    let db = setup_db().await?;

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

    Ok(())
}

/// Each direction's impairment is stored on its own and survives applying the schema again.
#[tokio::test]
async fn test_link_impairments_survive_schema_reapply() -> Result<()> {
    let db = setup_db().await?;

    let user = create_user(
        &db,
        "impair_user".to_string(),
        "TestPass123!",
        false,
        vec![],
    )
    .await?;
    create_node_image(&db, NodeConfig::get_model(NodeModel::UbuntuLinux)).await?;
    let lab = create_lab(
        &db,
        "Impair Lab",
        "impr0001",
        &user,
        &ipv4_lab_networks(
            "127.127.4.0/24",
            "172.31.4.0/24",
            "172.31.4.1",
            "172.31.4.2",
        ),
    )
    .await?;
    let lab_rid = lab.id.clone().expect("lab has id");
    let node1 = create_test_node_with_model(&db, "node1", 1, NodeModel::UbuntuLinux, &lab).await?;
    let node2 = create_test_node_with_model(&db, "node2", 2, NodeModel::UbuntuLinux, &lab).await?;
    let mut link = create_link(
        &db,
        0,
        BridgeKind::default(),
        node1.id.clone().expect("node1 has id"),
        node2.id.clone().expect("node2 has id"),
        "eth1".to_string(),
        "eth1".to_string(),
        String::new(),
        String::new(),
        lab_rid.clone(),
    )
    .await?;

    let a_to_b = LinkImpairment {
        delay_us: 20000,
        jitter_us: 2000,
        loss_percent: 1.5,
        reorder_percent: 0.0,
        corrupt_percent: 0.5,
    };
    let b_to_a = LinkImpairment {
        delay_us: 7000,
        ..LinkImpairment::default()
    };
    link.impairment_a_to_b = a_to_b;
    link.impairment_b_to_a = b_to_a;
    update_link(&db, link).await?;

    apply_schema(&db).await?;

    let links = list_links_by_lab(&db, lab_rid).await?;
    assert_eq!(links.len(), 1);
    assert_eq!(links[0].impairment_a_to_b, a_to_b);
    assert_eq!(links[0].impairment_b_to_a, b_to_a);
    Ok(())
}
