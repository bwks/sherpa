//! Env files for the ssh provisioner: `KEY=value` lines that login shells on the node load.
//! Values are data, never shell: each is resolved by the rule `environment_variables` in the
//! manifest follow, where `${NAME}` is the client's value of `NAME` and `\$` a literal `$`, and
//! is written single-quoted, so the node's shell expands nothing in it. A token on the client
//! reaches the node without being written to a file or sent to the server.

use std::env;
use std::path::Path;

use anyhow::{Context, Result};

use shared::util::resolve_env_value;

use super::remote::shell_quote;
use crate::error::ClientError;

/// The keyword a line may start with to export its variable, as in `export KEY=value`.
const EXPORT: &str = "export";

/// A `KEY=value` line: what comes before its value, written as is, its key and its value.
struct Assignment<'a> {
    prefix: &'a str,
    key: &'a str,
    value: &'a str,
}

/// The value of variable `name` in the client's environment, if it is set and is text.
pub fn client_variable(name: &str) -> Option<String> {
    env::var(name).ok()
}

/// Resolve the text of env file `path`, taking variables from `lookup`: each `KEY=value` line gets its
/// value resolved and single-quoted for the shell that loads the file. Comments, blank lines,
/// indentation, `export` and line endings are kept as written.
///
/// # Errors
///
/// A line that is not `KEY=value`, a comment or blank, naming the file and the line, or a value
/// with a `$` that is not `${NAME}` or `\$`, or naming a variable `lookup` does not have, naming
/// the file, the line and the key but never a value.
pub fn resolve_env_file(
    path: &Path,
    text: &str,
    lookup: fn(&str) -> Option<String>,
) -> Result<String> {
    let mut resolved = String::with_capacity(text.len());
    for (index, line) in text.split_inclusive('\n').enumerate() {
        let number = index + 1;
        let (content, ending) = split_line_ending(line);
        let trimmed = content.trim_start();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            resolved.push_str(content);
        } else {
            let assignment =
                assignment(content).ok_or_else(|| ClientError::EnvFileNotAssignment {
                    path: path.to_owned(),
                    line: number,
                })?;
            let value = resolve_env_value(assignment.value, lookup).with_context(|| {
                ClientError::EnvFileValue {
                    path: path.to_owned(),
                    line: number,
                    key: assignment.key.to_owned(),
                }
            })?;
            resolved.push_str(assignment.prefix);
            resolved.push_str(&shell_quote(&value));
        }
        resolved.push_str(ending);
    }
    Ok(resolved)
}

/// `line` split into its content and its line ending, `\r\n`, `\n` or none.
fn split_line_ending(line: &str) -> (&str, &str) {
    for ending in ["\r\n", "\n"] {
        if let Some(content) = line.strip_suffix(ending) {
            return (content, ending);
        }
    }
    (line, "")
}

/// The assignment `line` makes, if it is `KEY=value`, optionally indented or after `export`.
/// Spaces after the value are not part of it, nor is one pair of quotes around it.
fn assignment(line: &str) -> Option<Assignment<'_>> {
    let mut rest = line.trim_start();
    if let Some(after) = rest.strip_prefix(EXPORT)
        && after.starts_with(char::is_whitespace)
    {
        rest = after.trim_start();
    }
    let key_length = identifier_length(rest);
    if key_length == 0 || !rest[key_length..].starts_with('=') {
        return None;
    }
    let value_start = line.len() - rest.len() + key_length + 1;
    Some(Assignment {
        prefix: &line[..value_start],
        key: &rest[..key_length],
        value: unquoted(line[value_start..].trim_end()),
    })
}

/// `value` without one pair of matching single or double quotes around it.
fn unquoted(value: &str) -> &str {
    for quote in ['\'', '"'] {
        if let Some(inner) = value
            .strip_prefix(quote)
            .and_then(|rest| rest.strip_suffix(quote))
        {
            return inner;
        }
    }
    value
}

/// The length of the variable name `text` starts with: a letter or `_`, then letters, digits
/// and `_`. Zero if it starts with none.
fn identifier_length(text: &str) -> usize {
    let mut length = 0;
    for (index, character) in text.char_indices() {
        let allowed = character == '_'
            || character.is_ascii_alphabetic()
            || (index > 0 && character.is_ascii_digit());
        if !allowed {
            break;
        }
        length = index + character.len_utf8();
    }
    length
}

#[cfg(test)]
mod tests {
    use shared::SharedError;

    use super::*;

    const TOKEN: &str = "ghp_it's $ecret value";
    const QUOTED: &str = r"'ghp_it'\''s $ecret value'";

    fn variables(name: &str) -> Option<String> {
        match name {
            "GH_TOKEN" => Some(TOKEN.to_owned()),
            "HOST" => Some("lab.example".to_owned()),
            "EMPTY" => Some(String::new()),
            _ => None,
        }
    }

    /// `text` resolved as the env file `dev.env`.
    fn resolve(text: &str, lookup: fn(&str) -> Option<String>) -> Result<String> {
        resolve_env_file(Path::new("dev.env"), text, lookup)
    }

    #[test]
    fn provisioner_env_002_a_reference_becomes_the_client_value_quoted_whole_where_it_stands() {
        let text = "GH_TOKEN=${GH_TOKEN}\nURL=https://${HOST}:8443\nE=${EMPTY}\n";
        assert_eq!(
            resolve(text, variables).unwrap(),
            format!("GH_TOKEN={QUOTED}\nURL='https://lab.example:8443'\nE=''\n")
        );
    }

    #[test]
    fn provisioner_env_003_exported_indented_quoted_and_spaced_values_are_resolved_the_same_way() {
        let text = "export A=${GH_TOKEN}\n  B=${GH_TOKEN}  \nC='${GH_TOKEN}'\nD=\"${GH_TOKEN}\"\n";
        assert_eq!(
            resolve(text, variables).unwrap(),
            format!("export A={QUOTED}\n  B={QUOTED}\nC={QUOTED}\nD={QUOTED}\n")
        );
    }

    #[test]
    fn provisioner_env_004_a_backslash_dollar_is_a_literal_dollar_the_node_shell_does_not_expand() {
        assert_eq!(
            resolve(r"PASS=\$ecret", variables).unwrap(),
            "PASS='$ecret'"
        );
    }

    #[test]
    fn provisioner_env_005_plain_values_comments_and_blank_lines_are_kept_with_plain_values_quoted()
    {
        let text = "# token for gh\n\n  # indented\nEDITOR=vim\nMSG=a b\n";
        assert_eq!(
            resolve(text, variables).unwrap(),
            "# token for gh\n\n  # indented\nEDITOR='vim'\nMSG='a b'\n"
        );
    }

    #[test]
    fn provisioner_env_006_line_endings_are_kept() {
        let resolved = resolve("A=${GH_TOKEN}\r\nB=2", variables).unwrap();
        assert_eq!(resolved, format!("A={QUOTED}\r\nB='2'"));
    }

    #[test]
    fn provisioner_env_012_a_dollar_that_is_not_a_reference_or_escaped_is_refused_carrying_line_and_key()
     {
        for (text, key) in [
            ("A=1\nTOKEN=$GH_TOKEN\n", "TOKEN"),
            ("A=1\nPATH=$HOME/bin:$PATH\n", "PATH"),
            ("A=1\nARG=$1\n", "ARG"),
            ("A=1\nLONE=$\n", "LONE"),
        ] {
            let error = resolve(text, variables).unwrap_err();
            assert!(
                matches!(
                    error.downcast_ref::<ClientError>(),
                    Some(ClientError::EnvFileValue { path, line: 2, key: found })
                        if found == key && path == Path::new("dev.env")
                ),
                "{error:#}"
            );
            assert_eq!(
                error.root_cause().downcast_ref::<SharedError>(),
                Some(&SharedError::EnvValueUnescapedDollar),
                "{text}"
            );
        }
    }

    #[test]
    fn provisioner_env_013_a_line_that_is_not_an_assignment_comment_or_blank_is_refused_carrying_it()
     {
        for text in ["A=1\nset -a\n", "A=1\n1A=2\n", "A=1\nsource ~/.profile\n"] {
            let error = resolve(text, variables).unwrap_err();
            assert!(
                matches!(
                    error.downcast_ref::<ClientError>(),
                    Some(ClientError::EnvFileNotAssignment { path, line: 2 }) if path == Path::new("dev.env")
                ),
                "{error:#}"
            );
        }
    }

    #[test]
    fn provisioner_env_008_an_error_never_shows_a_value() {
        let error = resolve("A=${GH_TOKEN}\nB=${MISSING}\n", variables).unwrap_err();
        assert!(
            matches!(
                error.downcast_ref::<ClientError>(),
                Some(ClientError::EnvFileValue { line: 2, key, .. }) if key == "B"
            ),
            "{error:#}"
        );
        assert_eq!(
            error.root_cause().downcast_ref::<SharedError>(),
            Some(&SharedError::EnvValueUnsetVariable {
                variable: "MISSING".to_owned(),
            })
        );
        assert!(!format!("{error:#}").contains(TOKEN));
    }
}
