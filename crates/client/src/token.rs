//! Token storage and management for the Sherpa CLI client.
//!
//! The login token is kept in `~/.sherpa/token`, readable only by the current user.

use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result, ensure};
use base64::{Engine as _, engine::general_purpose};

use shared::auth::jwt::Claims;

use crate::error::ClientError;
use crate::secret_file::write_secret_file;

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
        ensure!(self.path.exists(), ClientError::NoToken);
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

/// The username a login `token` is for: its `sub` claim. The signature is not checked: this only
/// names the user to the CLI, and the server checks the token on every call.
///
/// # Errors
///
/// Fails when `token` is not a JWT or its claims do not parse.
pub fn username_from_token(token: &str) -> Result<String> {
    let payload = token.split('.').nth(1).context(ClientError::TokenNotJwt)?;
    let payload = general_purpose::URL_SAFE_NO_PAD
        .decode(payload)
        .context("Failed to decode the login token")?;
    let claims: Claims = serde_json::from_slice(&payload)
        .context("Failed to parse the login token; run: sherpa login")?;
    Ok(claims.sub)
}

/// Whether a token is saved.
pub fn token_exists() -> bool {
    TokenFile::in_home().is_ok_and(|file| file.path.exists())
}

#[cfg(test)]
mod tests {
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;
    use std::path::Path;

    use super::*;

    fn token_file(dir: &Path) -> TokenFile {
        TokenFile {
            path: dir.join(".sherpa").join("token"),
        }
    }

    #[test]
    fn user_login_012_a_token_is_saved_private_and_a_missing_one_says_to_log_in() {
        let dir = tempfile::tempdir().unwrap();
        let file = token_file(dir.path());
        file.save("eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.test.token")
            .unwrap();
        #[cfg(unix)]
        {
            let mode = fs::metadata(&file.path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }

        let dir = tempfile::tempdir().unwrap();
        let error = token_file(dir.path()).load().unwrap_err();
        assert!(matches!(
            error.downcast_ref::<ClientError>(),
            Some(ClientError::NoToken)
        ));
    }

    #[test]
    fn user_login_015_the_username_is_read_from_the_token_and_a_non_jwt_is_refused() {
        let claims = br#"{"sub":"dev","sid":"s","exp":1,"iat":1,"is_admin":false,"x":"??>>~~"}"#;
        let payload = general_purpose::URL_SAFE_NO_PAD.encode(claims);
        assert!(payload.contains(['-', '_']), "{payload}");
        let token = format!("header.{payload}.signature");
        assert_eq!(username_from_token(&token).unwrap(), "dev");

        let error = username_from_token("not-a-jwt").unwrap_err();
        assert!(matches!(
            error.downcast_ref::<ClientError>(),
            Some(ClientError::TokenNotJwt)
        ));
    }
}
