//! Token storage and management for the Sherpa CLI client.
//!
//! This module handles reading and writing JWT tokens to ~/.sherpa/token
//! with appropriate file permissions (0600).

use anyhow::{Context, Result};
use std::fs;
use std::path::PathBuf;

/// Get the path to the token file
///
/// Returns `~/.sherpa/token`
fn get_token_path() -> Result<PathBuf> {
    let home = std::env::var("HOME").context("HOME environment variable not set")?;
    let sherpa_dir = PathBuf::from(home).join(".sherpa");
    Ok(sherpa_dir.join("token"))
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
    let token_path = get_token_path()?;

    // Create ~/.sherpa directory if it doesn't exist
    if let Some(parent) = token_path.parent()
        && !parent.exists()
    {
        fs::create_dir_all(parent).context("Failed to create ~/.sherpa directory")?;
    }

    // Write token to file
    fs::write(&token_path, token).context("Failed to write token file")?;

    // Set file permissions to 0600 (owner read/write only)
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = fs::Permissions::from_mode(0o600);
        fs::set_permissions(&token_path, perms).context("Failed to set token file permissions")?;
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
    let token_path = get_token_path()?;

    if !token_path.exists() {
        anyhow::bail!("No token found. Please run: sherpa login");
    }

    let token = fs::read_to_string(&token_path).context("Failed to read token file")?;

    Ok(token.trim().to_string())
}

/// Delete the token file
///
/// # Errors
/// Returns an error if:
/// - HOME environment variable is not set  
/// - File deletion fails (if file exists)
pub fn delete_token() -> Result<()> {
    let token_path = get_token_path()?;

    if token_path.exists() {
        fs::remove_file(&token_path).context("Failed to delete token file")?;
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
    use std::env;

    #[test]
    fn test_get_token_path() {
        let path = get_token_path().expect("Failed to get token path");
        assert!(path.to_string_lossy().contains(".sherpa/token"));
    }

    #[test]
    #[ignore] // This test writes to the filesystem and uses unsafe
    fn test_save_and_load_token() {
        // Save original HOME for cleanup
        let original_home = env::var("HOME").ok();

        // Use a temp directory for testing
        let temp_dir = tempfile::tempdir().expect("Failed to create temp dir");
        // SAFETY: This test is single-threaded and restores the original value
        unsafe {
            env::set_var("HOME", temp_dir.path());
        }

        let test_token = "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.test.token";

        // Save token
        save_token(test_token).expect("Failed to save token");

        // Verify file exists
        assert!(token_exists());

        // Load token
        let loaded = load_token().expect("Failed to load token");
        assert_eq!(loaded, test_token);

        // Verify file permissions on Unix
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let token_path = get_token_path().expect("Failed to get token path");
            let metadata = fs::metadata(&token_path).expect("Failed to read metadata");
            let mode = metadata.permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }

        // Delete token
        delete_token().expect("Failed to delete token");
        assert!(!token_exists());

        // Restore original HOME
        // SAFETY: Restoring the original value
        if let Some(home) = original_home {
            unsafe {
                env::set_var("HOME", home);
            }
        }
    }

    #[test]
    #[ignore] // Uses unsafe env::set_var
    fn test_load_token_not_found() {
        // Save original HOME
        let original_home = env::var("HOME").ok();

        // Use a temp directory that doesn't have a token
        let temp_dir = tempfile::tempdir().expect("Failed to create temp dir");
        // SAFETY: This test is single-threaded and restores the original value
        unsafe {
            env::set_var("HOME", temp_dir.path());
        }

        let result = load_token();
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("No token found"));

        // Restore original HOME
        // SAFETY: Restoring the original value
        if let Some(home) = original_home {
            unsafe {
                env::set_var("HOME", home);
            }
        }
    }
}
