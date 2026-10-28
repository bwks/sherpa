//! Env files for the ssh provisioner: `KEY=value` lines that login shells on the node load.
//! A value that is exactly one variable reference, `KEY=$NAME` or `KEY=${NAME}`, is resolved
//! from the client's environment, as `environment_variables` in the manifest are, so a token
//! on the client reaches the node without being written to a file or sent to the server.

use std::env;

use anyhow::{Result, bail};

use super::remote::shell_quote;

/// The keyword a line may start with to export its variable, as in `export KEY=value`.
const EXPORT: &str = "export";

/// A line whose value is one variable reference.
struct Reference<'a> {
    /// The line up to and including `=`.
    assignment: &'a str,
    /// The variable referred to.
    name: &'a str,
}

/// The value of variable `name` in the client's environment, if it is set and is text.
pub fn client_variable(name: &str) -> Option<String> {
    env::var(name).ok()
}

/// Resolve the variable references in env file `text`, taking values from `lookup`: a line
/// whose value is exactly `$NAME` or `${NAME}` gets the value, single-quoted for the shell
/// that loads the file. Other lines, comments and line endings are kept as written, so a
/// value such as `$HOME/bin:$PATH` is still expanded on the node.
///
/// # Errors
///
/// A reference to a variable `lookup` does not have, naming the line and the variable but
/// never a value.
pub fn resolve_env_file(text: &str, lookup: fn(&str) -> Option<String>) -> Result<String> {
    let mut resolved = String::with_capacity(text.len());
    for (index, line) in text.split_inclusive('\n').enumerate() {
        let (content, ending) = split_line_ending(line);
        match reference(content) {
            Some(reference) => {
                let Some(value) = lookup(reference.name) else {
                    bail!(
                        "line {}: {} is not set in this environment",
                        index + 1,
                        reference.name
                    );
                };
                resolved.push_str(reference.assignment);
                resolved.push_str(&shell_quote(&value));
            }
            None => resolved.push_str(content),
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

/// The variable reference `line` assigns, if its value is exactly one: `KEY=$NAME` or
/// `KEY=${NAME}`, optionally indented, after `export`, or followed by spaces.
fn reference(line: &str) -> Option<Reference<'_>> {
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
    let name = variable_name(line[value_start..].trim_end())?;
    Some(Reference {
        assignment: &line[..value_start],
        name,
    })
}

/// The variable `value` refers to, if it is exactly `$NAME` or `${NAME}`.
fn variable_name(value: &str) -> Option<&str> {
    let reference = value.strip_prefix('$')?;
    let name = match reference.strip_prefix('{') {
        Some(braced) => braced.strip_suffix('}')?,
        None => reference,
    };
    if !name.is_empty() && identifier_length(name) == name.len() {
        Some(name)
    } else {
        None
    }
}

/// The length of the shell variable name `text` starts with: a letter or `_`, then letters,
/// digits and `_`. Zero if it starts with none.
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
    use super::*;

    const TOKEN: &str = "ghp_it's $ecret value";

    fn variables(name: &str) -> Option<String> {
        match name {
            "GH_TOKEN" => Some(TOKEN.to_owned()),
            "EMPTY" => Some(String::new()),
            _ => None,
        }
    }

    #[test]
    fn a_whole_value_reference_is_resolved_and_quoted() {
        let resolved = resolve_env_file("GH_TOKEN=$GH_TOKEN\n", variables).unwrap();
        assert_eq!(resolved, "GH_TOKEN='ghp_it'\\''s $ecret value'\n");
    }

    #[test]
    fn braced_and_exported_references_are_resolved() {
        let text = "TOKEN=${GH_TOKEN}\nexport OTHER=$GH_TOKEN\n  SPACED=$GH_TOKEN  \n";
        let resolved = resolve_env_file(text, variables).unwrap();
        let quoted = "'ghp_it'\\''s $ecret value'";
        assert_eq!(
            resolved,
            format!("TOKEN={quoted}\nexport OTHER={quoted}\n  SPACED={quoted}\n")
        );
    }

    #[test]
    fn a_set_but_empty_variable_resolves_to_an_empty_value() {
        assert_eq!(resolve_env_file("E=$EMPTY", variables).unwrap(), "E=''");
    }

    #[test]
    fn other_lines_are_kept_as_written() {
        let text = "# token for gh\n\nPLAIN=value\nQUOTED='a b'\nPATH=$HOME/bin:$PATH\n\
                    MIXED=pre$GH_TOKEN\nNOT_A_NAME=$1\nDOLLAR=$\nCRLF=x\r\n";
        assert_eq!(resolve_env_file(text, variables).unwrap(), text);
    }

    #[test]
    fn line_endings_are_kept() {
        let resolved = resolve_env_file("A=$GH_TOKEN\r\nB=2", variables).unwrap();
        assert_eq!(resolved, "A='ghp_it'\\''s $ecret value'\r\nB=2");
    }

    #[test]
    fn an_unset_variable_fails_naming_it_and_the_line() {
        let error = resolve_env_file("A=1\nTOKEN=$MISSING_TOKEN\n", variables)
            .unwrap_err()
            .to_string();
        assert!(error.contains("MISSING_TOKEN"), "{error}");
        assert!(error.contains("line 2"), "{error}");
    }

    #[test]
    fn errors_never_show_values() {
        let error = resolve_env_file("A=$GH_TOKEN\nB=$MISSING\n", variables)
            .unwrap_err()
            .to_string();
        assert!(!error.contains("ecret"), "{error}");
    }
}
