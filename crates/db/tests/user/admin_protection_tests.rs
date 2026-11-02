/// Protection of the built-in admin account through the db functions.
///
/// The same rules applied to raw SQL are tested in `crates/db/src/schema/user.rs`.
use anyhow::Result;
use db::{
    create_admin_user, create_user, delete_user, delete_user_by_username, get_user, list_users,
    update_user,
};
use shared::konst::{ADMIN_PROTECTED_MESSAGE, ADMIN_USERNAME};

use crate::setup_db;

#[tokio::test]
async fn test_admin_cannot_be_deleted_by_id_or_username() -> Result<()> {
    let db = setup_db().await?;
    let admin = create_admin_user(&db, "AdminPass123!").await?;

    let error = delete_user(&db, admin.id.clone().expect("admin id"))
        .await
        .unwrap_err();
    assert!(
        format!("{error:#}").contains(ADMIN_PROTECTED_MESSAGE),
        "{error:#}"
    );
    assert!(delete_user_by_username(&db, ADMIN_USERNAME).await.is_err());
    assert!(get_user(&db, ADMIN_USERNAME).await.is_ok());

    Ok(())
}

#[tokio::test]
async fn test_admin_cannot_be_renamed_or_demoted() -> Result<()> {
    let db = setup_db().await?;
    let admin = create_admin_user(&db, "AdminPass123!").await?;

    let mut demoted = admin.clone();
    demoted.is_admin = false;
    assert!(update_user(&db, demoted).await.is_err());

    let mut renamed = admin.clone();
    renamed.username = "former_admin".to_string();
    assert!(update_user(&db, renamed).await.is_err());

    let stored = get_user(&db, ADMIN_USERNAME).await?;
    assert!(stored.is_admin);

    Ok(())
}

#[tokio::test]
async fn test_admin_password_and_keys_can_change() -> Result<()> {
    let db = setup_db().await?;
    let mut admin = create_admin_user(&db, "AdminPass123!").await?;

    admin.ssh_keys = vec!["ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIExample".to_string()];
    admin.password_hash = "replaced-hash".to_string();
    let updated = update_user(&db, admin).await?;
    assert_eq!(updated.ssh_keys.len(), 1);
    assert_eq!(updated.password_hash, "replaced-hash");

    Ok(())
}

#[tokio::test]
async fn test_other_admins_can_be_demoted_and_deleted() -> Result<()> {
    let db = setup_db().await?;
    create_admin_user(&db, "AdminPass123!").await?;
    let mut dana = create_user(&db, "dana".to_string(), "TestPass123!", true, vec![]).await?;

    dana.is_admin = false;
    let demoted = update_user(&db, dana).await?;
    assert!(!demoted.is_admin);
    delete_user_by_username(&db, "dana").await?;
    assert_eq!(list_users(&db).await?.len(), 1);

    Ok(())
}
