//! Token storage and management for the Sherpa CLI client.
//!
//! The login token is kept in `~/.sherpa/token`, readable only by the current user.

use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result, bail};

use crate::private_key::write_secret_file;

/// The file that holds the login token.
struct TokenFile {
    path: PathBuf,
}

impl TokenFile {
    /// The token file in the current user's home folder, `~/.sherpa/token`.
    fn in_home() -> Result<Self> {
        let home = dirs::home_dir().context("Could not determine home directory")?;
        Ok(Self {
            path: home.join(".sherpa").join("token"),
        })
    }

    /// Save `token`, readable only by the current user, creating the folder if needed.
    fn save(&self, token: &str) -> Result<()> {
        if let Some(parent) = self.path.parent()
            && !parent.exists()
        {
            fs::create_dir_all(parent)
                .with_context(|| format!("Failed to create {}", parent.display()))?;
        }
        write_secret_file(&self.path, token, "auth token")
    }

    /// The saved token.
    fn load(&self) -> Result<String> {
        if !self.path.exists() {
            bail!("No token found. Please run: sherpa login");
        }
        let token = fs::read_to_string(&self.path).context("Failed to read token file")?;
        Ok(token.trim().to_string())
    }

    /// Remove the saved token, if there is one.
    fn delete(&self) -> Result<()> {
        if self.path.exists() {
            fs::remove_file(&self.path).context("Failed to delete token file")?;
        }
        Ok(())
    }
}

/// Save a JWT token to `~/.sherpa/token`, readable only by the current user.
///
/// # Errors
///
/// Fails when the home directory is unknown, or the folder or the token file cannot be written.
pub fn save_token(token: &str) -> Result<()> {
    TokenFile::in_home()?.save(token)
}

/// Load the JWT token saved in `~/.sherpa/token`.
///
/// # Errors
///
/// Fails when the home directory is unknown, there is no saved token (the error says to run
/// `sherpa login`), or the file cannot be read.
pub fn load_token() -> Result<String> {
    TokenFile::in_home()?.load()
}

/// Delete the saved token.
///
/// # Errors
///
/// Fails when the home directory is unknown or the file cannot be removed.
pub fn delete_token() -> Result<()> {
    TokenFile::in_home()?.delete()
}

/// Whether a token is saved.
pub fn token_exists() -> bool {
    TokenFile::in_home().is_ok_and(|file| file.path.exists())
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;

    fn token_file(dir: &Path) -> TokenFile {
        TokenFile {
            path: dir.join(".sherpa").join("token"),
        }
    }

    #[test]
    fn the_token_file_is_in_the_home_folder() {
        let file = TokenFile::in_home().unwrap();
        assert!(
            file.path.ends_with(Path::new(".sherpa").join("token")),
            "{}",
            file.path.display()
        );
    }

    #[test]
    fn a_saved_token_loads_and_deletes() {
        let dir = tempfile::tempdir().unwrap();
        let file = token_file(dir.path());
        let token = "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.test.token";

        file.save(token).unwrap();
        assert_eq!(file.load().unwrap(), token);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(&file.path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }

        file.delete().unwrap();
        assert!(!file.path.exists());
        file.delete().unwrap();
    }

    #[test]
    fn a_missing_token_says_to_log_in() {
        let dir = tempfile::tempdir().unwrap();
        let error = token_file(dir.path()).load().unwrap_err();
        assert_eq!(
            error.to_string(),
            "No token found. Please run: sherpa login"
        );
    }
}
