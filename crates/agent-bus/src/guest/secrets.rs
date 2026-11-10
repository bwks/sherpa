//! The guest agent's secrets: the Claude subscription token, the GitHub write token and its bus
//! seed. `start-guest.sh` pipes them to the agent's stdin, one per line in that order, and the
//! agent holds them in memory only.
//!
//! None of them is ever in the agent's own environment (a task, a process of the same user, can
//! read `/proc/<agent pid>/environ`), on a command line, in a file or in a log. Each type's `Debug`
//! is redacted and none has a `Display`. The agent refuses to start with the seed or the Claude
//! token in its environment, and replaces all three in every text it sends or records.
//!
//! The seed never leaves the agent: it only signs the bus's challenge. The Claude token is set in
//! the environment of each Claude Code run, which needs it there; the write token in a `--github`
//! run's alone.

use std::borrow::Cow;
use std::ffi::OsStr;
use std::fmt;
use std::io::{BufRead, Read};

use anyhow::{Context, Result, bail};
use tracing::instrument;

use super::github::{REDACTED, WriteToken};
use crate::keys::{SEED_VARIABLE, Seed};

/// The variable Claude Code takes the subscription token from.
pub(super) const TOKEN_VARIABLE: &str = "CLAUDE_CODE_OAUTH_TOKEN";
/// What the Claude token is replaced with in text the agent sends or records.
pub(super) const REDACTED_CLAUDE: &str = "[Claude token]";
/// What the bus seed is replaced with in text the agent sends or records.
pub(super) const REDACTED_SEED: &str = "[bus seed]";
/// Longer than any of the secrets; a longer line is not one.
const MAX_LINE: u64 = 1024;

/// The Claude subscription token, from `claude setup-token`.
pub struct ClaudeToken(String);

/// Everything secret the guest agent holds.
#[derive(Debug)]
pub struct Secrets {
    claude: ClaudeToken,
    write_token: Option<WriteToken>,
    seed: Seed,
}

impl fmt::Debug for ClaudeToken {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ClaudeToken(<redacted>)")
    }
}

impl ClaudeToken {
    /// A token from one line. Errors never contain the line.
    ///
    /// # Errors
    ///
    /// Fails when the line is empty, has spaces or control characters, or is a seed or a GitHub
    /// token, which means the lines are out of order.
    pub fn parse(line: &str) -> Result<Self> {
        let line = line.trim_end_matches(['\n', '\r']);
        if line.is_empty() {
            bail!("The Claude subscription token is empty");
        }
        if line.starts_with("SU") || line.starts_with("github_pat_") || line.starts_with("gh") {
            bail!(
                "The Claude subscription token line holds another secret: start the agent with a start-guest.sh of this version"
            );
        }
        if !line.bytes().all(|byte| byte.is_ascii_graphic()) {
            bail!("The Claude subscription token has spaces or control characters");
        }
        Ok(Self(line.to_owned()))
    }

    /// The token itself, for Claude Code's environment only.
    pub(super) fn expose(&self) -> &str {
        &self.0
    }
}

impl Secrets {
    /// The secrets, from their values.
    pub fn new(claude: ClaudeToken, write_token: Option<WriteToken>, seed: Seed) -> Self {
        Self {
            claude,
            write_token,
            seed,
        }
    }

    pub(super) fn claude(&self) -> &ClaudeToken {
        &self.claude
    }

    pub(super) fn write_token(&self) -> Option<&WriteToken> {
        self.write_token.as_ref()
    }

    pub(super) fn seed(&self) -> &Seed {
        &self.seed
    }

    /// `text` with every occurrence of each secret replaced by its label.
    pub(super) fn redact<'t>(&self, text: &'t str) -> Cow<'t, str> {
        let mut text = Cow::Borrowed(text);
        for (secret, label) in [
            (self.claude.expose(), REDACTED_CLAUDE),
            (self.seed.expose(), REDACTED_SEED),
        ] {
            if text.contains(secret) {
                text = Cow::Owned(text.replace(secret, label));
            }
        }
        if let Some(token) = &self.write_token
            && text.contains(token.expose())
        {
            text = Cow::Owned(text.replace(token.expose(), REDACTED));
        }
        text
    }
}

/// The secrets from `input`: the Claude token, the GitHub write token (an empty line for none)
/// and the bus seed, one per line in that order. Errors never contain a secret.
///
/// # Errors
///
/// Fails when the input cannot be read, or a line is missing, too long or not what it should be.
#[instrument(level = "debug", skip_all)]
pub fn read_secrets(mut input: impl BufRead) -> Result<Secrets> {
    let claude = ClaudeToken::parse(&read_line(&mut input)?)
        .context("The first line on stdin must be the Claude subscription token")?;
    let write_token = WriteToken::parse(&read_line(&mut input)?)
        .context("The second line on stdin must be the GitHub write token, or empty for none")?;
    let seed = Seed::parse(&read_line(&mut input)?)
        .context("The third line on stdin must be the agent's bus seed")?;
    Ok(Secrets::new(claude, write_token, seed))
}

/// One line of `input`; empty at the end of the input.
fn read_line(input: &mut impl BufRead) -> Result<String> {
    let mut line = String::new();
    input
        .by_ref()
        .take(MAX_LINE)
        .read_line(&mut line)
        .context("Unable to read the guest agent's secrets from stdin")?;
    if line.len() >= usize::try_from(MAX_LINE).unwrap_or(usize::MAX) {
        bail!("A line on stdin is too long to be one of the guest agent's secrets");
    }
    Ok(line)
}

/// Refuse to run with the seed or the Claude token in the agent's own environment, given as
/// `seed` and `claude`: every task could read them from `/proc/<agent pid>/environ`.
///
/// # Errors
///
/// Fails, naming the variables but never their values, when either is set.
pub(super) fn refuse_in_environment(seed: Option<&OsStr>, claude: Option<&OsStr>) -> Result<()> {
    let set: Vec<&str> = [(SEED_VARIABLE, seed), (TOKEN_VARIABLE, claude)]
        .into_iter()
        .filter_map(|(name, value)| value.map(|_| name))
        .collect();
    if !set.is_empty() {
        bail!(
            "{} must not be in the guest agent's environment, where every task could read it: the agent reads its secrets from stdin. Start it with dev/agent-bus/start-guest.sh",
            set.join(" and ")
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::io::Cursor;

    use crate::keys::generate;

    const CLAUDE: &str = "sk-ant-oat01-abcDEF_123-xyz";
    const WRITE: &str = "github_pat_11ABCDEFG0123456789_abcdefXYZ";

    fn input(claude: &str, write: &str, seed: &str) -> Cursor<String> {
        Cursor::new(format!("{claude}\n{write}\n{seed}\n"))
    }

    #[test]
    fn the_three_lines_are_the_claude_token_the_write_token_and_the_seed() {
        let seed = generate().unwrap().seed;
        let secrets = read_secrets(input(CLAUDE, WRITE, &seed)).unwrap();
        assert_eq!(secrets.claude().expose(), CLAUDE);
        assert_eq!(secrets.write_token().unwrap().expose(), WRITE);
        assert_eq!(secrets.seed().expose(), seed);

        let secrets = read_secrets(input(CLAUDE, "", &seed)).unwrap();
        assert!(secrets.write_token().is_none());

        let crlf = Cursor::new(format!("{CLAUDE}\r\n{WRITE}\r\n{seed}\r\n"));
        let secrets = read_secrets(crlf).unwrap();
        assert_eq!(secrets.claude().expose(), CLAUDE);
        assert_eq!(secrets.write_token().unwrap().expose(), WRITE);
        assert_eq!(secrets.seed().expose(), seed);

        // The last line need not end in a newline.
        let unterminated = Cursor::new(format!("{CLAUDE}\n\n{seed}"));
        assert_eq!(read_secrets(unterminated).unwrap().seed().expose(), seed);
    }

    #[test]
    fn missing_or_misordered_lines_are_refused_without_showing_a_secret() {
        let seed = generate().unwrap().seed;
        for (text, expected) in [
            (String::new(), "first line"),
            (format!("{CLAUDE}\n"), "third line"),
            (format!("{CLAUDE}\n{WRITE}\n"), "third line"),
            (format!("{CLAUDE}\n{seed}\n"), "second line"),
            (format!("{seed}\n{WRITE}\n{seed}\n"), "first line"),
            (format!("{WRITE}\n\n{seed}\n"), "first line"),
            (format!("{CLAUDE}\n{WRITE}\n{CLAUDE}\n"), "third line"),
            (format!("{CLAUDE} x\n\n{seed}\n"), "first line"),
        ] {
            let error = format!("{:#}", read_secrets(Cursor::new(text)).unwrap_err());
            assert!(error.contains(expected), "{error}");
            for secret in [CLAUDE, WRITE, seed.as_str()] {
                assert!(!error.contains(secret), "{error}");
            }
        }
    }

    #[test]
    fn an_overlong_line_is_refused() {
        let line = "a".repeat(2000);
        assert!(read_secrets(Cursor::new(line)).is_err());
    }

    #[test]
    fn no_secret_shows_in_debug() {
        let seed = generate().unwrap().seed;
        let secrets = read_secrets(input(CLAUDE, WRITE, &seed)).unwrap();
        for shown in [
            format!("{secrets:?}"),
            format!("{secrets:#?}"),
            format!("{:?}", secrets.claude()),
            format!("{:?}", Some(&secrets)),
        ] {
            for secret in [CLAUDE, WRITE, seed.as_str()] {
                assert!(!shown.contains(secret), "{shown}");
            }
            assert!(shown.contains("redacted"), "{shown}");
        }
    }

    #[test]
    fn redact_replaces_every_secret() {
        let seed = generate().unwrap().seed;
        let secrets = read_secrets(input(CLAUDE, WRITE, &seed)).unwrap();
        let text = format!("a={CLAUDE} b={WRITE} c={seed} again {CLAUDE}");
        assert_eq!(
            secrets.redact(&text),
            format!("a={REDACTED_CLAUDE} b={REDACTED} c={REDACTED_SEED} again {REDACTED_CLAUDE}")
        );
        assert!(matches!(secrets.redact("clean"), Cow::Borrowed(_)));

        let without_write = read_secrets(input(CLAUDE, "", &seed)).unwrap();
        assert_eq!(
            without_write.redact(&format!("{WRITE} {CLAUDE}")),
            format!("{WRITE} {REDACTED_CLAUDE}")
        );
    }

    #[test]
    fn the_seed_or_the_claude_token_in_the_environment_is_refused_by_name_only() {
        refuse_in_environment(None, None).unwrap();
        for (seed, claude, names) in [
            (Some("SUSECRET"), None, vec![SEED_VARIABLE]),
            (None, Some("sk-secret"), vec![TOKEN_VARIABLE]),
            (
                Some("SUSECRET"),
                Some("sk-secret"),
                vec![SEED_VARIABLE, TOKEN_VARIABLE],
            ),
            (Some(""), None, vec![SEED_VARIABLE]),
        ] {
            let error = refuse_in_environment(seed.map(OsStr::new), claude.map(OsStr::new))
                .unwrap_err()
                .to_string();
            for name in names {
                assert!(error.contains(name), "{error}");
            }
            assert!(!error.contains("SUSECRET"), "{error}");
            assert!(!error.contains("sk-secret"), "{error}");
        }
    }
}
