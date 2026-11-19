use anyhow::{Result, bail};
use tracing::instrument;

use shared::konst::SHERPA_USERNAME;

/// Check that `sherpad init` runs as the `sherpa` service account, `account` being the name of
/// the account it runs as. The files `init` writes must be owned by the account sherpad runs as,
/// and `sherpa.toml` is readable by its owner only, so any other account, root included, would
/// leave a server that cannot read its own config.
#[instrument(level = "debug")]
pub fn validate_init_account(account: &str) -> Result<()> {
    if account != SHERPA_USERNAME {
        bail!(
            "sherpad init must run as the '{SHERPA_USERNAME}' service account, not '{account}'. \
             Nothing was changed. Run it as: sudo -u {SHERPA_USERNAME} -H sherpad init, with the \
             same options"
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_the_service_account() {
        assert!(validate_init_account("sherpa").is_ok());
    }

    #[test]
    fn refuses_any_other_account_with_the_command_to_run() {
        for account in ["bradmin", "root", "Sherpa", "sherpa2", ""] {
            let error = validate_init_account(account).unwrap_err().to_string();
            assert!(
                error.contains(&format!("not '{account}'")),
                "{account}: {error}"
            );
            assert!(
                error.contains("sudo -u sherpa -H sherpad init"),
                "{account}: {error}"
            );
        }
    }
}
