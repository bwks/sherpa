use anyhow::Result;
use db::{
    DbError, DefaultRequest, LabDetails, create_lab, create_node, create_user, get_node,
    list_nodes_by_lab, update_node_ready_check, upsert_node_image,
};
use shared::data::{DbNode, ModelKind, NodeKind, NodeModel, ReadyCheck, RecordId};

use crate::helper::{ipv4_lab_networks, setup_db};

async fn lab_with_node(db: &db::Database) -> Result<DbNode> {
    let user = create_user(db, "alice".to_string(), "TestPass123!", false, vec![]).await?;
    let lab = create_lab(
        db,
        "Test Lab",
        "lab-0001",
        &user,
        &ipv4_lab_networks("172.31.1.0/24", "172.31.1.1", "172.31.1.2"),
        &LabDetails::default(),
    )
    .await?;
    let image = upsert_node_image(
        db,
        ModelKind::new(NodeModel::Generic, Some(NodeKind::Container))
            .unwrap()
            .node_config(),
        DefaultRequest::Keep,
    )
    .await?;
    create_node(
        db,
        "web",
        1,
        image.id.clone().unwrap(),
        lab.id.clone().unwrap(),
    )
    .await
}

#[tokio::test]
async fn a_nodes_ready_check_is_stored_and_read_back() -> Result<()> {
    let db = setup_db().await?;
    let node = lab_with_node(&db).await?;
    let id = node.id.clone().unwrap();

    let check = ReadyCheck {
        port: Some(8080),
        skip: false,
        sshd_marker: false,
    };
    let updated = update_node_ready_check(&db, id.clone(), &check).await?;
    assert_eq!(updated.ready_check, check);
    assert_eq!(get_node(&db, id.clone()).await?.ready_check, check);

    // Every listing reads the same columns.
    let listed = list_nodes_by_lab(&db, node.lab.clone()).await?;
    assert_eq!(listed[0].ready_check, check);

    // A redeploy replaces every value.
    let skipped = ReadyCheck {
        port: None,
        skip: true,
        sshd_marker: false,
    };
    update_node_ready_check(&db, id.clone(), &skipped).await?;
    assert_eq!(get_node(&db, id.clone()).await?.ready_check, skipped);

    // A Windows node's check waits for Sherpa's SSH ready marker.
    let windows = ReadyCheck {
        port: Some(22),
        skip: false,
        sshd_marker: true,
    };
    update_node_ready_check(&db, id.clone(), &windows).await?;
    assert_eq!(get_node(&db, id).await?.ready_check, windows);
    Ok(())
}

#[tokio::test]
async fn the_ready_check_of_a_node_that_does_not_exist_is_an_error() -> Result<()> {
    let db = setup_db().await?;
    lab_with_node(&db).await?;
    let missing = RecordId::new("node", "nope");
    let error = update_node_ready_check(&db, missing.clone(), &ReadyCheck::default())
        .await
        .unwrap_err();
    assert_eq!(
        error.downcast_ref::<DbError>(),
        Some(&DbError::NodeNotFound { id: missing }),
        "{error:#}"
    );
    Ok(())
}
