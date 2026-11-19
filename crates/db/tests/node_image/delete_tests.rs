/// DELETE operation tests for `node_image`
use anyhow::Result;
use db::{
    DefaultRequest, delete_node_image, find_node_image_by_id,
    find_node_image_by_model_kind_version, upsert_node_image,
};
use shared::data::{ModelKind, NodeKind, NodeModel, RecordId};

use crate::setup_db;

#[tokio::test]
async fn deleting_one_kind_of_a_model_at_a_version_keeps_the_other_kind() -> Result<()> {
    let db = setup_db().await?;
    let mut ids = Vec::new();
    for kind in [NodeKind::VirtualMachine, NodeKind::Unikernel] {
        let mut image = ModelKind::new(NodeModel::Generic, Some(kind))?.node_config();
        image.version = "1.0".to_owned();
        let created = upsert_node_image(&db, image, DefaultRequest::Keep).await?;
        ids.push(created.id.expect("created image has an id"));
    }

    delete_node_image(&db, ids[0].clone()).await?;

    let unikernel = find_node_image_by_model_kind_version(
        &db,
        &NodeModel::Generic,
        &NodeKind::Unikernel,
        "1.0",
    )
    .await?;
    assert_eq!(unikernel.and_then(|image| image.id), Some(ids[1].clone()));
    assert!(find_node_image_by_id(&db, ids[0].clone()).await?.is_none());
    Ok(())
}

#[tokio::test]
async fn test_delete_nonexistent_config_fails() -> Result<()> {
    let db = setup_db().await?;

    // Attempt to delete a config with fake/nonexistent ID
    let fake_id = RecordId::new("node_image", "nonexistent_id_99999");
    let result = delete_node_image(&db, fake_id).await;

    assert!(result.is_err(), "Delete with nonexistent ID should fail");
    let err = result.unwrap_err();
    assert!(
        err.to_string().contains("not found"),
        "Error should mention record not found: {err}"
    );

    Ok(())
}
