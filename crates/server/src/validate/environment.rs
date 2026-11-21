use anyhow::{Result, bail};

/// Validate environment variable entries match `KEY=VALUE` format.
///
/// - KEY must be a valid identifier: alphanumeric + underscore, not starting with a digit.
/// - VALUE can be a literal or a `$VAR` reference (resolved at runtime by the client).
pub fn validate_environment_variables(entries: &[String], node_name: &str) -> Result<()> {
    for entry in entries {
        let Some((key, _value)) = entry.split_once('=') else {
            bail!(
                "Invalid environment variable '{entry}' for node '{node_name}': must be KEY=VALUE format"
            );
        };

        if key.is_empty() {
            bail!(
                "Invalid environment variable '{entry}' for node '{node_name}': KEY must not be empty"
            );
        }

        if key.starts_with(|c: char| c.is_ascii_digit()) {
            bail!(
                "Invalid environment variable key '{key}' for node '{node_name}': KEY must not start with a digit"
            );
        }

        if !key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            bail!(
                "Invalid environment variable key '{key}' for node '{node_name}': KEY must contain only alphanumeric characters and underscores"
            );
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn boot_settings_002_an_entry_with_a_name_as_its_key_is_taken() {
        let entries = vec![
            "EDITOR=vim".to_string(),
            "MY_VAR_123=hello world".to_string(),
            "GH_TOKEN=$GH_TOKEN".to_string(),
            "MY_VAR=".to_string(),
            "MY_VAR=key=value".to_string(),
        ];
        assert!(validate_environment_variables(&entries, "node1").is_ok());
    }

    #[test]
    fn boot_settings_003_an_entry_with_no_equals_an_empty_key_or_a_key_not_a_name_is_refused() {
        for (entry, named) in [
            ("NO_EQUALS", "variable 'NO_EQUALS'"),
            ("=value", "variable '=value'"),
            ("1BAD_KEY=value", "key '1BAD_KEY'"),
            ("BAD-KEY=value", "key 'BAD-KEY'"),
        ] {
            let message = validate_environment_variables(&[entry.to_string()], "node1")
                .unwrap_err()
                .to_string();
            assert!(
                message.contains(&format!("{named} for node 'node1'")),
                "{message}"
            );
        }
    }
}
