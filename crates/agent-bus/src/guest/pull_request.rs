//! Where a run ended, for every outcome (`ended`): the checkout's repository, its branch, and
//! the open pull requests of that branch in this checkout and in `../sherpa-docs`. And after a
//! `--github` run, the `## Generation` section of those pull requests, rewritten with `gh`
//! (`generation::update_body`).
//!
//! Finding where a run ended uses the agent's own `GH_TOKEN`, the read-only one. Rewriting a
//! section runs `gh` with the write token as `GH_TOKEN` in its own environment only, as Claude
//! Code does for a `--github` task: never in an argument, a file or the agent's environment. Each command
//! has a time limit and is killed if the agent drops it. Nothing here fails the task: every
//! failure becomes an `Outcome`, which the outcome message reports.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Output, Stdio};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use serde::Deserialize;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;
use tokio::time::timeout;
use tracing::instrument;

use super::generation::{Row, update_body};
use super::github::{GH_TOKEN_VARIABLE, OTHER_GITHUB_TOKEN_VARIABLES, WriteToken};
use crate::message::{Ended, PullRequestRef, check_repo};

/// The GitHub CLI.
pub(super) const GH: &str = "gh";
const GIT: &str = "git";
/// The docs checkout, beside this one.
const DOCS_CHECKOUT: &str = "sherpa-docs";
/// How long one `git` or `gh` command may take.
const COMMAND_TIMEOUT: Duration = Duration::from_secs(60);
/// The most of a command's stderr an error quotes.
const MAX_STDERR: usize = 400;
/// The most open pull requests of one branch looked at in one checkout.
const MAX_PULL_REQUESTS: &str = "20";

/// An open pull request, as `gh pr list --json` gives it.
#[derive(Debug, Deserialize)]
struct PullRequest {
    number: u64,
    url: String,
    #[serde(default)]
    body: String,
    /// Whether its head is in a fork: a branch of the same name in someone else's repository.
    #[serde(rename = "isCrossRepository")]
    cross_repository: bool,
}

/// A repository, as `gh repo view --json nameWithOwner,defaultBranchRef` gives it.
#[derive(Debug, Deserialize)]
struct Repository {
    #[serde(rename = "nameWithOwner")]
    name: String,
    /// `None` for an empty repository.
    #[serde(rename = "defaultBranchRef")]
    default_branch: Option<BranchName>,
}

/// A branch, as `gh` names it in JSON.
#[derive(Debug, Deserialize)]
struct BranchName {
    name: String,
}

/// What happened to one pull request, or to one checkout's.
#[derive(Debug, PartialEq)]
pub(super) enum Outcome {
    /// Its section was rewritten: its URL.
    Updated(String),
    /// Its section already said this: its URL.
    Unchanged(String),
    /// It, or the checkout's list, could not be updated: what and why.
    Failed(String),
}

/// What the Generation step did after a `--github` run, for the outcome message.
#[derive(Debug, PartialEq)]
pub(super) enum Note {
    /// No pull request was looked at: why.
    Skipped(String),
    /// The open pull requests of `branch` were looked at.
    Checked {
        branch: String,
        outcomes: Vec<Outcome>,
    },
}

impl Note {
    /// One line for the outcome message and the log.
    pub(super) fn text(&self) -> String {
        match self {
            Self::Skipped(why) => format!("Generation section not updated: {why}"),
            Self::Checked { branch, outcomes } if outcomes.is_empty() => {
                format!("Generation section: no open pull request for branch {branch}")
            }
            Self::Checked { branch, outcomes } => {
                let mut text = format!("Generation section for branch {branch}:");
                for (index, outcome) in outcomes.iter().enumerate() {
                    let separator = if index == 0 { " " } else { "; " };
                    let _ = match outcome {
                        Outcome::Updated(url) => write!(text, "{separator}updated {url}"),
                        Outcome::Unchanged(url) => write!(text, "{separator}up to date {url}"),
                        Outcome::Failed(why) => write!(text, "{separator}not updated: {why}"),
                    };
                }
                text
            }
        }
    }
}

/// The checkouts whose pull requests are looked at: `workdir`, and `../sherpa-docs` beside it
/// when that is a git checkout.
pub(super) fn checkouts(workdir: &Path) -> Vec<PathBuf> {
    let mut checkouts = vec![workdir.to_owned()];
    if let Some(parent) = workdir.parent() {
        let docs = parent.join(DOCS_CHECKOUT);
        if docs != workdir && docs.join(".git").exists() {
            checkouts.push(docs);
        }
    }
    checkouts
}

/// The branch `workdir` has checked out; `None` for a detached HEAD.
///
/// # Errors
///
/// Fails when `git` cannot be run or `workdir` is not a git checkout.
#[instrument(level = "debug", skip_all, fields(workdir = %workdir.display()))]
pub(super) async fn current_branch(workdir: &Path) -> Result<Option<String>> {
    let mut command = Command::new(GIT);
    command
        .args(["symbolic-ref", "--quiet", "--short", "HEAD"])
        .current_dir(workdir);
    let output = run(&mut command, None, "git symbolic-ref").await?;
    let branch = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    match output.status.code() {
        Some(0) if !branch.is_empty() => Ok(Some(branch)),
        // --quiet: exit status 1 and nothing said is a detached HEAD.
        Some(1) if output.stderr.is_empty() => Ok(None),
        _ => bail!(
            "Unable to find the branch of {}: {}",
            workdir.display(),
            stderr(&output)
        ),
    }
}

/// Where the run in `workdir` ended, found with `gh` at `gh` and the agent's own token: the
/// repository, the branch unless it is the repository's default or HEAD is detached, and the
/// open pull requests whose head is that branch in `checkouts(workdir)`, those from forks left
/// out. A docs checkout whose pull requests cannot be listed is reported on stderr and left out.
///
/// # Errors
///
/// Fails when `workdir` is not a git checkout, or its repository cannot be read.
#[instrument(level = "debug", skip_all, fields(workdir = %workdir.display()))]
pub(super) async fn ended(gh: &str, workdir: &Path) -> Result<Ended> {
    let branch = current_branch(workdir).await?;
    let home = repository(gh, workdir).await?;
    let default_branch = home.default_branch.map(|branch| branch.name);
    let branch = branch.filter(|branch| Some(branch) != default_branch.as_ref());
    let mut pull_requests = Vec::new();
    if let Some(branch) = &branch {
        for checkout in checkouts(workdir) {
            let listed = if checkout == workdir {
                list(gh, &checkout, branch, None)
                    .await
                    .map(|listed| (home.name.clone(), listed))
            } else {
                match repository(gh, &checkout).await {
                    Ok(docs) => list(gh, &checkout, branch, None)
                        .await
                        .map(|listed| (docs.name, listed)),
                    Err(error) => Err(error),
                }
            };
            match listed {
                Ok((repo, listed)) => pull_requests.extend(
                    listed
                        .into_iter()
                        .filter(|pull_request| !pull_request.cross_repository)
                        .map(|pull_request| PullRequestRef {
                            repo: repo.clone(),
                            number: pull_request.number,
                            url: pull_request.url,
                        }),
                ),
                Err(error) => eprintln!(
                    "Unable to list the pull requests of {}: {error:#}",
                    checkout.display()
                ),
            }
        }
    }
    Ok(Ended {
        repo: home.name,
        branch,
        pull_requests,
    })
}

/// The repository of `checkout`, with its default branch, from `gh repo view`.
async fn repository(gh: &str, checkout: &Path) -> Result<Repository> {
    let mut command = gh_command(gh, checkout, None);
    command.args(["repo", "view", "--json", "nameWithOwner,defaultBranchRef"]);
    let output = run(&mut command, None, "gh repo view").await?;
    if !output.status.success() {
        bail!("gh repo view failed: {}", stderr(&output));
    }
    let repository: Repository = serde_json::from_slice(&output.stdout)
        .context("Unable to read the output of gh repo view")?;
    check_repo(&repository.name)?;
    Ok(repository)
}

/// Rewrite the Generation section of every open pull request of `branch` in `checkouts` to hold
/// `rows`, with `gh` at `gh` and the write token. Pull requests from forks are left alone.
#[instrument(level = "debug", skip_all, fields(%branch))]
pub(super) async fn update(
    gh: &str,
    checkouts: &[PathBuf],
    branch: &str,
    rows: &[Row],
    token: &WriteToken,
) -> Vec<Outcome> {
    let mut outcomes = Vec::new();
    for checkout in checkouts {
        let pull_requests = match list(gh, checkout, branch, Some(token)).await {
            Ok(pull_requests) => pull_requests,
            Err(error) => {
                outcomes.push(Outcome::Failed(format!(
                    "unable to list the pull requests of {}: {error:#}",
                    checkout.display()
                )));
                continue;
            }
        };
        for pull_request in pull_requests.into_iter().filter(|pr| !pr.cross_repository) {
            let body = match update_body(&pull_request.body, rows) {
                Ok(body) => body,
                Err(error) => {
                    outcomes.push(Outcome::Failed(format!("{}: {error:#}", pull_request.url)));
                    continue;
                }
            };
            if body == pull_request.body {
                outcomes.push(Outcome::Unchanged(pull_request.url));
                continue;
            }
            outcomes.push(
                match set_body(gh, checkout, pull_request.number, &body, token).await {
                    Ok(()) => Outcome::Updated(pull_request.url),
                    Err(error) => Outcome::Failed(format!("{}: {error:#}", pull_request.url)),
                },
            );
        }
    }
    outcomes
}

/// The open pull requests whose head is `branch`, in the repository of `checkout`, with `token`,
/// else the agent's own.
async fn list(
    gh: &str,
    checkout: &Path,
    branch: &str,
    token: Option<&WriteToken>,
) -> Result<Vec<PullRequest>> {
    let mut command = gh_command(gh, checkout, token);
    command
        .args(["pr", "list", "--state", "open"])
        .arg(format!("--head={branch}"))
        .args(["--limit", MAX_PULL_REQUESTS])
        .args(["--json", "number,url,body,isCrossRepository"]);
    let output = run(&mut command, None, "gh pr list").await?;
    if !output.status.success() {
        bail!("gh pr list failed: {}", stderr(&output));
    }
    serde_json::from_slice(&output.stdout).context("Unable to read the output of gh pr list")
}

/// Replace the body of pull request `number` in the repository of `checkout` with `body`, given
/// on stdin.
async fn set_body(
    gh: &str,
    checkout: &Path,
    number: u64,
    body: &str,
    token: &WriteToken,
) -> Result<()> {
    let mut command = gh_command(gh, checkout, Some(token));
    command
        .args(["pr", "edit", &number.to_string()])
        .args(["--body-file", "-"]);
    let output = run(&mut command, Some(body), "gh pr edit").await?;
    if !output.status.success() {
        bail!("gh pr edit failed: {}", stderr(&output));
    }
    Ok(())
}

/// `gh` in `checkout`, never prompting, with the write token as `GH_TOKEN` when given, else the
/// agent's own `GH_TOKEN` (the read-only one), and no other token.
fn gh_command(gh: &str, checkout: &Path, token: Option<&WriteToken>) -> Command {
    let mut command = Command::new(gh);
    command
        .current_dir(checkout)
        .env("GH_PROMPT_DISABLED", "1")
        .env("NO_COLOR", "1");
    if let Some(token) = token {
        command.env(GH_TOKEN_VARIABLE, token.expose());
    }
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
async fn run(command: &mut Command, input: Option<&str>, what: &str) -> Result<Output> {
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
        .with_context(|| format!("Unable to start {what}"))?;
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
            Err(error).with_context(|| format!("Unable to write to {what}"))
        }
        Err(error) => Err(error).with_context(|| {
            format!(
                "Unable to write to {what}, which ended with {}",
                output.status
            )
        }),
    }
}

/// The start of a command's stderr, on one line.
fn stderr(output: &Output) -> String {
    let text = String::from_utf8_lossy(&output.stderr);
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut shown: String = text.chars().take(MAX_STDERR).collect();
    if shown.len() < text.len() {
        shown.push_str("...");
    }
    if shown.is_empty() {
        format!("exit status {}", output.status)
    } else {
        shown
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::process::Command as StdCommand;
    use std::slice;

    use super::*;
    use crate::guest::generation::END;
    use crate::message::{PullRequestRef, TaskUsage};

    const TOKEN: &str = "github_pat_pullrequest0123456789";

    /// A stand-in for `gh`: `repo view` prints `repo.json` from the checkout, or fails without
    /// it; `pr list` prints `prs.json` from the checkout; `pr edit <n>` saves
    /// its stdin as `body-<n>.md`, or, without reading it, fails when `edit-fails` exists, fails
    /// saying nothing when `edit-fails-silently` exists and succeeds when `edit-ignores-body`
    /// exists. Every call is logged, with whether it had the token, to `gh.log` in the checkout.
    const FAKE_GH: &str = r#"#!/bin/bash
echo "$* token=$([ "$GH_TOKEN" = "github_pat_pullrequest0123456789" ] && echo yes || echo no) other=${GITHUB_TOKEN:-none}" >> gh.log
case "$1 $2" in
"pr list")
    if [ -e list-fails ]; then echo "HTTP 401: Bad credentials" >&2; exit 1; fi
    cat prs.json
    ;;
"repo view")
    if [ -e repo.json ]; then cat repo.json; else echo "no git remotes found" >&2; exit 1; fi
    ;;
"pr edit")
    if [ -e edit-fails ]; then echo "GraphQL: not allowed" >&2; exit 1; fi
    if [ -e edit-fails-silently ]; then exit 1; fi
    if [ -e edit-ignores-body ]; then exit 0; fi
    cat > "body-$3.md"
    ;;
*)
    exit 2
    ;;
esac
"#;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("agent-bus-pr-{name}-{}", std::process::id()));
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

    fn token() -> WriteToken {
        WriteToken::parse(TOKEN).unwrap().unwrap()
    }

    fn rows() -> Vec<Row> {
        vec![Row {
            task: "t1".to_owned(),
            agent: "dev01".to_owned(),
            models: vec!["claude-opus-5-5".to_owned()],
            usage: TaskUsage {
                runs: 1,
                cost_usd: 0.5,
                ..TaskUsage::default()
            },
        }]
    }

    fn git(dir: &Path, args: &[&str]) {
        let status = StdCommand::new(GIT)
            .args(args)
            .current_dir(dir)
            .status()
            .unwrap();
        assert!(status.success(), "git {args:?}");
    }

    #[tokio::test]
    async fn the_branch_is_the_checked_out_one_and_none_when_detached() {
        let dir = scratch("branch");
        git(&dir, &["init", "-q", "-b", "feat/x"]);
        assert_eq!(
            current_branch(&dir).await.unwrap().as_deref(),
            Some("feat/x")
        );
        git(
            &dir,
            &[
                "-c",
                "user.name=t",
                "-c",
                "user.email=t@example.invalid",
                "commit",
                "-q",
                "--allow-empty",
                "-m",
                "one",
            ],
        );
        git(&dir, &["checkout", "-q", "--detach"]);
        assert_eq!(current_branch(&dir).await.unwrap(), None);
        let outside = scratch("not-a-checkout");
        assert!(current_branch(&outside).await.is_err());
        fs::remove_dir_all(&dir).unwrap();
        fs::remove_dir_all(&outside).unwrap();
    }

    #[test]
    fn the_docs_checkout_is_looked_at_when_it_is_a_checkout() {
        let dir = scratch("checkouts");
        let sherpa = dir.join("sherpa");
        fs::create_dir_all(&sherpa).unwrap();
        assert_eq!(checkouts(&sherpa), slice::from_ref(&sherpa));
        fs::create_dir_all(dir.join("sherpa-docs").join(".git")).unwrap();
        assert_eq!(
            checkouts(&sherpa),
            [sherpa.clone(), dir.join("sherpa-docs")]
        );
        fs::remove_dir_all(&dir).unwrap();
    }

    #[tokio::test]
    async fn open_pull_requests_get_the_section_and_forks_are_left_alone() {
        let dir = scratch("update");
        let gh = fake_gh(&dir);
        let current = update_body("Body", &rows()).unwrap();
        let prs = serde_json::json!([
            {"number": 7, "url": "https://example.invalid/pull/7", "body": "Body", "isCrossRepository": false},
            {"number": 8, "url": "https://example.invalid/pull/8", "body": current, "isCrossRepository": false},
            {"number": 9, "url": "https://example.invalid/pull/9", "body": "Fork", "isCrossRepository": true},
        ]);
        fs::write(dir.join("prs.json"), prs.to_string()).unwrap();
        let outcomes = update(&gh, slice::from_ref(&dir), "feat/x", &rows(), &token()).await;
        assert_eq!(
            outcomes,
            [
                Outcome::Updated("https://example.invalid/pull/7".to_owned()),
                Outcome::Unchanged("https://example.invalid/pull/8".to_owned()),
            ]
        );
        let written = fs::read_to_string(dir.join("body-7.md")).unwrap();
        assert!(written.starts_with("Body\n\n"), "{written}");
        assert!(written.trim_end().ends_with(END), "{written}");
        assert!(!dir.join("body-8.md").exists());
        assert!(!dir.join("body-9.md").exists());
        // The token is in gh's environment, never in its arguments; no other token is.
        let log = fs::read_to_string(dir.join("gh.log")).unwrap();
        assert!(log.contains("pr list --state open --head=feat/x"), "{log}");
        assert!(log.contains("pr edit 7 --body-file -"), "{log}");
        assert!(!log.contains(TOKEN), "{log}");
        assert!(!log.contains("token=no"), "{log}");
        assert!(
            !log.contains("other=") || log.contains("other=none"),
            "{log}"
        );
        fs::remove_dir_all(&dir).unwrap();
    }

    #[tokio::test]
    async fn failures_are_outcomes_not_errors() {
        let dir = scratch("failures");
        let gh = fake_gh(&dir);
        let prs = serde_json::json!([
            {"number": 7, "url": "https://example.invalid/pull/7", "body": "x", "isCrossRepository": false},
        ]);
        fs::write(dir.join("prs.json"), prs.to_string()).unwrap();
        fs::write(dir.join("edit-fails"), "").unwrap();
        let outcomes = update(&gh, slice::from_ref(&dir), "feat/x", &rows(), &token()).await;
        assert!(
            matches!(&outcomes[..], [Outcome::Failed(why)] if why.contains("pull/7") && why.contains("not allowed")),
            "{outcomes:?}"
        );
        fs::write(dir.join("list-fails"), "").unwrap();
        let outcomes = update(&gh, slice::from_ref(&dir), "feat/x", &rows(), &token()).await;
        assert!(
            matches!(&outcomes[..], [Outcome::Failed(why)] if why.contains("Bad credentials")),
            "{outcomes:?}"
        );
        let missing = dir.join("no-such-gh").to_string_lossy().into_owned();
        let outcomes = update(&missing, slice::from_ref(&dir), "feat/x", &rows(), &token()).await;
        assert!(
            matches!(&outcomes[..], [Outcome::Failed(why)] if why.contains("Unable to start gh pr list")),
            "{outcomes:?}"
        );
        fs::remove_dir_all(&dir).unwrap();
    }

    /// The outcome of editing pull request 7, whose body is far larger than a pipe holds, with
    /// the fake `gh` told what to do by the file `marker`: every write of the body to a `gh`
    /// that ends without reading it fails with a broken pipe.
    async fn edit_without_reading_the_body(name: &str, marker: &str) -> Vec<Outcome> {
        let dir = scratch(name);
        let gh = fake_gh(&dir);
        let body = "x".repeat(4 << 20);
        let prs = serde_json::json!([
            {"number": 7, "url": "https://example.invalid/pull/7", "body": body, "isCrossRepository": false},
        ]);
        fs::write(dir.join("prs.json"), prs.to_string()).unwrap();
        fs::write(dir.join(marker), "").unwrap();
        let outcomes = update(&gh, slice::from_ref(&dir), "feat/x", &rows(), &token()).await;
        fs::remove_dir_all(&dir).unwrap();
        outcomes
    }

    #[tokio::test]
    async fn a_gh_that_fails_before_reading_the_body_is_reported_not_the_broken_pipe() {
        let outcomes = edit_without_reading_the_body("fails-early", "edit-fails").await;
        assert!(
            matches!(&outcomes[..], [Outcome::Failed(why)]
                if why == "https://example.invalid/pull/7: gh pr edit failed: GraphQL: not allowed"),
            "{outcomes:?}"
        );
    }

    #[tokio::test]
    async fn a_broken_pipe_is_reported_when_gh_gives_no_reason() {
        let outcomes = edit_without_reading_the_body("fails-silently", "edit-fails-silently").await;
        assert!(
            matches!(&outcomes[..], [Outcome::Failed(why)]
                if why.contains("Unable to write to gh pr edit")
                    && why.contains("exit status: 1")
                    && why.contains("Broken pipe")),
            "{outcomes:?}"
        );
    }

    #[tokio::test]
    async fn a_broken_pipe_is_reported_when_gh_succeeds_without_reading_the_body() {
        let outcomes = edit_without_reading_the_body("ignores-body", "edit-ignores-body").await;
        assert!(
            matches!(&outcomes[..], [Outcome::Failed(why)]
                if why.contains("Unable to write to gh pr edit") && why.contains("Broken pipe")),
            "{outcomes:?}"
        );
    }

    /// Make `dir` a checkout of `repo` (default branch `main`) for the fake `gh`, on `branch`,
    /// with the open pull requests `prs`.
    fn fake_checkout(dir: &Path, repo: &str, branch: &str, prs: &serde_json::Value) {
        fs::create_dir_all(dir).unwrap();
        git(dir, &["init", "-q", "-b", branch]);
        let view = serde_json::json!({"nameWithOwner": repo, "defaultBranchRef": {"name": "main"}});
        fs::write(dir.join("repo.json"), view.to_string()).unwrap();
        fs::write(dir.join("prs.json"), prs.to_string()).unwrap();
    }

    #[tokio::test]
    async fn ended_names_the_branch_and_its_open_pull_requests_in_both_checkouts() {
        let dir = scratch("ended");
        let gh = fake_gh(&dir);
        let sherpa = dir.join("sherpa");
        let docs = dir.join("sherpa-docs");
        fake_checkout(
            &sherpa,
            "o/r",
            "feat/x",
            &serde_json::json!([
                {"number": 7, "url": "https://github.com/o/r/pull/7", "body": "b", "isCrossRepository": false},
                {"number": 9, "url": "https://github.com/fork/r/pull/9", "body": "b", "isCrossRepository": true},
            ]),
        );
        fake_checkout(
            &docs,
            "o/r-docs",
            "feat/x",
            &serde_json::json!([
                {"number": 3, "url": "https://github.com/o/r-docs/pull/3", "body": "b", "isCrossRepository": false},
            ]),
        );
        let found = ended(&gh, &sherpa).await.unwrap();
        assert_eq!(
            found,
            Ended {
                repo: "o/r".to_owned(),
                branch: Some("feat/x".to_owned()),
                pull_requests: vec![
                    PullRequestRef {
                        repo: "o/r".to_owned(),
                        number: 7,
                        url: "https://github.com/o/r/pull/7".to_owned(),
                    },
                    PullRequestRef {
                        repo: "o/r-docs".to_owned(),
                        number: 3,
                        url: "https://github.com/o/r-docs/pull/3".to_owned(),
                    },
                ],
            }
        );
        // Listed with the agent's own token (the read-only one), never the write token.
        let log = fs::read_to_string(sherpa.join("gh.log")).unwrap();
        assert!(
            log.contains("repo view --json nameWithOwner,defaultBranchRef"),
            "{log}"
        );
        assert!(log.contains("pr list --state open --head=feat/x"), "{log}");
        assert!(!log.contains("token=yes"), "{log}");
        assert!(!log.contains("pr edit"), "{log}");

        // A docs checkout whose repository cannot be read leaves only its own pull requests out.
        fs::remove_file(docs.join("repo.json")).unwrap();
        let found = ended(&gh, &sherpa).await.unwrap();
        assert_eq!(found.pull_requests.len(), 1);
        assert_eq!(found.pull_requests[0].number, 7);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[tokio::test]
    async fn ended_on_the_default_branch_or_a_detached_head_names_no_branch() {
        let dir = scratch("ended-default");
        let gh = fake_gh(&dir);
        let sherpa = dir.join("sherpa");
        let prs = serde_json::json!([
            {"number": 1, "url": "https://github.com/o/r/pull/1", "body": "b", "isCrossRepository": false},
        ]);
        fake_checkout(&sherpa, "o/r", "main", &prs);
        let on_main = ended(&gh, &sherpa).await.unwrap();
        assert_eq!(
            on_main,
            Ended {
                repo: "o/r".to_owned(),
                branch: None,
                pull_requests: Vec::new(),
            }
        );
        let log = fs::read_to_string(sherpa.join("gh.log")).unwrap();
        assert!(!log.contains("pr list"), "{log}");

        git(
            &sherpa,
            &[
                "-c",
                "user.name=t",
                "-c",
                "user.email=t@example.invalid",
                "commit",
                "-q",
                "--allow-empty",
                "-m",
                "one",
            ],
        );
        git(&sherpa, &["checkout", "-q", "--detach"]);
        assert_eq!(ended(&gh, &sherpa).await.unwrap().branch, None);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[tokio::test]
    async fn ended_fails_outside_a_checkout_or_without_the_repository() {
        let dir = scratch("ended-fails");
        let gh = fake_gh(&dir);
        assert!(ended(&gh, &dir).await.is_err());
        let sherpa = dir.join("sherpa");
        fake_checkout(&sherpa, "o/r", "feat/x", &serde_json::json!([]));
        fs::remove_file(sherpa.join("repo.json")).unwrap();
        let error = format!("{:#}", ended(&gh, &sherpa).await.unwrap_err());
        assert!(error.contains("no git remotes found"), "{error}");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_note_says_what_happened_on_one_line() {
        let checked = Note::Checked {
            branch: "feat/x".to_owned(),
            outcomes: vec![
                Outcome::Updated("https://example.invalid/pull/7".to_owned()),
                Outcome::Unchanged("https://example.invalid/pull/8".to_owned()),
                Outcome::Failed("https://example.invalid/pull/9: refused".to_owned()),
            ],
        };
        assert_eq!(
            checked.text(),
            "Generation section for branch feat/x: updated https://example.invalid/pull/7; \
             up to date https://example.invalid/pull/8; not updated: \
             https://example.invalid/pull/9: refused"
        );
        let none = Note::Checked {
            branch: "feat/x".to_owned(),
            outcomes: Vec::new(),
        };
        assert_eq!(
            none.text(),
            "Generation section: no open pull request for branch feat/x"
        );
        assert_eq!(
            Note::Skipped("detached HEAD".to_owned()).text(),
            "Generation section not updated: detached HEAD"
        );
    }
}
