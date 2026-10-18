/// Database-level protection of the built-in admin account
use anyhow::Result;
use db::{
    create_admin_user, create_user, delete_user, delete_user_by_username, get_user, list_users,
    update_user,
};
use shared::konst::{ADMIN_PROTECTED_MESSAGE, ADMIN_USERNAME};

use crate::{setup_db, teardown_db};

#[tokio::test]
#[ignore] // Requires running SurrealDB instance
async fn test_admin_cannot_be_deleted_by_id_or_username() -> Result<()> {
    let db = setup_db("test_admin_cannot_be_deleted").await?;
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

    teardown_db(&db).await?;
    Ok(())
}

#[tokio::test]
#[ignore]
async fn test_bulk_delete_is_aborted_while_admin_exists() -> Result<()> {
    let db = setup_db("test_bulk_delete_is_aborted").await?;
    create_admin_user(&db, "AdminPass123!").await?;
    create_user(&db, "alice".to_string(), "TestPass123!", false, vec![]).await?;

    let result = db.query("DELETE user").await?.check();
    assert!(result.is_err(), "Deleting every user must be refused");
    assert_eq!(list_users(&db).await?.len(), 2, "No user may be deleted");

    teardown_db(&db).await?;
    Ok(())
}

#[tokio::test]
#[ignore]
async fn test_admin_cannot_be_renamed_or_demoted() -> Result<()> {
    let db = setup_db("test_admin_cannot_be_renamed_or_demoted").await?;
    let admin = create_admin_user(&db, "AdminPass123!").await?;

    let mut demoted = admin.clone();
    demoted.is_admin = false;
    assert!(update_user(&db, demoted).await.is_err());

    let mut renamed = admin.clone();
    renamed.username = "former_admin".to_string();
    assert!(update_user(&db, renamed).await.is_err());

    for statement in [
        "UPDATE user SET is_admin = false WHERE username = 'admin'",
        "UPDATE user SET username = 'former_admin' WHERE username = 'admin'",
    ] {
        let result = db.query(statement).await?.check();
        assert!(result.is_err(), "Should refuse: {statement}");
    }

    let stored = get_user(&db, ADMIN_USERNAME).await?;
    assert!(stored.is_admin);

    teardown_db(&db).await?;
    Ok(())
}

#[tokio::test]
#[ignore]
async fn test_admin_password_and_keys_can_change() -> Result<()> {
    let db = setup_db("test_admin_password_and_keys_can_change").await?;
    let mut admin = create_admin_user(&db, "AdminPass123!").await?;

    admin.ssh_keys = vec!["ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIExample".to_string()];
    admin.password_hash = "replaced-hash".to_string();
    let updated = update_user(&db, admin).await?;
    assert_eq!(updated.ssh_keys.len(), 1);
    assert_eq!(updated.password_hash, "replaced-hash");

    teardown_db(&db).await?;
    Ok(())
}

#[tokio::test]
#[ignore]
async fn test_admin_record_without_admin_flag_is_refused() -> Result<()> {
    let db = setup_db("test_admin_record_without_admin_flag").await?;

    let result = db
        .query(
            "CREATE user CONTENT { username: 'admin', password_hash: 'x', is_admin: false, \
             created_at: time::now(), updated_at: time::now() }",
        )
        .await?
        .check();
    assert!(result.is_err(), "An admin record must have the admin flag");
    assert!(list_users(&db).await?.is_empty());

    teardown_db(&db).await?;
    Ok(())
}

#[tokio::test]
#[ignore]
async fn test_other_admins_can_be_demoted_and_deleted() -> Result<()> {
    let db = setup_db("test_other_admins_can_be_demoted_and_deleted").await?;
    create_admin_user(&db, "AdminPass123!").await?;
    let mut dana = create_user(&db, "dana".to_string(), "TestPass123!", true, vec![]).await?;

    dana.is_admin = false;
    let demoted = update_user(&db, dana).await?;
    assert!(!demoted.is_admin);
    delete_user_by_username(&db, "dana").await?;
    assert_eq!(list_users(&db).await?.len(), 1);

    teardown_db(&db).await?;
    Ok(())
}
