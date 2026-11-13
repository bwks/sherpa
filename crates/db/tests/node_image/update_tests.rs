/// UPDATE operation tests for `node_image`
use anyhow::Result;
use db::{DefaultRequest, find_node_image_by_id, update_node_image, upsert_node_image};
use shared::data::{NodeModel, RecordId, Ztp};

use crate::{create_test_config, setup_db};

#[tokio::test]
async fn test_update_node_image_success() -> Result<()> {
    let db = setup_db().await?;

    // Create a config
    let test_config = create_test_config(NodeModel::AristaVeos);
    let created = upsert_node_image(&db, test_config, DefaultRequest::Keep).await?;
    let original_id = created.id.clone();

    // Modify multiple fields
    let mut updated_config = created.clone();
    updated_config.memory = 4096;
    updated_config.cpu.count = 4;
    updated_config.interfaces.data_count = 24;

    // Update the config
    let result = update_node_image(&db, updated_config, DefaultRequest::Keep).await?;

    // Verify changes were applied
    assert_eq!(result.memory, 4096, "Memory should be updated");
    assert_eq!(result.cpu.count, 4, "CPU count should be updated");
    assert_eq!(
        result.interfaces.data_count, 24,
        "Interface count should be updated"
    );
    assert_eq!(result.id, original_id, "ID should remain unchanged");
    assert_eq!(result.model, NodeModel::AristaVeos, "Model unchanged");

    Ok(())
}

#[tokio::test]
async fn test_update_node_image_without_id_fails() -> Result<()> {
    let db = setup_db().await?;

    // Create a config without an ID
    let mut test_config = create_test_config(NodeModel::CiscoAsav);
    test_config.id = None; // Explicitly remove ID

    // Attempt to update should fail
    let result = update_node_image(&db, test_config, DefaultRequest::Keep).await;

    assert!(result.is_err(), "Update without ID should fail");
    let err = result.unwrap_err();
    assert!(
        err.to_string().contains("has no ID"),
        "Error should mention missing ID: {err}"
    );

    Ok(())
}

#[tokio::test]
async fn test_update_nonexistent_config_fails() -> Result<()> {
    let db = setup_db().await?;

    // Create a config with a fake/nonexistent ID
    let mut test_config = create_test_config(NodeModel::JuniperVrouter);
    test_config.id = Some(RecordId::new("node_image", "nonexistent_id_12345"));

    // Attempt to update should fail
    let result = update_node_image(&db, test_config, DefaultRequest::Keep).await;

    assert!(result.is_err(), "Update with nonexistent ID should fail");
    let err = result.unwrap_err();
    assert!(
        err.to_string().contains("not found"),
        "Error should mention record not found: {err}"
    );

    Ok(())
}

#[tokio::test]
async fn test_update_preserves_id() -> Result<()> {
    let db = setup_db().await?;

    // Create a config
    let test_config = create_test_config(NodeModel::NokiaSrlinux);
    let created = upsert_node_image(&db, test_config, DefaultRequest::Keep).await?;
    let original_id = created.id.clone().expect("Created config should have ID");

    // Update multiple times
    for i in 1..=3 {
        let mut updated_config = created.clone();
        updated_config.memory = 1024 * i;
        updated_config.id = original_id.clone().into();

        let result = update_node_image(&db, updated_config, DefaultRequest::Keep).await?;

        assert_eq!(
            result.id,
            original_id.clone().into(),
            "ID should remain constant after update {i}"
        );
        assert_eq!(
            result.memory,
            1024 * i,
            "Memory should be updated to {}",
            1024 * i
        );
    }

    // Verify via get_by_id that ID is still correct
    let retrieved = find_node_image_by_id(&db, original_id.clone())
        .await?
        .expect("Config should still exist");

    assert_eq!(
        retrieved.id,
        original_id.into(),
        "Retrieved config should have original ID"
    );
    assert_eq!(retrieved.memory, 3072, "Final memory value should be 3072");

    Ok(())
}

#[tokio::test]
async fn test_update_refuses_a_ztp_method_the_model_does_not_take() -> Result<()> {
    let db = setup_db().await?;
    let created = upsert_node_image(
        &db,
        create_test_config(NodeModel::AristaVeos),
        DefaultRequest::Keep,
    )
    .await?;
    let id = created.id.clone().expect("the image has an ID");

    let mut edited = created.clone();
    edited.ztp.method = Ztp::Volume;
    edited.memory = 8192;
    let error = update_node_image(&db, edited, DefaultRequest::Keep)
        .await
        .expect_err("the edit is refused");

    assert!(
        format!("{error:#}").contains("allows: disabled, tftp"),
        "{error:#}"
    );
    let stored = find_node_image_by_id(&db, id)
        .await?
        .expect("the image is stored");
    assert_eq!(stored.ztp.method, Ztp::Tftp, "the setting is unchanged");
    assert_eq!(stored.memory, created.memory, "nothing else is written");
    Ok(())
}

#[tokio::test]
async fn test_update_stores_a_ztp_setting_the_model_takes() -> Result<()> {
    let db = setup_db().await?;
    let created = upsert_node_image(
        &db,
        create_test_config(NodeModel::AristaVeos),
        DefaultRequest::Keep,
    )
    .await?;

    let mut edited = created;
    edited.ztp.method = Ztp::Disabled;
    let stored = update_node_image(&db, edited, DefaultRequest::Keep).await?;

    assert_eq!(stored.ztp.method, Ztp::Disabled);
    Ok(())
}
