/// READ operation tests for `node_image`
use anyhow::Result;
use db::{
    DefaultRequest, count_node_images, find_default_node_image, find_node_image_by_id,
    find_node_image_by_model_kind_version, get_node_image_versions, list_node_images,
    upsert_node_image,
};
use shared::data::{NodeConfig, NodeKind, NodeModel};

use crate::{create_test_config, setup_db};

#[tokio::test]
async fn test_get_node_image_by_id() -> Result<()> {
    let db = setup_db().await?;

    // Create a test config
    let test_config = create_test_config(NodeModel::AristaCeos);
    let created = upsert_node_image(&db, test_config, DefaultRequest::Keep).await?;

    let created_id = created
        .id
        .clone()
        .expect("Created config should have an ID");

    // Get by ID
    let retrieved = find_node_image_by_id(&db, created_id).await?;

    assert!(retrieved.is_some(), "Should find config by ID");
    let retrieved = retrieved.unwrap();
    assert_eq!(retrieved.model, NodeModel::AristaCeos);

    Ok(())
}

#[tokio::test]
async fn test_get_node_config_by_model_kind() -> Result<()> {
    let db = setup_db().await?;

    // Create a test config
    let test_config = create_test_config(NodeModel::CiscoAsav);
    let _created = upsert_node_image(&db, test_config.clone(), DefaultRequest::Keep).await?;

    // Get by model and kind
    let retrieved: Option<_> = find_node_image_by_model_kind_version(
        &db,
        &NodeModel::CiscoAsav,
        &test_config.kind,
        &test_config.version,
    )
    .await?;

    assert!(retrieved.is_some(), "Should find config by model and kind");
    let retrieved = retrieved.unwrap();
    assert_eq!(retrieved.model, NodeModel::CiscoAsav);

    Ok(())
}

#[tokio::test]
async fn test_count_node_images() -> Result<()> {
    let db = setup_db().await?;

    let initial_count = count_node_images(&db).await?;

    // Create a test config
    let test_config = create_test_config(NodeModel::JuniperVrouter);
    let _created = upsert_node_image(&db, test_config, DefaultRequest::Keep).await?;

    let new_count = count_node_images(&db).await?;
    assert_eq!(
        new_count,
        initial_count + 1,
        "Count should increase by 1 after creation"
    );

    Ok(())
}

#[tokio::test]
async fn test_get_nonexistent_config() -> Result<()> {
    let db = setup_db().await?;

    // Try to get a config that doesn't exist
    let result: Option<_> = find_node_image_by_model_kind_version(
        &db,
        &NodeModel::WindowsServer,
        &NodeKind::VirtualMachine,
        "nonexistent_version",
    )
    .await?;

    // Should return None if not found
    assert!(
        result.is_none(),
        "Should return None for nonexistent config"
    );

    Ok(())
}

#[tokio::test]
async fn test_list_configs_returns_all_fields() -> Result<()> {
    let db = setup_db().await?;

    // Create a config
    let test_config = create_test_config(NodeModel::UbuntuLinux);
    let _created = upsert_node_image(&db, test_config, DefaultRequest::Keep).await?;

    // List and verify all fields are populated
    let configs = list_node_images(&db).await?;
    let ubuntu_config = configs
        .iter()
        .find(|c| c.model == NodeModel::UbuntuLinux)
        .expect("Should find Ubuntu config");

    // Verify key fields
    assert!(ubuntu_config.id.is_some());
    assert!(ubuntu_config.cpu.count > 0);
    assert!(ubuntu_config.memory > 0);
    assert!(ubuntu_config.interfaces.data_count > 0);
    assert_ne!(ubuntu_config.interfaces.prefix, "");

    Ok(())
}

#[tokio::test]
async fn image_versions_are_in_order_and_one_is_the_default() -> Result<()> {
    let db = setup_db().await?;
    for (version, request) in [
        ("2", DefaultRequest::Keep),
        ("1", DefaultRequest::Make),
        ("3", DefaultRequest::Keep),
    ] {
        let mut image = create_test_config(NodeModel::UbuntuLinux);
        image.version = version.to_owned();
        upsert_node_image(&db, image, request).await?;
    }
    let kind = create_test_config(NodeModel::UbuntuLinux).kind;

    let versions = get_node_image_versions(&db, &NodeModel::UbuntuLinux, &kind).await?;
    let names: Vec<&str> = versions
        .iter()
        .map(|image| image.version.as_str())
        .collect();
    assert_eq!(names, ["1", "2", "3"]);
    let default = find_default_node_image(&db, &NodeModel::UbuntuLinux, &kind).await?;
    assert_eq!(default.map(|image| image.version), Some("1".to_owned()));

    // A new default replaces the old one.
    let mut three = create_test_config(NodeModel::UbuntuLinux);
    three.version = "3".to_owned();
    upsert_node_image(&db, three, DefaultRequest::Make).await?;
    let versions = get_node_image_versions(&db, &NodeModel::UbuntuLinux, &kind).await?;
    let defaults: Vec<&str> = versions
        .iter()
        .filter(|image| image.default)
        .map(|image| image.version.as_str())
        .collect();
    assert_eq!(defaults, ["3"]);
    Ok(())
}

/// Every model's built-in image reads back as it was written: the mapper reads the columns by
/// position, so a field read out of column order would land in another field or fail to decode.
#[tokio::test]
async fn test_every_model_image_reads_back_as_written() -> Result<()> {
    let db = setup_db().await?;

    for model in NodeModel::to_vec() {
        let written = NodeConfig::get_model(model);
        let created = upsert_node_image(&db, written.clone(), DefaultRequest::Keep).await?;
        let id = created.id.clone().expect("Created image should have an ID");

        let read = find_node_image_by_id(&db, id)
            .await?
            .expect("Created image should be found");

        let mut read_json = serde_json::to_value(&read)?;
        let mut written_json = serde_json::to_value(&written)?;
        read_json["id"] = serde_json::Value::Null;
        written_json["id"] = serde_json::Value::Null;
        assert_eq!(read_json, written_json, "{model}");
    }

    Ok(())
}
