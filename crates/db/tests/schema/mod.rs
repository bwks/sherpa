use anyhow::Result;

use crate::{create_test_node_with_model, ipv4_lab_networks, setup_db};
use db::{
    DefaultRequest, LabDetails, apply_schema, create_bridge, create_lab, create_link, create_user,
    list_links_by_lab, list_users, update_link_impairments, upsert_node_image,
};
use shared::data::{BridgeKind, BridgeMember, DbLink, LinkImpairment, NodeConfig, NodeModel};

/// Schema application is idempotent — applying twice does not fail or corrupt data.
/// `setup_db` already applies schema once. We apply it again and verify no error.
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
    let count = list_users(&db).await?.len();
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
    upsert_node_image(
        &db,
        NodeConfig::get_model(NodeModel::UbuntuLinux),
        DefaultRequest::Keep,
    )
    .await?;

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
        &LabDetails::default(),
    )
    .await?;

    // node (via helper)
    let node = create_test_node_with_model(&db, "node1", 1, NodeModel::UbuntuLinux, &lab).await?;

    // link
    let node2 = create_test_node_with_model(&db, "node2", 2, NodeModel::UbuntuLinux, &lab).await?;
    let first_end = node.id.clone().expect("node has id");
    let second_end = node2.id.clone().expect("node2 has id");
    let lab_id = lab.id.clone().expect("lab has id");
    create_link(
        &db,
        DbLink {
            id: None,
            index: 0,
            kind: BridgeKind::default(),
            node_a: first_end.clone(),
            node_b: second_end.clone(),
            int_a: "eth1".to_string(),
            int_b: "eth1".to_string(),
            lab: lab_id.clone(),
            tap_a: String::new(),
            tap_b: String::new(),
            impairment_a_to_b: LinkImpairment::default(),
            impairment_b_to_a: LinkImpairment::default(),
        },
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
                node: first_end,
                interface: "eth2".to_string(),
            },
            BridgeMember {
                node: second_end,
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
    upsert_node_image(
        &db,
        NodeConfig::get_model(NodeModel::UbuntuLinux),
        DefaultRequest::Keep,
    )
    .await?;
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
        &LabDetails::default(),
    )
    .await?;
    let lab_rid = lab.id.clone().expect("lab has id");
    let node1 = create_test_node_with_model(&db, "node1", 1, NodeModel::UbuntuLinux, &lab).await?;
    let node2 = create_test_node_with_model(&db, "node2", 2, NodeModel::UbuntuLinux, &lab).await?;
    let link = create_link(
        &db,
        DbLink {
            id: None,
            index: 0,
            kind: BridgeKind::default(),
            node_a: node1.id.clone().expect("node1 has id"),
            node_b: node2.id.clone().expect("node2 has id"),
            int_a: "eth1".to_string(),
            int_b: "eth1".to_string(),
            lab: lab_rid.clone(),
            tap_a: String::new(),
            tap_b: String::new(),
            impairment_a_to_b: LinkImpairment::default(),
            impairment_b_to_a: LinkImpairment::default(),
        },
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
    update_link_impairments(&db, &link.id.clone().unwrap(), &a_to_b, &b_to_a).await?;

    apply_schema(&db).await?;

    let links = list_links_by_lab(&db, lab_rid).await?;
    assert_eq!(links.len(), 1);
    assert_eq!(links[0].impairment_a_to_b, a_to_b);
    assert_eq!(links[0].impairment_b_to_a, b_to_a);
    Ok(())
}
