/// One default image per model: the rule every image write follows.
use anyhow::Result;
use db::{
    Database, DefaultRequest, LastDefaultImage, delete_node_image, find_default_node_image,
    get_node_image_versions, update_node_image, upsert_node_image,
};
use shared::data::{NodeConfig, NodeModel};

use crate::{create_test_config, setup_db};

const MODEL: NodeModel = NodeModel::UbuntuLinux;

fn version(name: &str) -> NodeConfig {
    let mut image = create_test_config(MODEL);
    name.clone_into(&mut image.version);
    image
}

async fn default_version(db: &Database) -> Result<Option<String>> {
    let kind = create_test_config(MODEL).kind();
    Ok(find_default_node_image(db, &MODEL, &kind)
        .await?
        .map(|image| image.version))
}

async fn record(db: &Database, name: &str, request: DefaultRequest) -> Result<NodeConfig> {
    upsert_node_image(db, version(name), request).await
}

#[tokio::test]
async fn image_default_001_the_first_version_of_a_model_and_kind_becomes_its_default_without_being_asked()
-> Result<()> {
    let db = setup_db().await?;
    record(&db, "1", DefaultRequest::Keep).await?;
    record(&db, "2", DefaultRequest::Keep).await?;
    assert_eq!(default_version(&db).await?.as_deref(), Some("1"));
    Ok(())
}

#[tokio::test]
async fn image_default_003_recording_the_default_again_without_default_keeps_it_the_default()
-> Result<()> {
    let db = setup_db().await?;
    record(&db, "1", DefaultRequest::Keep).await?;
    record(&db, "2", DefaultRequest::Keep).await?;

    let again = record(&db, "1", DefaultRequest::Keep).await?;

    assert!(again.default);
    assert_eq!(default_version(&db).await?.as_deref(), Some("1"));
    Ok(())
}

#[tokio::test]
async fn image_import_011_importing_a_version_again_keeps_the_settings_an_admin_changed()
-> Result<()> {
    let db = setup_db().await?;
    let mut edited = record(&db, "1", DefaultRequest::Keep).await?;
    edited.memory = 8192;
    edited.cpu.count = 6;
    let id = edited.id.clone();
    update_node_image(&db, edited, DefaultRequest::Keep).await?;

    let again = record(&db, "1", DefaultRequest::Keep).await?;

    assert_eq!(again.id, id, "the import returns the record it already had");
    assert_eq!(again.memory, 8192);
    assert_eq!(again.cpu.count, 6);
    let kind = create_test_config(MODEL).kind();
    assert_eq!(get_node_image_versions(&db, &MODEL, &kind).await?.len(), 1);
    Ok(())
}

#[tokio::test]
async fn image_default_002_making_a_version_the_default_leaves_the_old_default_no_longer_one()
-> Result<()> {
    let db = setup_db().await?;
    record(&db, "1", DefaultRequest::Keep).await?;
    record(&db, "2", DefaultRequest::Make).await?;

    assert_eq!(default_version(&db).await?.as_deref(), Some("2"));
    let kind = create_test_config(MODEL).kind();
    let versions = get_node_image_versions(&db, &MODEL, &kind).await?;
    let defaults: Vec<&str> = versions
        .iter()
        .filter(|image| image.default)
        .map(|image| image.version.as_str())
        .collect();
    assert_eq!(defaults, ["2"]);
    Ok(())
}

#[tokio::test]
async fn image_default_006_an_edit_cannot_unset_the_default() -> Result<()> {
    let db = setup_db().await?;
    let one = record(&db, "1", DefaultRequest::Keep).await?;
    let two = record(&db, "2", DefaultRequest::Keep).await?;

    let error = update_node_image(&db, one, DefaultRequest::Unset)
        .await
        .unwrap_err();
    assert!(
        error.downcast_ref::<LastDefaultImage>().is_some(),
        "{error:#}"
    );
    assert_eq!(default_version(&db).await?.as_deref(), Some("1"));

    // Unsetting a version that is not the default is no change.
    let two = update_node_image(&db, two, DefaultRequest::Unset).await?;
    assert!(!two.default);
    Ok(())
}

#[tokio::test]
async fn an_edit_keeps_the_version_it_was_read_with() -> Result<()> {
    let db = setup_db().await?;
    let mut renamed = record(&db, "1", DefaultRequest::Keep).await?;
    "other".clone_into(&mut renamed.version);

    let updated = update_node_image(&db, renamed, DefaultRequest::Keep).await?;

    assert_eq!(updated.version, "1");
    Ok(())
}

#[tokio::test]
async fn image_default_007_deleting_the_default_makes_the_most_recently_added_remaining_version_the_default()
-> Result<()> {
    let db = setup_db().await?;
    let one = record(&db, "1", DefaultRequest::Keep).await?;
    record(&db, "3", DefaultRequest::Keep).await?;
    record(&db, "2", DefaultRequest::Keep).await?;

    let promoted = delete_node_image(&db, one.id.unwrap()).await?;

    assert_eq!(promoted.map(|image| image.version).as_deref(), Some("2"));
    assert_eq!(default_version(&db).await?.as_deref(), Some("2"));
    Ok(())
}

#[tokio::test]
async fn image_default_008_deleting_a_version_that_is_not_the_default_or_the_last_changes_no_default()
-> Result<()> {
    let db = setup_db().await?;
    let one = record(&db, "1", DefaultRequest::Keep).await?;
    let two = record(&db, "2", DefaultRequest::Keep).await?;

    assert!(delete_node_image(&db, two.id.unwrap()).await?.is_none());
    assert_eq!(default_version(&db).await?.as_deref(), Some("1"));

    assert!(delete_node_image(&db, one.id.unwrap()).await?.is_none());
    let kind = create_test_config(MODEL).kind();
    assert_eq!(get_node_image_versions(&db, &MODEL, &kind).await?.len(), 0);
    Ok(())
}
