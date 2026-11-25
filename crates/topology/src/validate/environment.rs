use anyhow::{Result, bail};

use crate::{Node, TopologyError};

/// Check every node's `environment_variables` entries are `KEY=VALUE`, where KEY is a name of
/// ASCII letters, digits and `_` that does not start with a digit. The node's first-boot config
/// writes KEY into a shell file. VALUE can be a literal or a `$VAR` reference, which the client
/// resolves.
///
/// # Errors
///
/// Fails on the first entry with no `=`, an empty key or a key that is not a name, naming the
/// entry or the key and the node.
pub fn validate_environment_variables(nodes: &[Node]) -> Result<()> {
    for node in nodes {
        let node_name = &node.name;
        for entry in node.environment_variables.iter().flatten() {
            let Some((key, _value)) = entry.split_once('=') else {
                bail!(TopologyError::EnvironmentVariableNotKeyValue {
                    entry: entry.clone(),
                    node: node_name.clone(),
                });
            };

            if key.is_empty() {
                bail!(TopologyError::EnvironmentVariableEmptyKey {
                    entry: entry.clone(),
                    node: node_name.clone(),
                });
            }

            if key.starts_with(|c: char| c.is_ascii_digit()) {
                bail!(TopologyError::EnvironmentVariableKeyStartsWithDigit {
                    key: key.to_owned(),
                    node: node_name.clone(),
                });
            }

            if !key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
                bail!(TopologyError::EnvironmentVariableKeyNotAName {
                    key: key.to_owned(),
                    node: node_name.clone(),
                });
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(entries: &[&str]) -> Node {
        Node {
            name: "node1".to_string(),
            environment_variables: Some(entries.iter().map(ToString::to_string).collect()),
            ..Node::default()
        }
    }

    #[test]
    fn boot_settings_002_an_entry_with_a_name_as_its_key_is_taken() {
        let entries = [
            "EDITOR=vim",
            "MY_VAR_123=hello world",
            "GH_TOKEN=$GH_TOKEN",
            "MY_VAR=",
            "MY_VAR=key=value",
        ];
        assert!(validate_environment_variables(&[node(&entries)]).is_ok());
    }

    #[test]
    fn boot_settings_003_an_entry_with_no_equals_an_empty_key_or_a_key_not_a_name_is_refused() {
        let node1 = || "node1".to_owned();
        for (entry, refusal) in [
            (
                "NO_EQUALS",
                TopologyError::EnvironmentVariableNotKeyValue {
                    entry: "NO_EQUALS".to_owned(),
                    node: node1(),
                },
            ),
            (
                "=value",
                TopologyError::EnvironmentVariableEmptyKey {
                    entry: "=value".to_owned(),
                    node: node1(),
                },
            ),
            (
                "1BAD_KEY=value",
                TopologyError::EnvironmentVariableKeyStartsWithDigit {
                    key: "1BAD_KEY".to_owned(),
                    node: node1(),
                },
            ),
            (
                "BAD-KEY=value",
                TopologyError::EnvironmentVariableKeyNotAName {
                    key: "BAD-KEY".to_owned(),
                    node: node1(),
                },
            ),
        ] {
            let error = validate_environment_variables(&[node(&[entry])]).unwrap_err();
            assert_eq!(error.downcast_ref::<TopologyError>(), Some(&refusal));
        }
    }
}
