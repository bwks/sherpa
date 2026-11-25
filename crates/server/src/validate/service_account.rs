use anyhow::{Result, bail};
use tracing::instrument;

use shared::konst::SHERPA_USERNAME;

use crate::services::error::ServiceError;

/// Check that the `sherpad` subcommand `command` (such as `init`) runs as the `sherpa` service
/// account, `account` being the name of the account it runs as. The files `init` writes must be
/// owned by the account sherpad runs as, and `sherpa.toml`, the database and its key are
/// readable by their owner only, so any other account, root included, would leave a server that
/// cannot read its own files.
#[instrument(level = "debug")]
pub fn validate_service_account(account: &str, command: &str) -> Result<()> {
    if account != SHERPA_USERNAME {
        bail!(ServiceError::NotServiceAccount {
            command: command.to_owned(),
            account: account.to_owned(),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn server_init_002_any_account_but_sherpa_is_refused_with_the_command_to_run() {
        assert!(validate_service_account("sherpa", "init").is_ok());
        for account in ["bradmin", "root", "Sherpa", "sherpa2", ""] {
            let error = validate_service_account(account, "init").unwrap_err();
            assert_eq!(
                error.downcast_ref::<ServiceError>(),
                Some(&ServiceError::NotServiceAccount {
                    command: "init".to_owned(),
                    account: account.to_owned(),
                }),
                "{account}"
            );
        }
    }
}
