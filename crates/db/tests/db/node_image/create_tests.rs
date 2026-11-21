/// CREATE operation tests for `node_image`
use anyhow::Result;
use db::{
    DefaultRequest, find_node_image_by_id, list_node_images, update_node_image, upsert_node_image,
};
use shared::data::{NodeModel, Ztp};

use crate::{create_test_config, set_ztp, setup_db};

#[tokio::test]
async fn ztp_setting_007_the_database_refuses_a_method_the_model_does_not_take() -> Result<()> {
    let db = setup_db().await?;
    let mut config = create_test_config(NodeModel::FrrLinux);
    set_ztp(&mut config, Ztp::Cdrom);

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
async fn ztp_setting_008_the_database_stores_disabled_and_a_method_the_model_takes() -> Result<()> {
    let db = setup_db().await?;
    let mut off = create_test_config(NodeModel::FrrLinux);
    set_ztp(&mut off, Ztp::Disabled);
    let stored = upsert_node_image(&db, off, DefaultRequest::Keep).await?;
    assert_eq!(stored.ztp(), Ztp::Disabled);

    let mut usb = create_test_config(NodeModel::JuniperEvolved);
    set_ztp(&mut usb, Ztp::Usb);
    let stored = upsert_node_image(&db, usb, DefaultRequest::Keep).await?;
    assert_eq!(stored.ztp(), Ztp::Usb);

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
