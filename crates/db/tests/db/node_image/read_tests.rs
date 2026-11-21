/// READ operation tests for `node_image`
use anyhow::Result;
use db::{
    DefaultRequest, count_node_images, find_node_image_by_id, get_node_image_versions,
    upsert_node_image,
};
use shared::data::{ModelKind, NodeModel};

use crate::{create_test_config, setup_db};

#[tokio::test]
async fn web_layout_007_the_dashboards_image_count_is_every_recorded_image() -> Result<()> {
    let db = setup_db().await?;

    let initial_count = count_node_images(&db).await?;

    // Create a test config
    let test_config = create_test_config(NodeModel::JuniperRouter);
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
async fn image_list_011_the_versions_of_a_model_and_kind_are_listed_in_version_order() -> Result<()>
{
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
    let kind = create_test_config(NodeModel::UbuntuLinux).kind();

    let versions = get_node_image_versions(&db, &NodeModel::UbuntuLinux, &kind).await?;
    let names: Vec<&str> = versions
        .iter()
        .map(|image| image.version.as_str())
        .collect();
    assert_eq!(names, ["1", "2", "3"]);
    Ok(())
}

/// Every model's built-in image reads back as it was written: the mapper reads the columns by
/// position, so a field read out of column order would land in another field or fail to decode.
#[tokio::test]
async fn image_settings_012_every_models_built_in_settings_are_recorded_and_read_back_unchanged()
-> Result<()> {
    let db = setup_db().await?;

    for model in NodeModel::to_vec() {
        let written = ModelKind::default_for(model).node_config();
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
