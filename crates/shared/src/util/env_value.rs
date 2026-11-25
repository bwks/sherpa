//! The one rule for values of `environment_variables` and env files: `${NAME}` is the value
//! `NAME` has in the client's environment, `\$` is a literal `$`, and any other `$` is refused.

use anyhow::Result;

use crate::SharedError;

/// One part of a value: text as written, or a reference to a client variable.
#[derive(Debug, PartialEq)]
pub(crate) enum ValuePart<'a> {
    Text(&'a str),
    Reference(&'a str),
}

/// Split `value` into its text and its `${NAME}` references, with each `\$` as a `$` of text.
///
/// # Errors
///
/// `SharedError::EnvValueUnescapedDollar` for a `$` that starts no `${NAME}` and has no `\`.
pub(crate) fn value_parts(value: &str) -> Result<Vec<ValuePart<'_>>> {
    let mut parts = Vec::new();
    let mut text_start = 0;
    let mut index = 0;
    while index < value.len() {
        let rest = &value[index..];
        if rest.starts_with("\\$") {
            parts.push(ValuePart::Text(&value[text_start..index]));
            text_start = index + 1;
            index += 2;
        } else if let Some(name) = reference_name(rest) {
            parts.push(ValuePart::Text(&value[text_start..index]));
            parts.push(ValuePart::Reference(name));
            index += name.len() + 3;
            text_start = index;
        } else if rest.starts_with('$') {
            return Err(SharedError::EnvValueUnescapedDollar.into());
        } else {
            index += rest.chars().next().map_or(1, char::len_utf8);
        }
    }
    parts.push(ValuePart::Text(&value[text_start..]));
    parts.retain(|part| *part != ValuePart::Text(""));
    Ok(parts)
}

/// Resolve `value` with variables from `lookup`: each `${NAME}` becomes the value of `NAME`,
/// and each `\$` a `$`.
///
/// # Errors
///
/// `SharedError::EnvValueUnescapedDollar` for any other `$`, and
/// `SharedError::EnvValueUnsetVariable` for a variable `lookup` does not have, naming the
/// variable and never a value.
pub fn resolve_env_value(value: &str, lookup: fn(&str) -> Option<String>) -> Result<String> {
    let mut resolved = String::with_capacity(value.len());
    for part in value_parts(value)? {
        match part {
            ValuePart::Text(text) => resolved.push_str(text),
            ValuePart::Reference(name) => {
                let variable = lookup(name).ok_or_else(|| SharedError::EnvValueUnsetVariable {
                    variable: name.to_owned(),
                })?;
                resolved.push_str(&variable);
            }
        }
    }
    Ok(resolved)
}

/// The name of the `${NAME}` reference `text` starts with, if it starts with one.
fn reference_name(text: &str) -> Option<&str> {
    let (name, _) = text.strip_prefix("${")?.split_once('}')?;
    if is_variable_name(name) {
        Some(name)
    } else {
        None
    }
}

/// Whether `name` is a variable name: a letter or `_`, then letters, digits and `_`.
fn is_variable_name(name: &str) -> bool {
    let mut characters = name.chars();
    let Some(first) = characters.next() else {
        return false;
    };
    if first != '_' && !first.is_ascii_alphabetic() {
        return false;
    }
    for character in characters {
        if character != '_' && !character.is_ascii_alphanumeric() {
            return false;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    const TOKEN: &str = "ghp_it's $ecret value";

    fn variables(name: &str) -> Option<String> {
        match name {
            "GH_TOKEN" => Some(TOKEN.to_owned()),
            "HOST" => Some("lab.example".to_owned()),
            "EMPTY" => Some(String::new()),
            _ => None,
        }
    }

    #[test]
    fn provisioner_env_014_a_reference_anywhere_in_a_value_becomes_the_client_value() {
        assert_eq!(resolve_env_value("${GH_TOKEN}", variables).unwrap(), TOKEN);
        assert_eq!(
            resolve_env_value("https://${HOST}:8443/${HOST}", variables).unwrap(),
            "https://lab.example:8443/lab.example"
        );
        assert_eq!(resolve_env_value("${EMPTY}", variables).unwrap(), "");
    }

    #[test]
    fn provisioner_env_015_a_backslash_dollar_is_a_literal_dollar_and_other_text_is_kept() {
        assert_eq!(resolve_env_value("\\$ecret", variables).unwrap(), "$ecret");
        assert_eq!(
            resolve_env_value("\\${GH_TOKEN}", variables).unwrap(),
            "${GH_TOKEN}"
        );
        assert_eq!(
            resolve_env_value(r"C:\tools a'b", variables).unwrap(),
            r"C:\tools a'b"
        );
    }

    #[test]
    fn provisioner_env_016_any_other_dollar_is_refused() {
        for value in [
            "$NAME",
            "$ecret",
            "$HOME/bin",
            "$1",
            "$",
            "pre$",
            "${",
            "${NAME",
            "${}",
            "${1A}",
            "${A-B}",
        ] {
            let error = resolve_env_value(value, variables).unwrap_err();
            assert_eq!(
                error.downcast_ref::<SharedError>(),
                Some(&SharedError::EnvValueUnescapedDollar),
                "{value}"
            );
        }
    }

    #[test]
    fn provisioner_env_017_a_reference_to_an_unset_variable_is_refused_naming_it_and_no_value() {
        let error = resolve_env_value("${GH_TOKEN}${MISSING}", variables).unwrap_err();
        assert_eq!(
            error.downcast_ref::<SharedError>(),
            Some(&SharedError::EnvValueUnsetVariable {
                variable: "MISSING".to_owned(),
            })
        );
        assert!(!format!("{error:#}").contains(TOKEN));
    }
}
