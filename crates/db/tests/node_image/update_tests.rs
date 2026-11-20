/// UPDATE operation tests for `node_image`
use anyhow::Result;
use db::{DefaultRequest, find_node_image_by_id, update_node_image, upsert_node_image};
use shared::data::{NodeModel, Ztp};

use crate::{create_test_config, set_ztp, setup_db};

#[tokio::test]
async fn image_settings_002_a_changed_cpu_count_memory_and_interface_count_are_stored_under_the_same_image()
-> Result<()> {
    let db = setup_db().await?;

    // Create a config
    let test_config = create_test_config(NodeModel::AristaEos);
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
    assert_eq!(result.model, NodeModel::AristaEos, "Model unchanged");

    let stored = find_node_image_by_id(&db, original_id.unwrap())
        .await?
        .expect("the image is stored");
    assert_eq!(stored.memory, 4096);
    assert_eq!(stored.cpu.count, 4);
    assert_eq!(stored.interfaces.data_count, 24);

    Ok(())
}

#[tokio::test]
async fn test_update_refuses_a_ztp_method_the_model_does_not_take() -> Result<()> {
    let db = setup_db().await?;
    let created = upsert_node_image(
        &db,
        create_test_config(NodeModel::AristaEos),
        DefaultRequest::Keep,
    )
    .await?;
    let id = created.id.clone().expect("the image has an ID");

    let mut edited = created.clone();
    set_ztp(&mut edited, Ztp::Volume);
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
    assert_eq!(stored.ztp(), Ztp::Tftp, "the setting is unchanged");
    assert_eq!(stored.memory, created.memory, "nothing else is written");
    Ok(())
}

#[tokio::test]
async fn test_update_stores_a_ztp_setting_the_model_takes() -> Result<()> {
    let db = setup_db().await?;
    let created = upsert_node_image(
        &db,
        create_test_config(NodeModel::AristaEos),
        DefaultRequest::Keep,
    )
    .await?;

    let mut edited = created;
    set_ztp(&mut edited, Ztp::Disabled);
    let stored = update_node_image(&db, edited, DefaultRequest::Keep).await?;

    assert_eq!(stored.ztp(), Ztp::Disabled);
    Ok(())
}
