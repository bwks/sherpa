//! Running `git` and the GitHub CLI: the guest finds where a run ended with them, and the host
//! reads and rewrites pull request bodies with `gh` (`generation::sync`).
//!
//! Every command has a time limit and is killed when it is dropped. `gh` never prompts, and
//! never sees a GitHub token variable other than `GH_TOKEN`: `gh` takes `GH_TOKEN` first, and
//! only `GH_TOKEN` is set on purpose.

use std::process::{Output, Stdio};
use std::time::Duration;

use anyhow::{Context, Result};
use tokio::io::AsyncWriteExt;
use tokio::process::Command;
use tokio::time::timeout;
use tracing::instrument;

use crate::error::AgentBusError;

/// The GitHub CLI.
pub const GH: &str = "gh";
/// The variable `gh`, and through `gh auth git-credential` git, take the token from.
pub const GH_TOKEN_VARIABLE: &str = "GH_TOKEN";
/// Other GitHub token variables, removed from every run of Claude Code and `gh`.
pub const OTHER_GITHUB_TOKEN_VARIABLES: [&str; 2] = ["GITHUB_TOKEN", "GH_ENTERPRISE_TOKEN"];
/// How long one `git` or `gh` command may take.
const COMMAND_TIMEOUT: Duration = Duration::from_secs(60);
/// The most of a command's stderr an error quotes.
const MAX_STDERR: usize = 400;

/// `gh` at `gh`, never prompting, without colour, and with no GitHub token variable but the
/// `GH_TOKEN` it inherits or is given.
pub fn gh_command(gh: &str) -> Command {
    let mut command = Command::new(gh);
    command.env("GH_PROMPT_DISABLED", "1").env("NO_COLOR", "1");
    for variable in OTHER_GITHUB_TOKEN_VARIABLES {
        command.env_remove(variable);
    }
    command
}

/// Run `command` to its end within `COMMAND_TIMEOUT`, with `input` on its stdin, and collect
/// its output. `what` names it in errors.
///
/// The input is written while the command runs, so neither waits on the other. A command may
/// end without reading it, as `gh` does when it is refused, and the write then fails with a
/// broken pipe. The command's own result says why, so the write error is returned only when
/// the command succeeded or failed without a word on stderr.
///
/// # Errors
///
/// Fails when the command cannot be started, takes too long, or its input cannot be written.
#[instrument(level = "debug", skip(command, input))]
pub async fn run(command: &mut Command, input: Option<&str>, what: &str) -> Result<Output> {
    command
        .stdin(if input.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let mut child = command
        .spawn()
        .with_context(|| AgentBusError::CommandNotStarted {
            what: what.to_owned(),
        })?;
    let stdin = child.stdin.take();
    let write = async {
        match (stdin, input) {
            (Some(mut stdin), Some(input)) => stdin.write_all(input.as_bytes()).await,
            _ => Ok(()),
        }
    };
    let (written, output) = timeout(COMMAND_TIMEOUT, async {
        tokio::join!(write, child.wait_with_output())
    })
    .await
    .with_context(|| format!("{what} took longer than {}s", COMMAND_TIMEOUT.as_secs()))?;
    let output = output.with_context(|| format!("{what} did not finish"))?;
    match written {
        Ok(()) => Ok(output),
        Err(_) if !output.status.success() && !output.stderr.is_empty() => Ok(output),
        Err(error) if output.status.success() => {
            Err(error).context(AgentBusError::InputNotWritten {
                what: what.to_owned(),
            })
        }
        Err(error) => Err(error).context(AgentBusError::InputNotWrittenAfterExit {
            what: what.to_owned(),
            status: output.status,
        }),
    }
}

/// The start of a command's stderr, on one line.
pub fn stderr(output: &Output) -> String {
    let text = String::from_utf8_lossy(&output.stderr);
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut shown: String = text.chars().take(MAX_STDERR).collect();
    if shown.len() < text.len() {
        shown.push_str("...");
    }
    if shown.is_empty() {
        output.status.to_string()
    } else {
        shown
    }
}

#[cfg(test)]
mod tests {
    use std::env;
    use std::fs;
    use std::io::{self, ErrorKind};
    use std::os::unix::fs::PermissionsExt;
    use std::path::{Path, PathBuf};
    use std::process;

    use super::*;

    /// A stand-in for `gh pr edit`: fails saying why when `edit-fails` exists in its directory,
    /// fails saying nothing when `edit-fails-silently` does, and otherwise succeeds; none of
    /// them reads its stdin.
    const FAKE_GH: &str = r#"#!/bin/bash
cd "$(dirname "$0")"
if [ -e edit-fails ]; then echo "GraphQL: not allowed" >&2; exit 1; fi
if [ -e edit-fails-silently ]; then exit 1; fi
exit 0
"#;

    fn scratch(name: &str) -> PathBuf {
        let dir = env::temp_dir().join(format!("agent-bus-gh-{name}-{}", process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn fake_gh(dir: &Path) -> String {
        let gh = dir.join("fake-gh");
        fs::write(&gh, FAKE_GH).unwrap();
        fs::set_permissions(&gh, fs::Permissions::from_mode(0o755)).unwrap();
        gh.to_string_lossy().into_owned()
    }

    /// The result of giving a body far larger than a pipe holds to a `gh` told what to do by
    /// the file `marker`, which ends without reading it: the write fails with a broken pipe.
    async fn edit_without_reading_the_body(name: &str, marker: Option<&str>) -> Result<Output> {
        let dir = scratch(name);
        let gh = fake_gh(&dir);
        if let Some(marker) = marker {
            fs::write(dir.join(marker), "").unwrap();
        }
        let body = "x".repeat(4 << 20);
        let mut command = gh_command(&gh);
        command.args(["pr", "edit", "7", "--body-file", "-"]);
        let result = run(&mut command, Some(&body), "gh pr edit").await;
        fs::remove_dir_all(&dir).unwrap();
        result
    }

    #[tokio::test]
    async fn a_gh_that_fails_before_reading_the_body_is_reported_not_the_broken_pipe() {
        let output = edit_without_reading_the_body("fails-early", Some("edit-fails"))
            .await
            .unwrap();
        assert!(!output.status.success());
        assert_eq!(stderr(&output), "GraphQL: not allowed");
    }

    #[tokio::test]
    async fn a_broken_pipe_is_reported_when_gh_gives_no_reason() {
        let error = edit_without_reading_the_body("fails-silently", Some("edit-fails-silently"))
            .await
            .unwrap_err();
        assert!(
            matches!(
                error.downcast_ref::<AgentBusError>(),
                Some(AgentBusError::InputNotWrittenAfterExit { what, status })
                    if what == "gh pr edit" && status.code() == Some(1)
            ),
            "{error:#}"
        );
        assert_eq!(
            error.downcast_ref::<io::Error>().map(io::Error::kind),
            Some(ErrorKind::BrokenPipe),
            "{error:#}"
        );
    }

    #[tokio::test]
    async fn a_broken_pipe_is_reported_when_gh_succeeds_without_reading_the_body() {
        let error = edit_without_reading_the_body("ignores-body", None)
            .await
            .unwrap_err();
        assert!(
            matches!(
                error.downcast_ref::<AgentBusError>(),
                Some(AgentBusError::InputNotWritten { what }) if what == "gh pr edit"
            ),
            "{error:#}"
        );
        assert_eq!(
            error.downcast_ref::<io::Error>().map(io::Error::kind),
            Some(ErrorKind::BrokenPipe),
            "{error:#}"
        );
    }

    #[tokio::test]
    async fn a_missing_program_cannot_be_started() {
        let mut command = gh_command("/nonexistent/agent-bus-gh");
        let error = run(&mut command, None, "gh pr view").await.unwrap_err();
        assert!(
            matches!(
                error.downcast_ref::<AgentBusError>(),
                Some(AgentBusError::CommandNotStarted { what }) if what == "gh pr view"
            ),
            "{error:#}"
        );
    }

    #[test]
    fn stderr_is_one_line_else_the_exit_status() {
        let output = process::Command::new("sh")
            .args(["-c", "printf 'a\\n  b\\n' >&2; exit 3"])
            .output()
            .unwrap();
        assert_eq!(stderr(&output), "a b");
        let silent = process::Command::new("sh")
            .args(["-c", "exit 3"])
            .output()
            .unwrap();
        assert_eq!(stderr(&silent), "exit status: 3");
    }
}
