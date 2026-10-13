//! Token storage and management for the Sherpa CLI client.
//!
//! This module handles reading and writing JWT tokens to ~/.sherpa/token
//! with appropriate file permissions (0600).

use anyhow::{Context, Result};
use std::fs;
use std::path::{Path, PathBuf};

/// Get the path to the token file
///
/// Returns `~/.sherpa/token`
fn get_token_path() -> Result<PathBuf> {
    let home = dirs::home_dir().context("Could not determine home directory")?;
    Ok(token_path_in(&home))
}

/// Path of the token file under the given home directory
fn token_path_in(home: &Path) -> PathBuf {
    home.join(".sherpa").join("token")
}

/// Save a JWT token to ~/.sherpa/token with 0600 permissions
///
/// Creates the ~/.sherpa directory if it doesn't exist.
///
/// # Arguments
/// * `token` - The JWT token string to save
///
/// # Errors
/// Returns an error if:
/// - HOME environment variable is not set
/// - Directory creation fails
/// - File write fails
/// - Permission setting fails (on Unix)
pub fn save_token(token: &str) -> Result<()> {
    save_token_to(&get_token_path()?, token)
}

fn save_token_to(token_path: &Path, token: &str) -> Result<()> {
    // Create ~/.sherpa directory if it doesn't exist
    if let Some(parent) = token_path.parent()
        && !parent.exists()
    {
        fs::create_dir_all(parent).context("Failed to create ~/.sherpa directory")?;
    }

    // Write token to file
    fs::write(token_path, token).context("Failed to write token file")?;

    // Set file permissions to 0600 (owner read/write only)
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = fs::Permissions::from_mode(0o600);
        fs::set_permissions(token_path, perms).context("Failed to set token file permissions")?;
    }

    Ok(())
}

/// Load a JWT token from ~/.sherpa/token
///
/// # Returns
/// The token string if the file exists and is readable
///
/// # Errors
/// Returns an error if:
/// - HOME environment variable is not set
/// - Token file doesn't exist
/// - File read fails
pub fn load_token() -> Result<String> {
    load_token_from(&get_token_path()?)
}

fn load_token_from(token_path: &Path) -> Result<String> {
    if !token_path.exists() {
        anyhow::bail!("No token found. Please run: sherpa login");
    }

    let token = fs::read_to_string(token_path).context("Failed to read token file")?;

    Ok(token.trim().to_string())
}

/// Delete the token file
///
/// # Errors
/// Returns an error if:
/// - HOME environment variable is not set  
/// - File deletion fails (if file exists)
pub fn delete_token() -> Result<()> {
    delete_token_at(&get_token_path()?)
}

fn delete_token_at(token_path: &Path) -> Result<()> {
    if token_path.exists() {
        fs::remove_file(token_path).context("Failed to delete token file")?;
    }

    Ok(())
}

/// Check if a token file exists
pub fn token_exists() -> bool {
    get_token_path().map(|path| path.exists()).unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_token_path() {
        let path = get_token_path().expect("Failed to get token path");
        assert!(path.to_string_lossy().contains(".sherpa/token"));
    }

    #[test]
    fn test_token_path_in_is_under_dot_sherpa() {
        let path = token_path_in(Path::new("/home/example"));
        assert_eq!(path, PathBuf::from("/home/example/.sherpa/token"));
    }

    #[test]
    fn test_save_and_load_token() {
        let temp_dir = tempfile::tempdir().expect("Failed to create temp dir");
        let token_path = token_path_in(temp_dir.path());
        let test_token = "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.test.token";

        save_token_to(&token_path, test_token).expect("Failed to save token");
        assert!(token_path.exists());

        let loaded = load_token_from(&token_path).expect("Failed to load token");
        assert_eq!(loaded, test_token);

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let metadata = fs::metadata(&token_path).expect("Failed to read metadata");
            let mode = metadata.permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }

        delete_token_at(&token_path).expect("Failed to delete token");
        assert!(!token_path.exists());
    }

    #[test]
    fn test_load_token_not_found() {
        let temp_dir = tempfile::tempdir().expect("Failed to create temp dir");
        let token_path = token_path_in(temp_dir.path());

        let result = load_token_from(&token_path);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("No token found"));
    }

    #[test]
    fn test_delete_token_when_missing_is_ok() {
        let temp_dir = tempfile::tempdir().expect("Failed to create temp dir");
        let token_path = token_path_in(temp_dir.path());

        delete_token_at(&token_path).expect("Deleting a missing token should succeed");
    }
}
