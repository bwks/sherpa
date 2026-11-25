use anyhow::{Result, ensure};
use tracing::instrument;

use crate::SharedError;

/// Check that `key` looks like a Tailscale auth key (`tskey-auth-...`), not an OAuth
/// credential.
///
/// # Errors
///
/// Fails when the key is not a Tailscale auth key.
#[instrument(skip(key), level = "debug")]
pub fn validate_tailscale_key(key: &str) -> Result<()> {
    ensure!(
        key.starts_with("tskey-auth-")
            && key.len() > 11
            && key.len() <= 512
            && key
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_'),
        SharedError::MalformedTailscaleKey
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tailnet_key_005_a_malformed_key_is_refused_and_the_error_carries_nothing_of_it() {
        for key in [
            "",
            "tskey-auth-",
            "tskey-auth-secret\n",
            "tskey-auth-sec ret",
            "tskey-client-secret",
            "secret",
        ] {
            let error = validate_tailscale_key(key).unwrap_err();
            assert_eq!(
                error.downcast_ref::<SharedError>(),
                Some(&SharedError::MalformedTailscaleKey),
                "{key:?}"
            );
        }
        assert!(validate_tailscale_key("tskey-auth-example123456789").is_ok());
    }
}
