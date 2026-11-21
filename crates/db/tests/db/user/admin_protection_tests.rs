/// What the built-in admin account may still change.
use anyhow::Result;
use db::{create_admin_user, update_user};

use crate::setup_db;

#[tokio::test]
async fn builtin_admin_006_the_password_and_keys_of_admin_can_be_changed() -> Result<()> {
    let db = setup_db().await?;
    let mut admin = create_admin_user(&db, "AdminPass123!").await?;

    admin.ssh_keys = vec!["ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIExample".to_string()];
    admin.password_hash = "replaced-hash".to_string();
    let updated = update_user(&db, admin).await?;
    assert_eq!(updated.ssh_keys.len(), 1);
    assert_eq!(updated.password_hash, "replaced-hash");

    Ok(())
}
