/// CREATE operation tests for `node_image`
use anyhow::Result;
use db::{DefaultRequest, list_node_images, upsert_node_image};
use shared::data::{NodeModel, Ztp};

use crate::{create_test_config, setup_db};

#[tokio::test]
async fn test_create_and_list_node_images() -> Result<()> {
    let db = setup_db().await?;

    // Create a test config
    let test_config = create_test_config(NodeModel::AristaVeos);
    let created = upsert_node_image(&db, test_config.clone(), DefaultRequest::Keep).await?;

    assert_eq!(created.model, NodeModel::AristaVeos);
    assert!(created.id.is_some(), "Created config should have an ID");

    // List all configs
    let all_configs = list_node_images(&db).await?;
    assert!(
        !all_configs.is_empty(),
        "Should have at least one config after creation"
    );

    // Verify our config is in the list
    let found = all_configs.iter().any(|c| c.model == NodeModel::AristaVeos);
    assert!(found, "Created config should be in the list");

    // Cleanup

    Ok(())
}

#[tokio::test]
async fn test_upsert_node_image() -> Result<()> {
    let db = setup_db().await?;

    // First upsert - should create
    let test_config = create_test_config(NodeModel::NokiaSrlinux);
    let first_upsert = upsert_node_image(&db, test_config.clone(), DefaultRequest::Keep).await?;

    assert_eq!(first_upsert.model, NodeModel::NokiaSrlinux);
    let first_id = first_upsert.id.clone();

    // Second upsert - should return existing
    let second_upsert = upsert_node_image(&db, test_config, DefaultRequest::Keep).await?;

    assert_eq!(second_upsert.model, NodeModel::NokiaSrlinux);
    assert_eq!(
        second_upsert.id, first_id,
        "Upsert should return the same record"
    );

    // Cleanup

    Ok(())
}

#[tokio::test]
async fn test_upsert_refuses_a_ztp_method_the_model_does_not_take() -> Result<()> {
    let db = setup_db().await?;
    let mut config = create_test_config(NodeModel::FrrLinux);
    config.ztp.method = Ztp::Cdrom;

    let error = upsert_node_image(&db, config, DefaultRequest::Keep)
        .await
        .expect_err("the image is refused");

    assert!(
        format!("{error:#}").contains("allows: disabled, volume"),
        "{error:#}"
    );
    assert!(
        list_node_images(&db).await?.is_empty(),
        "nothing is written"
    );
    Ok(())
}

#[tokio::test]
async fn test_upsert_stores_disabled_and_a_listed_method() -> Result<()> {
    let db = setup_db().await?;
    let mut off = create_test_config(NodeModel::FrrLinux);
    off.ztp.method = Ztp::Disabled;
    let stored = upsert_node_image(&db, off, DefaultRequest::Keep).await?;
    assert_eq!(stored.ztp.method, Ztp::Disabled);

    let mut tftp = create_test_config(NodeModel::JuniperVsrxv3);
    tftp.ztp.method = Ztp::Tftp;
    let stored = upsert_node_image(&db, tftp, DefaultRequest::Keep).await?;
    assert_eq!(stored.ztp.method, Ztp::Tftp);
    Ok(())
}
