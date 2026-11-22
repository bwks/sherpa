//! The host account a `sherpad` subcommand runs as.

use anyhow::{Context, Result};
use nix::unistd::{User, geteuid};
use tracing::instrument;

/// The name of the host account this process runs as, from its effective uid.
///
/// # Errors
///
/// Fails when the uid cannot be looked up or has no account.
#[instrument(level = "debug")]
pub fn running_account_name() -> Result<String> {
    let uid = geteuid();
    let account = User::from_uid(uid)
        .with_context(|| format!("Failed to look up the account of uid {uid}"))?
        .with_context(|| format!("uid {uid} has no account"))?;
    Ok(account.name)
}
