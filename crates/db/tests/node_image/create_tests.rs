/// CREATE operation tests for `node_image`
use anyhow::Result;
use db::{DefaultRequest, list_node_images, upsert_node_image};
use shared::data::{NodeModel, Ztp};

use crate::{create_test_config, set_ztp, setup_db};

#[tokio::test]
async fn test_upsert_refuses_a_ztp_method_the_model_does_not_take() -> Result<()> {
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
    Ok(())
}

#[tokio::test]
async fn test_upsert_stores_disabled_and_a_listed_method() -> Result<()> {
    let db = setup_db().await?;
    let mut off = create_test_config(NodeModel::FrrLinux);
    set_ztp(&mut off, Ztp::Disabled);
    let stored = upsert_node_image(&db, off, DefaultRequest::Keep).await?;
    assert_eq!(stored.ztp(), Ztp::Disabled);

    let mut usb = create_test_config(NodeModel::JuniperEvolved);
    set_ztp(&mut usb, Ztp::Usb);
    let stored = upsert_node_image(&db, usb, DefaultRequest::Keep).await?;
    assert_eq!(stored.ztp(), Ztp::Usb);
    Ok(())
}
