use anyhow::{Result, ensure};
use tracing::instrument;

#[instrument(skip(key), level = "debug")]
pub fn validate_tailscale_key(key: &str) -> Result<()> {
    ensure!(
        key.starts_with("tskey-auth-")
            && key.len() > 11
            && key.len() <= 512
            && key
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_'),
        "Tailscale enrollment requires a nonempty Tailscale auth key; OAuth credentials and whitespace are not supported"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalid_key_errors_do_not_echo_secrets() {
        for key in [
            "",
            "tskey-auth-",
            "tskey-auth-secret\n",
            "tskey-client-secret",
            "secret",
        ] {
            let error = validate_tailscale_key(key).unwrap_err().to_string();
            if !key.is_empty() {
                assert!(!error.contains(key));
            }
        }
        assert!(validate_tailscale_key("tskey-auth-example123456789").is_ok());
    }
}
