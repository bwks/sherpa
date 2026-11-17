//! A node's image reference, which a redeploy moves to the image it rebuilds the node from.

use anyhow::Result;
use db::{
    DefaultRequest, LabDetails, create_lab, create_node, create_user, delete_node_image, get_node,
    update_node_image_ref, upsert_node_image,
};
use shared::data::{ModelKind, NodeConfig, NodeKind, NodeModel, RecordId};

use crate::helper::{ipv4_lab_networks, setup_db};

/// A stored `arista_eos` container image at `version`.
async fn ceos(db: &db::Database, version: &str) -> Result<NodeConfig> {
    let image = NodeConfig {
        version: version.to_owned(),
        ..ModelKind::new(NodeModel::AristaEos, Some(NodeKind::Container))?.node_config()
    };
    upsert_node_image(db, image, DefaultRequest::Keep).await
}

/// A lab with one node on `image`.
async fn node_on(db: &db::Database, image: &NodeConfig) -> Result<RecordId> {
    let user = create_user(db, "alice".to_string(), "TestPass123!", false, vec![]).await?;
    let lab = create_lab(
        db,
        "Test Lab",
        "lab-0001",
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
    let node = create_node(
        db,
        "ceos1",
        1,
        image.id.clone().unwrap(),
        lab.id.clone().unwrap(),
    )
    .await?;
    Ok(node.id.unwrap())
}

#[tokio::test]
async fn a_node_moved_to_another_version_frees_the_old_image() -> Result<()> {
    let db = setup_db().await?;
    let old = ceos(&db, "4.33").await?;
    let new = ceos(&db, "4.34").await?;
    let node = node_on(&db, &old).await?;

    // The old image is in use, so it cannot be deleted.
    assert!(
        delete_node_image(&db, old.id.clone().unwrap())
            .await
            .is_err()
    );

    let moved = update_node_image_ref(&db, node.clone(), new.id.as_ref().unwrap()).await?;
    assert_eq!(moved.image, new.id.clone().unwrap());
    assert_eq!(get_node(&db, node).await?.image, new.id.clone().unwrap());

    // Now the node uses the new image, the old one can go, and the new one cannot.
    assert!(delete_node_image(&db, old.id.unwrap()).await.is_ok());
    assert!(delete_node_image(&db, new.id.unwrap()).await.is_err());
    Ok(())
}

#[tokio::test]
async fn a_node_cannot_be_moved_to_an_image_that_does_not_exist() -> Result<()> {
    let db = setup_db().await?;
    let image = ceos(&db, "4.33").await?;
    let node = node_on(&db, &image).await?;

    let missing = RecordId::new("node_image", "missing");
    assert!(
        update_node_image_ref(&db, node.clone(), &missing)
            .await
            .is_err()
    );
    assert_eq!(get_node(&db, node).await?.image, image.id.unwrap());
    Ok(())
}
