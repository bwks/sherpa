//! Session rows: start, look up, end, sweep, and how password changes and user deletion end them.
use anyhow::Result;
use db::{
    change_user_password, create_session, create_user, delete_expired_sessions, delete_session,
    delete_user, find_session, get_user_id,
};
use jiff::{SignedDuration, Timestamp};
use shared::data::{DbUser, SessionChannel};

use crate::setup_db;

const PASSWORD: &str = "TestPass123!";

fn in_a_day() -> Timestamp {
    Timestamp::now() + SignedDuration::from_hours(24)
}

async fn alice(db: &db::Database) -> Result<DbUser> {
    create_user(db, "alice".to_owned(), PASSWORD, false, vec![]).await
}

#[tokio::test]
async fn a_started_session_is_found_with_its_user() -> Result<()> {
    let db = setup_db().await?;
    let user = alice(&db).await?;
    let expires_at = Timestamp::from_second(in_a_day().as_second())?;

    let session = create_session(&db, get_user_id(&user)?, SessionChannel::Cli, expires_at).await?;

    let found = find_session(&db, &session.id).await?.expect("session row");
    assert_eq!(found.session.id, session.id);
    assert_eq!(found.session.user, get_user_id(&user)?);
    assert_eq!(found.session.channel, SessionChannel::Cli);
    assert_eq!(found.session.expires_at, expires_at);
    assert_eq!(found.username, "alice");
    assert!(!found.is_admin);
    Ok(())
}

#[tokio::test]
async fn each_session_has_its_own_id() -> Result<()> {
    let db = setup_db().await?;
    let user = alice(&db).await?;
    let first = create_session(&db, get_user_id(&user)?, SessionChannel::Web, in_a_day()).await?;
    let second = create_session(&db, get_user_id(&user)?, SessionChannel::Web, in_a_day()).await?;
    assert_ne!(first.id, second.id);
    Ok(())
}

#[tokio::test]
async fn deleting_a_session_ends_only_that_session() -> Result<()> {
    let db = setup_db().await?;
    let user = alice(&db).await?;
    let web = create_session(&db, get_user_id(&user)?, SessionChannel::Web, in_a_day()).await?;
    let cli = create_session(&db, get_user_id(&user)?, SessionChannel::Cli, in_a_day()).await?;

    assert!(delete_session(&db, &web.id).await?);

    assert!(find_session(&db, &web.id).await?.is_none());
    assert!(find_session(&db, &cli.id).await?.is_some());
    // Ending it again is not an error: it is already ended.
    assert!(!delete_session(&db, &web.id).await?);
    Ok(())
}

#[tokio::test]
async fn deleting_a_user_ends_their_sessions() -> Result<()> {
    let db = setup_db().await?;
    let user = alice(&db).await?;
    let session = create_session(&db, get_user_id(&user)?, SessionChannel::Web, in_a_day()).await?;

    delete_user(&db, get_user_id(&user)?).await?;

    assert!(find_session(&db, &session.id).await?.is_none());
    // A new account with the same name does not inherit the session.
    alice(&db).await?;
    assert!(find_session(&db, &session.id).await?.is_none());
    Ok(())
}

#[tokio::test]
async fn a_password_change_ends_every_session_but_the_one_kept() -> Result<()> {
    let db = setup_db().await?;
    let user = alice(&db).await?;
    let id = get_user_id(&user)?;
    let current = create_session(&db, id.clone(), SessionChannel::Web, in_a_day()).await?;
    let other = create_session(&db, id.clone(), SessionChannel::Cli, in_a_day()).await?;

    let ended = change_user_password(&db, id.clone(), "new-hash", Some(&current.id)).await?;

    assert_eq!(ended, 1);
    assert!(find_session(&db, &current.id).await?.is_some());
    assert!(find_session(&db, &other.id).await?.is_none());
    let changed = db::get_user(&db, "alice").await?;
    assert_eq!(changed.password_hash, "new-hash");
    Ok(())
}

#[tokio::test]
async fn a_password_reset_without_a_kept_session_ends_them_all() -> Result<()> {
    let db = setup_db().await?;
    let user = alice(&db).await?;
    let id = get_user_id(&user)?;
    let first = create_session(&db, id.clone(), SessionChannel::Web, in_a_day()).await?;
    let second = create_session(&db, id.clone(), SessionChannel::Cli, in_a_day()).await?;

    assert_eq!(change_user_password(&db, id, "new-hash", None).await?, 2);

    assert!(find_session(&db, &first.id).await?.is_none());
    assert!(find_session(&db, &second.id).await?.is_none());
    Ok(())
}

#[tokio::test]
async fn a_password_change_keeps_concurrent_ssh_key_edits() -> Result<()> {
    let db = setup_db().await?;
    let user = alice(&db).await?;
    // An SSH key edit lands between the password change's read and its write.
    let mut edited = user.clone();
    edited.ssh_keys = vec!["ssh-ed25519 AAAA alice".to_owned()];
    db::update_user(&db, edited).await?;

    change_user_password(&db, get_user_id(&user)?, "new-hash", None).await?;

    let stored = db::get_user(&db, "alice").await?;
    assert_eq!(stored.ssh_keys, vec!["ssh-ed25519 AAAA alice".to_owned()]);
    assert_eq!(stored.password_hash, "new-hash");
    Ok(())
}

#[tokio::test]
async fn the_sweep_removes_only_expired_sessions() -> Result<()> {
    let db = setup_db().await?;
    let user = alice(&db).await?;
    let id = get_user_id(&user)?;
    let now = Timestamp::now();
    let expired = create_session(
        &db,
        id.clone(),
        SessionChannel::Web,
        now - SignedDuration::from_secs(1),
    )
    .await?;
    let at_now = create_session(&db, id.clone(), SessionChannel::Web, now).await?;
    let live = create_session(&db, id, SessionChannel::Cli, in_a_day()).await?;

    assert_eq!(delete_expired_sessions(&db, now).await?, 2);

    assert!(find_session(&db, &expired.id).await?.is_none());
    assert!(find_session(&db, &at_now.id).await?.is_none());
    assert!(find_session(&db, &live.id).await?.is_some());
    Ok(())
}
