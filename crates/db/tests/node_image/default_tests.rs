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
    let kind = create_test_config(MODEL).kind;
    Ok(find_default_node_image(db, &MODEL, &kind)
        .await?
        .map(|image| image.version))
}

async fn record(db: &Database, name: &str, request: DefaultRequest) -> Result<NodeConfig> {
    upsert_node_image(db, version(name), request).await
}

#[tokio::test]
async fn the_first_version_of_a_model_becomes_its_default_without_being_asked() -> Result<()> {
    let db = setup_db().await?;
    record(&db, "1", DefaultRequest::Keep).await?;
    record(&db, "2", DefaultRequest::Keep).await?;
    assert_eq!(default_version(&db).await?.as_deref(), Some("1"));
    Ok(())
}

#[tokio::test]
async fn importing_the_default_again_without_the_option_keeps_it_the_default() -> Result<()> {
    let db = setup_db().await?;
    record(&db, "1", DefaultRequest::Keep).await?;
    record(&db, "2", DefaultRequest::Keep).await?;

    let again = record(&db, "1", DefaultRequest::Keep).await?;

    assert!(again.default);
    assert_eq!(default_version(&db).await?.as_deref(), Some("1"));
    Ok(())
}

#[tokio::test]
async fn importing_a_version_again_keeps_its_edited_settings() -> Result<()> {
    let db = setup_db().await?;
    let mut edited = record(&db, "1", DefaultRequest::Keep).await?;
    edited.compute.memory = 8192;
    edited.compute.cpu_count = 6;
    update_node_image(&db, edited, DefaultRequest::Keep).await?;

    let again = record(&db, "1", DefaultRequest::Keep).await?;

    assert_eq!(again.compute.memory, 8192);
    assert_eq!(again.compute.cpu_count, 6);
    Ok(())
}

#[tokio::test]
async fn making_a_version_the_default_moves_the_flag() -> Result<()> {
    let db = setup_db().await?;
    record(&db, "1", DefaultRequest::Keep).await?;
    record(&db, "2", DefaultRequest::Make).await?;
    assert_eq!(default_version(&db).await?.as_deref(), Some("2"));
    Ok(())
}

#[tokio::test]
async fn an_edit_cannot_unset_the_only_default() -> Result<()> {
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
async fn deleting_the_default_makes_the_newest_remaining_version_the_default() -> Result<()> {
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
async fn deleting_another_version_or_the_last_promotes_nothing() -> Result<()> {
    let db = setup_db().await?;
    let one = record(&db, "1", DefaultRequest::Keep).await?;
    let two = record(&db, "2", DefaultRequest::Keep).await?;

    assert!(delete_node_image(&db, two.id.unwrap()).await?.is_none());
    assert_eq!(default_version(&db).await?.as_deref(), Some("1"));

    assert!(delete_node_image(&db, one.id.unwrap()).await?.is_none());
    let kind = create_test_config(MODEL).kind;
    assert!(
        get_node_image_versions(&db, &MODEL, &kind)
            .await?
            .is_empty()
    );
    Ok(())
}
