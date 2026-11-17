use anyhow::Result;
use db::{
    DefaultRequest, LabDetails, create_lab, create_node, create_user, get_node, list_nodes_by_lab,
    update_node_ssh_username, upsert_node_image,
};
use shared::data::{DbNode, ModelKind, NodeModel, RecordId};

use crate::helper::{ipv4_lab_networks, setup_db};

async fn lab_with_node(db: &db::Database) -> Result<DbNode> {
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
    let image = upsert_node_image(
        db,
        ModelKind::default_for(NodeModel::UbuntuLinux).node_config(),
        DefaultRequest::Keep,
    )
    .await?;
    create_node(
        db,
        "vm",
        1,
        image.id.clone().unwrap(),
        lab.id.clone().unwrap(),
    )
    .await
}

#[tokio::test]
async fn a_new_node_logs_in_as_sherpa() -> Result<()> {
    let db = setup_db().await?;
    let node = lab_with_node(&db).await?;
    assert_eq!(node.ssh_username, None);
    Ok(())
}

#[tokio::test]
async fn a_nodes_ssh_username_is_stored_read_back_and_cleared() -> Result<()> {
    let db = setup_db().await?;
    let node = lab_with_node(&db).await?;
    let id = node.id.clone().unwrap();

    let updated = update_node_ssh_username(&db, id.clone(), Some("alice")).await?;
    assert_eq!(updated.ssh_username.as_deref(), Some("alice"));
    assert_eq!(
        get_node(&db, id.clone()).await?.ssh_username.as_deref(),
        Some("alice")
    );
    let listed = list_nodes_by_lab(&db, node.lab.clone()).await?;
    assert_eq!(listed[0].ssh_username.as_deref(), Some("alice"));

    // A redeploy that gives the node the sherpa account clears it.
    update_node_ssh_username(&db, id.clone(), None).await?;
    assert_eq!(get_node(&db, id).await?.ssh_username, None);
    Ok(())
}

#[tokio::test]
async fn the_ssh_username_of_a_node_that_does_not_exist_is_an_error() -> Result<()> {
    let db = setup_db().await?;
    lab_with_node(&db).await?;
    let error = update_node_ssh_username(&db, RecordId::new("node", "nope"), Some("alice"))
        .await
        .unwrap_err();
    assert!(error.to_string().contains("not found"), "{error}");
    Ok(())
}
