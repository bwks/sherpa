use anyhow::{Result, bail};

/// Validate environment variable entries match `KEY=VALUE` format.
///
/// - KEY must be a valid identifier: alphanumeric + underscore, not starting with a digit.
/// - VALUE can be a literal or a `$VAR` reference (resolved at runtime by the client).
pub fn validate_environment_variables(entries: &[String], node_name: &str) -> Result<()> {
    for entry in entries {
        let Some((key, _value)) = entry.split_once('=') else {
            bail!(
                "Invalid environment variable '{}' for node '{}': must be KEY=VALUE format",
                entry,
                node_name
            );
        };

        if key.is_empty() {
            bail!(
                "Invalid environment variable '{}' for node '{}': KEY must not be empty",
                entry,
                node_name
            );
        }

        if key.starts_with(|c: char| c.is_ascii_digit()) {
            bail!(
                "Invalid environment variable key '{}' for node '{}': KEY must not start with a digit",
                key,
                node_name
            );
        }

        if !key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            bail!(
                "Invalid environment variable key '{}' for node '{}': KEY must contain only alphanumeric characters and underscores",
                key,
                node_name
            );
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_valid_literal_entries() {
        let entries = vec![
            "EDITOR=vim".to_string(),
            "PATH=/usr/bin".to_string(),
            "MY_VAR_123=hello world".to_string(),
        ];
        assert!(validate_environment_variables(&entries, "node1").is_ok());
    }

    #[test]
    fn test_valid_env_ref_entries() {
        let entries = vec![
            "GH_TOKEN=$GH_TOKEN".to_string(),
            "API_KEY=$ANTHROPIC_API_KEY".to_string(),
        ];
        assert!(validate_environment_variables(&entries, "node1").is_ok());
    }

    #[test]
    fn test_empty_value_is_valid() {
        let entries = vec!["MY_VAR=".to_string()];
        assert!(validate_environment_variables(&entries, "node1").is_ok());
    }

    #[test]
    fn test_missing_equals_sign() {
        let entries = vec!["NO_EQUALS".to_string()];
        let result = validate_environment_variables(&entries, "node1");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("KEY=VALUE"));
    }

    #[test]
    fn test_empty_key() {
        let entries = vec!["=value".to_string()];
        let result = validate_environment_variables(&entries, "node1");
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("must not be empty")
        );
    }

    #[test]
    fn test_key_starts_with_digit() {
        let entries = vec!["1BAD_KEY=value".to_string()];
        let result = validate_environment_variables(&entries, "node1");
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("must not start with a digit")
        );
    }

    #[test]
    fn test_key_with_invalid_characters() {
        let entries = vec!["BAD-KEY=value".to_string()];
        let result = validate_environment_variables(&entries, "node1");
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("alphanumeric characters and underscores")
        );
    }

    #[test]
    fn test_key_with_dots() {
        let entries = vec!["BAD.KEY=value".to_string()];
        let result = validate_environment_variables(&entries, "node1");
        assert!(result.is_err());
    }

    #[test]
    fn test_empty_entries() {
        let entries: Vec<String> = vec![];
        assert!(validate_environment_variables(&entries, "node1").is_ok());
    }

    #[test]
    fn test_value_with_equals_sign() {
        let entries = vec!["MY_VAR=key=value".to_string()];
        assert!(validate_environment_variables(&entries, "node1").is_ok());
    }
}
