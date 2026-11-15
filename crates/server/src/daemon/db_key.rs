//! The database key: the 32-byte key that seals the database's secret values.
//!
//! It is a file beside the JWT secret, mode 0600 in the 0700 secrets directory
//! (`SHERPA_DB_KEY_PATH`), made at first start the way the JWT secret is. It is never in the
//! database or the database directory, so a database file or backup alone holds only
//! ciphertext; restoring a backup needs the key it was made with.

use std::fs;
use std::io::ErrorKind;
use std::path::Path;

use anyhow::{Context, Result, bail};
use db::SecretKey;
use tracing::instrument;

use crate::tls::generator::write_file_with_permissions;

/// The database key's file mode.
const KEY_FILE_MODE: u32 = 0o600;

/// Load the database key from `path`, or make one there when the file does not exist and the
/// database has none (`registered` is false: no value has been sealed with a key yet).
///
/// # Errors
///
/// Fails when the file is missing but the database's values are sealed with a key, which a new
/// key would not open; when the file is not 32 bytes long; and when it cannot be read or written.
/// A damaged or missing key is never replaced: the error names the file to restore.
#[instrument(fields(path = %path.display()))]
pub fn load_or_create_db_key(path: &Path, registered: bool) -> Result<SecretKey> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == ErrorKind::NotFound => {
            if registered {
                bail!(
                    "The database key {} is missing, and the database's secret values are \
                     sealed with it. Restore the key file that was backed up with this database; \
                     a new key cannot open them",
                    path.display()
                );
            }
            let key = SecretKey::generate();
            write_file_with_permissions(path, key.as_bytes(), KEY_FILE_MODE)
                .context("Failed to write the database key")?;
            tracing::info!("Generated a new database key");
            return Ok(key);
        }
        Err(error) => {
            return Err(error)
                .with_context(|| format!("Failed to read the database key {}", path.display()));
        }
    };
    let key = SecretKey::from_bytes(&bytes)
        .with_context(|| format!("The database key {} is damaged", path.display()))?;
    tracing::info!("Loaded the database key");
    Ok(key)
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt;

    use super::*;

    #[test]
    fn a_key_is_made_0600_on_first_start_and_loaded_after() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("db.key");
        let generated = load_or_create_db_key(&path, false).unwrap();
        let file_mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(file_mode, KEY_FILE_MODE);
        assert_eq!(fs::read(&path).unwrap(), generated.as_bytes());
        let loaded = load_or_create_db_key(&path, true).unwrap();
        assert_eq!(loaded.as_bytes(), generated.as_bytes());
    }

    #[test]
    fn a_missing_key_for_sealed_values_is_refused_and_not_made() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("db.key");
        let error = load_or_create_db_key(&path, true).unwrap_err();
        let message = format!("{error:#}");
        assert!(message.contains("is missing"), "{message}");
        assert!(message.contains(&path.display().to_string()), "{message}");
        assert!(!path.exists());
    }

    #[test]
    fn a_damaged_key_is_refused_and_kept() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("db.key");
        fs::write(&path, b"short").unwrap();
        let error = load_or_create_db_key(&path, false).unwrap_err();
        let message = format!("{error:#}");
        assert!(message.contains("damaged"), "{message}");
        assert_eq!(fs::read(&path).unwrap(), b"short");
    }
}
