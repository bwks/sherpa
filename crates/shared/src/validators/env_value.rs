use anyhow::Result;

use crate::util::value_parts;

/// Check that every `$` in an `environment_variables` or env file value is a `${NAME}`
/// reference or a `\$` literal.
///
/// # Errors
///
/// `SharedError::EnvValueUnescapedDollar` for any other `$`.
pub fn validate_env_value(value: &str) -> Result<()> {
    value_parts(value)?;
    Ok(())
}
