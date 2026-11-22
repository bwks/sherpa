use anyhow::Result;
use db::{Database, UsernameTaken};
use tracing::instrument;

use shared::data::DbUser;
use shared::util::normalize_username;
use shared::validators::{validate_password_strength, validate_username};

use crate::services::error::ServiceError;

/// Create a user account, the one path every transport takes: the username must be a valid,
/// unreserved username and the password must meet the strength rules, and a username another
/// user has is refused.
///
/// # Errors
///
/// - [`ServiceError::Invalid`] when the username or the password breaks its rule; the message
///   says which rule
/// - [`ServiceError::Conflict`] when the username is taken
/// - When the database cannot be written
#[instrument(skip(db, password), level = "debug")]
pub async fn create_user(
    db: &Database,
    username: &str,
    password: &str,
    is_admin: bool,
) -> Result<DbUser> {
    let username = normalize_username(username);
    validate_username(&username).map_err(|error| ServiceError::Invalid(format!("{error:#}")))?;
    validate_password_strength(password)
        .map_err(|error| ServiceError::Invalid(format!("{error:#}")))?;
    db::create_user(db, username, password, is_admin, vec![])
        .await
        .map_err(|error| match error.downcast_ref::<UsernameTaken>() {
            Some(taken) => ServiceError::Conflict(taken.to_string()).into(),
            None => error,
        })
}
