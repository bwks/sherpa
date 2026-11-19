use anyhow::Result;
use db::{
    DefaultRequest, LabDetails, create_lab, create_link, create_node, create_user,
    list_links_by_lab, update_link_impairments, upsert_node_image,
};
use shared::data::{BridgeKind, DbLink, LinkImpairment, ModelKind, NodeModel, RecordId};

use crate::helper::{ipv4_lab_networks, setup_db};

#[tokio::test]
async fn impairments_are_set_and_the_rest_of_the_link_is_kept() -> Result<()> {
    let db = setup_db().await?;
    let user = create_user(&db, "gina".to_string(), "TestPass123!", false, vec![]).await?;
    let lab = create_lab(
        &db,
        "Impaired",
        "imp00001",
        &user,
        &ipv4_lab_networks("172.31.8.0/24", "172.31.8.1", "172.31.8.2"),
        &LabDetails::default(),
    )
    .await?;
    let image = upsert_node_image(
        &db,
        ModelKind::default_for(NodeModel::UbuntuLinux).node_config(),
        DefaultRequest::Keep,
    )
    .await?;
    let lab_rid = lab.id.clone().unwrap();
    let a = create_node(&db, "a", 1, image.id.clone().unwrap(), lab_rid.clone()).await?;
    let b = create_node(&db, "b", 2, image.id.clone().unwrap(), lab_rid.clone()).await?;
    let link = create_link(
        &db,
        DbLink {
            id: None,
            index: 0,
            kind: BridgeKind::P2p,
            node_a: a.id.clone().unwrap(),
            node_b: b.id.clone().unwrap(),
            int_a: "eth1".to_string(),
            int_b: "eth1".to_string(),
            lab: lab_rid.clone(),
            tap_a: "ta".to_string(),
            tap_b: "tb".to_string(),
            impairment_a_to_b: LinkImpairment::default(),
            impairment_b_to_a: LinkImpairment::default(),
        },
    )
    .await?;
    let a_to_b = LinkImpairment {
        delay_us: 5000,
        jitter_us: 2000,
        loss_percent: 2.5,
        reorder_percent: 0.0,
        corrupt_percent: 0.5,
    };
    let b_to_a = LinkImpairment {
        delay_us: 7000,
        ..LinkImpairment::default()
    };

    let updated = update_link_impairments(&db, &link.id.clone().unwrap(), &a_to_b, &b_to_a).await?;
    assert_eq!(updated.impairment_a_to_b, a_to_b);
    assert_eq!(updated.impairment_b_to_a, b_to_a);
    assert_eq!(
        (updated.tap_a.as_str(), updated.int_b.as_str()),
        ("ta", "eth1")
    );
    let stored = &list_links_by_lab(&db, lab_rid).await?[0];
    assert_eq!(stored.impairment_a_to_b, a_to_b);
    assert_eq!(stored.impairment_b_to_a, b_to_a);
    Ok(())
}

#[tokio::test]
async fn impairments_of_a_missing_link_are_refused() -> Result<()> {
    let db = setup_db().await?;
    let missing = RecordId::new("link", "missing");
    let error = update_link_impairments(
        &db,
        &missing,
        &LinkImpairment::default(),
        &LinkImpairment::default(),
    )
    .await
    .unwrap_err();
    assert!(format!("{error:#}").contains("missing"), "{error:#}");
    Ok(())
}
