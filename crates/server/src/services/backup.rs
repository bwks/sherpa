use std::path::PathBuf;

use anyhow::{Context, Result};
use jiff::Timestamp;
use tracing::instrument;

use shared::data::BackupResponse;
use shared::konst::{
    SHERPA_DB_BACKUP_EXTENSION, SHERPA_DB_BACKUP_PATH, SHERPA_DB_BACKUP_PREFIX,
    SHERPA_DB_BACKUP_TIME_FORMAT,
};
use shared::util::create_dir;

use crate::auth::context::AuthContext;
use crate::daemon::state::AppState;

/// Copy the database to a new file in the server's backup directory while it stays in use.
///
/// Writes wait for the copy to finish. The file is named after the time it was taken, in UTC.
#[instrument(skip(state), fields(username = %auth_ctx.username))]
pub async fn backup_database(auth_ctx: &AuthContext, state: &AppState) -> Result<BackupResponse> {
    create_dir(SHERPA_DB_BACKUP_PATH)?;
    let path = backup_path(Timestamp::now());
    state.db.backup(&path).await?;
    let size_bytes = std::fs::metadata(&path)
        .context(format!("Failed to read the backup file {}", path.display()))?
        .len();
    tracing::info!(path = %path.display(), size_bytes, "Database backed up");
    Ok(BackupResponse {
        path: path.display().to_string(),
        size_bytes,
    })
}

fn backup_path(taken: Timestamp) -> PathBuf {
    let stamp = taken.strftime(SHERPA_DB_BACKUP_TIME_FORMAT).to_string();
    PathBuf::from(SHERPA_DB_BACKUP_PATH).join(format!(
        "{SHERPA_DB_BACKUP_PREFIX}{stamp}{SHERPA_DB_BACKUP_EXTENSION}"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backup_files_are_named_after_the_time_in_utc() {
        let taken: Timestamp = "2026-11-01T09:08:07.654Z".parse().unwrap();
        assert_eq!(
            backup_path(taken),
            PathBuf::from("/opt/sherpa/db/backups/sherpa-20261101T090807Z.sqlite")
        );
    }
}
