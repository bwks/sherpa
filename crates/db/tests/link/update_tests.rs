use anyhow::Result;
use db::{
    LabDetails, create_lab, create_link, create_node, create_user, list_links_by_lab,
    update_link_impairments, upsert_node_image,
};
use shared::data::{BridgeKind, DbLink, LinkImpairment, NodeConfig, NodeModel, RecordId};

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
        &ipv4_lab_networks(
            "127.127.8.0/24",
            "172.31.8.0/24",
            "172.31.8.1",
            "172.31.8.2",
        ),
        &LabDetails::default(),
    )
    .await?;
    let image = upsert_node_image(&db, NodeConfig::get_model(NodeModel::UbuntuLinux)).await?;
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
        loss_percent: 2.5,
        ..LinkImpairment::default()
    };

    let updated = update_link_impairments(
        &db,
        &link.id.clone().unwrap(),
        &a_to_b,
        &LinkImpairment::default(),
    )
    .await?;
    assert_eq!(updated.impairment_a_to_b, a_to_b);
    assert_eq!(updated.impairment_b_to_a, LinkImpairment::default());
    assert_eq!(
        (updated.tap_a.as_str(), updated.int_b.as_str()),
        ("ta", "eth1")
    );
    assert_eq!(
        list_links_by_lab(&db, lab_rid).await?[0].impairment_a_to_b,
        a_to_b
    );
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
