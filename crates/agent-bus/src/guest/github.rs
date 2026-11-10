//! The GitHub write token: given to the guest agent on its stdin, held in its memory only, and
//! set in the environment of a `--github` task's Claude Code and nowhere else.
//!
//! Every task has the guest's read-only token, `GH_TOKEN` from `~/.env.d/github.env`, which the
//! guest agent inherits from its login shell. The write token never goes into the agent's own
//! environment (a task can read `/proc/<agent pid>/environ`), a file, a log or the bus: its
//! `Debug` is redacted, it has no `Display`, and every text the agent sends or records has it
//! replaced by `REDACTED`. It reaches the agent on stdin with its other secrets (`secrets`).

use std::ffi::OsStr;
use std::fmt;

use anyhow::{Result, bail};

/// The variable `gh`, and through `gh auth git-credential` git, take the token from.
pub(super) const GH_TOKEN_VARIABLE: &str = "GH_TOKEN";
/// Other GitHub token variables, removed from every run of Claude Code and `gh`: `gh` takes
/// `GH_TOKEN` first, and only `GH_TOKEN` is set on purpose, read-only for every task or the write
/// token for a `--github` one.
pub(super) const OTHER_GITHUB_TOKEN_VARIABLES: [&str; 2] = ["GITHUB_TOKEN", "GH_ENTERPRISE_TOKEN"];
/// What the write token is replaced with in text the agent sends or records.
pub(super) const REDACTED: &str = "[GitHub write token]";
/// The prefixes of GitHub's tokens: fine-grained, classic, OAuth, user-to-server, server-to-server
/// and refresh.
const TOKEN_PREFIXES: [&str; 6] = ["github_pat_", "ghp_", "gho_", "ghu_", "ghs_", "ghr_"];

/// The GitHub token that can write: commit, push and open pull requests. Only `--github` tasks
/// get it.
pub struct WriteToken(String);

/// The GitHub token a Claude Code run gets.
#[derive(Debug, Clone, Copy)]
pub(super) enum Access<'a> {
    /// The guest's read-only token, inherited from the agent's environment.
    ReadOnly,
    /// The write token, for a `--github` task.
    Write(&'a WriteToken),
}

impl fmt::Debug for WriteToken {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("WriteToken(<redacted>)")
    }
}

impl WriteToken {
    /// A token from one line; `None` for an empty line. Errors never contain the line.
    ///
    /// # Errors
    ///
    /// Fails when the line is not a GitHub token.
    pub fn parse(line: &str) -> Result<Option<Self>> {
        let line = line.trim_end_matches(['\n', '\r']);
        if line.is_empty() {
            return Ok(None);
        }
        if line.starts_with("SU") {
            bail!(
                "The GitHub write token line holds an NKey seed: start the agent with an agent-bus-start-agent of this version"
            );
        }
        if !TOKEN_PREFIXES.iter().any(|prefix| line.starts_with(prefix))
            || !line
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
        {
            bail!("The GitHub write token is not a GitHub token (github_pat_..., ghp_...)");
        }
        Ok(Some(Self(line.to_owned())))
    }

    /// The token itself, for Claude Code's environment only.
    pub(super) fn expose(&self) -> &str {
        &self.0
    }
}

impl Access<'_> {
    /// Whether the run is a `--github` task's.
    pub(super) fn github(self) -> bool {
        matches!(self, Self::Write(_))
    }
}

/// Refuse a write token that is also the agent's own `GH_TOKEN`: every task, with or without
/// `--github`, would have it.
///
/// # Errors
///
/// Fails when `environment`, the agent's `GH_TOKEN`, is the write token.
pub(super) fn check_environment(
    environment: Option<&OsStr>,
    token: Option<&WriteToken>,
) -> Result<()> {
    if let (Some(environment), Some(token)) = (environment, token)
        && environment == OsStr::new(token.expose())
    {
        bail!(
            "{GH_TOKEN_VARIABLE} in the guest agent's environment is the GitHub write token, so every task would have it: give the guest the read-only token in ~/.env.d/github.env"
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const TOKEN: &str = "github_pat_11ABCDEFG0123456789_abcdefXYZ";

    fn token() -> WriteToken {
        WriteToken::parse(TOKEN).unwrap().unwrap()
    }

    #[test]
    fn every_github_token_prefix_is_taken() {
        for prefix in TOKEN_PREFIXES {
            let line = format!("{prefix}abc123");
            assert_eq!(WriteToken::parse(&line).unwrap().unwrap().expose(), line);
        }
    }

    #[test]
    fn a_line_that_is_not_a_token_is_refused_without_showing_it() {
        for line in [
            "SUAMLK2ZNL35WSMW37E7UD4VZ7ELPEW7BY3LGQZ3QKWIIM4Q2ATZCM4TQ",
            "not-a-token",
            "ghp_with space",
            "ghp_semi;colon",
            "-----BEGIN CERTIFICATE-----",
        ] {
            let error = format!("{:#}", WriteToken::parse(line).unwrap_err());
            assert!(!error.contains(line), "{error}");
        }
        let seed = WriteToken::parse("SUAMLK2ZNL35WSMW37E7UD4VZ7EL").unwrap_err();
        assert!(seed.to_string().contains("NKey seed"));
    }

    #[test]
    fn the_token_never_shows_in_debug() {
        let token = token();
        for shown in [
            format!("{token:?}"),
            format!("{:?}", Access::Write(&token)),
            format!("{:?}", Some(&token)),
        ] {
            assert!(!shown.contains(TOKEN), "{shown}");
            assert!(shown.contains("redacted"), "{shown}");
        }
    }

    #[test]
    fn the_write_token_must_not_be_the_agents_own_gh_token() {
        let token = token();
        let error = check_environment(Some(OsStr::new(TOKEN)), Some(&token)).unwrap_err();
        let error = format!("{error:#}");
        assert!(error.contains("read-only token"), "{error}");
        assert!(!error.contains(TOKEN), "{error}");
        check_environment(Some(OsStr::new("github_pat_readonly")), Some(&token)).unwrap();
        check_environment(None, Some(&token)).unwrap();
        check_environment(Some(OsStr::new(TOKEN)), None).unwrap();
    }

    #[test]
    fn only_write_access_is_github() {
        let token = token();
        assert!(Access::Write(&token).github());
        assert!(!Access::ReadOnly.github());
    }
}
