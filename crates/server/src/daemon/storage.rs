//! Permissions on the server directories that hold lab data and the database, and on the
//! server's SSH private key.
use std::fs;
use std::io::ErrorKind;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use anyhow::{Context, Result};
use nix::unistd::{Group, chown};
use tracing::instrument;

use shared::konst::{
    SHERPA_DB_BACKUP_DIR_MODE, SHERPA_DB_BACKUP_PATH, SHERPA_DB_DIR_MODE, SHERPA_DB_PATH,
    SHERPA_LABS_DIR_MODE, SHERPA_LABS_PATH, SHERPA_SSH_PRIVATE_KEY_MODE,
    SHERPA_SSH_PRIVATE_KEY_PATH, SHERPA_STORAGE_POOL_DIR_MODE, SHERPA_STORAGE_POOL_GROUP,
    SHERPA_STORAGE_POOL_PATH,
};

/// Restrict the directories that hold lab data and the database, so other server users cannot
/// read node configs, user data, ZTP media, VM disks, password hashes or database backups: lab
/// directories and the database to the `sherpa` user and group, backups to `sherpa`, and the
/// storage pool to `sherpa` and the QEMU group. Directory modes cover files created before
/// this, whatever their own mode. A failure is logged rather than fatal, so the server still
/// starts, for example when an administrator created a directory under another owner.
///
/// The server's SSH private key, when it exists, is set to its owner-only mode too, as servers
/// set up by older releases made it readable by the `sherpa` group.
#[instrument]
pub fn secure_storage() {
    let dirs = [
        (SHERPA_LABS_PATH, SHERPA_LABS_DIR_MODE, None),
        (SHERPA_DB_PATH, SHERPA_DB_DIR_MODE, None),
        (SHERPA_DB_BACKUP_PATH, SHERPA_DB_BACKUP_DIR_MODE, None),
        (
            SHERPA_STORAGE_POOL_PATH,
            SHERPA_STORAGE_POOL_DIR_MODE,
            Some(SHERPA_STORAGE_POOL_GROUP),
        ),
    ];
    for (path, mode, group) in dirs {
        match restrict_dir(Path::new(path), mode, group) {
            Ok(()) => {
                tracing::info!(path = %path, mode = %format!("{mode:o}"), "Storage restricted");
            }
            Err(error) => tracing::warn!(
                path = %path,
                error = %format!("{error:#}"),
                "Cannot restrict storage; other server users may be able to read lab data or the database"
            ),
        }
    }
    match restrict_file(
        Path::new(SHERPA_SSH_PRIVATE_KEY_PATH),
        SHERPA_SSH_PRIVATE_KEY_MODE,
    ) {
        Ok(true) => tracing::info!(
            path = %SHERPA_SSH_PRIVATE_KEY_PATH,
            mode = %format!("{SHERPA_SSH_PRIVATE_KEY_MODE:o}"),
            "Server SSH key restricted"
        ),
        Ok(false) => tracing::debug!(
            path = %SHERPA_SSH_PRIVATE_KEY_PATH,
            "No server SSH key yet"
        ),
        Err(error) => tracing::warn!(
            path = %SHERPA_SSH_PRIVATE_KEY_PATH,
            error = %format!("{error:#}"),
            "Cannot restrict the server SSH key; other server users may be able to read it"
        ),
    }
}

/// Set an existing file at `path` to `mode`. Returns `true` when the file existed and was
/// restricted, and `false` when it is missing; a missing file is left missing, as
/// `sherpad init` creates it.
#[instrument(level = "debug")]
pub fn restrict_file(path: &Path, mode: u32) -> Result<bool> {
    match fs::metadata(path) {
        Ok(_) => {
            fs::set_permissions(path, fs::Permissions::from_mode(mode))
                .with_context(|| format!("Cannot set mode {mode:o} on {}", path.display()))?;
            Ok(true)
        }
        Err(error) if error.kind() == ErrorKind::NotFound => {
            tracing::debug!(path = %path.display(), "No file to restrict");
            Ok(false)
        }
        Err(error) => {
            Err(error).with_context(|| format!("Cannot read the metadata of {}", path.display()))
        }
    }
}

/// Create `path` if it is missing and restrict it to `mode`, owned by `group` when given.
#[instrument(level = "debug")]
pub fn restrict_dir(path: &Path, mode: u32, group: Option<&str>) -> Result<()> {
    fs::create_dir_all(path).with_context(|| format!("Cannot create {}", path.display()))?;
    if let Some(name) = group {
        let group = Group::from_name(name)
            .with_context(|| format!("Cannot look up group '{name}'"))?
            .with_context(|| format!("Group '{name}' does not exist"))?;
        chown(path, None, Some(group.gid))
            .with_context(|| format!("Cannot give {} to group '{name}'", path.display()))?;
    }
    fs::set_permissions(path, fs::Permissions::from_mode(mode))
        .with_context(|| format!("Cannot set mode {mode:o} on {}", path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::os::unix::fs::{MetadataExt, PermissionsExt};

    use nix::unistd::{Group, getgid};
    use tempfile::tempdir;

    #[test]
    fn restrict_dir_creates_a_missing_dir_with_the_mode() {
        let root = tempdir().unwrap();
        let dir = root.path().join("labs");
        restrict_dir(&dir, 0o2750, None).unwrap();
        let metadata = fs::metadata(&dir).unwrap();
        assert!(metadata.is_dir());
        assert_eq!(metadata.permissions().mode() & 0o7777, 0o2750);
    }

    #[test]
    fn restrict_dir_tightens_an_existing_dir_and_sets_its_group() {
        let root = tempdir().unwrap();
        let dir = root.path().join("pool");
        fs::create_dir(&dir).unwrap();
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o2775)).unwrap();
        let group = Group::from_gid(getgid()).unwrap().unwrap();
        restrict_dir(&dir, 0o2750, Some(&group.name)).unwrap();
        let metadata = fs::metadata(&dir).unwrap();
        assert_eq!(metadata.permissions().mode() & 0o7777, 0o2750);
        assert_eq!(metadata.gid(), group.gid.as_raw());
    }

    #[test]
    fn restrict_file_takes_group_read_off_an_existing_server_key() {
        let root = tempdir().unwrap();
        let key = root.path().join("sherpa_ssh_key");
        fs::write(&key, "key").unwrap();
        fs::set_permissions(&key, fs::Permissions::from_mode(0o640)).unwrap();
        assert!(restrict_file(&key, SHERPA_SSH_PRIVATE_KEY_MODE).unwrap());
        let mode = fs::metadata(&key).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        assert_eq!(fs::read_to_string(&key).unwrap(), "key");
    }

    #[test]
    fn restrict_file_leaves_a_missing_file_missing() {
        let root = tempdir().unwrap();
        let key = root.path().join("sherpa_ssh_key");
        assert!(!restrict_file(&key, SHERPA_SSH_PRIVATE_KEY_MODE).unwrap());
        assert!(!key.exists());
    }

    #[test]
    fn restrict_dir_rejects_an_unknown_group() {
        let root = tempdir().unwrap();
        let dir = root.path().join("pool");
        assert!(restrict_dir(&dir, 0o2750, Some("sherpa-no-such-group")).is_err());
    }
}
