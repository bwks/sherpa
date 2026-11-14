//! Where a run ended, for every outcome (`ended`): the checkout's repository, its branch, and
//! the open pull requests of that branch in this checkout and in `../sherpa-docs`. The host
//! keeps their `## Generation` sections (`generation`); the guest only reports them.
//!
//! Finding where a run ended uses the agent's own `GH_TOKEN`, the read-only one. Each command
//! has a time limit and is killed if the agent drops it.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::Deserialize;
use tokio::process::Command;
use tracing::instrument;

use crate::gh::{gh_command, run, stderr};
use crate::message::{Ended, PullRequestRef, check_repo};

const GIT: &str = "git";
/// The docs checkout, beside this one.
const DOCS_CHECKOUT: &str = "sherpa-docs";
/// The most open pull requests of one branch looked at in one checkout.
const MAX_PULL_REQUESTS: &str = "20";

/// An open pull request, as `gh pr list --json` gives it.
#[derive(Debug, Deserialize)]
struct PullRequest {
    number: u64,
    url: String,
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
                list(gh, &checkout, branch)
                    .await
                    .map(|listed| (home.name.clone(), listed))
            } else {
                match repository(gh, &checkout).await {
                    Ok(docs) => list(gh, &checkout, branch)
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
    let mut command = gh_command(gh);
    command.current_dir(checkout).args([
        "repo",
        "view",
        "--json",
        "nameWithOwner,defaultBranchRef",
    ]);
    let output = run(&mut command, None, "gh repo view").await?;
    if !output.status.success() {
        bail!("gh repo view failed: {}", stderr(&output));
    }
    let repository: Repository = serde_json::from_slice(&output.stdout)
        .context("Unable to read the output of gh repo view")?;
    check_repo(&repository.name)?;
    Ok(repository)
}

/// The open pull requests whose head is `branch`, in the repository of `checkout`, with the
/// agent's own token.
async fn list(gh: &str, checkout: &Path, branch: &str) -> Result<Vec<PullRequest>> {
    let mut command = gh_command(gh);
    command
        .current_dir(checkout)
        .args(["pr", "list", "--state", "open"])
        .arg(format!("--head={branch}"))
        .args(["--limit", MAX_PULL_REQUESTS])
        .args(["--json", "number,url,isCrossRepository"]);
    let output = run(&mut command, None, "gh pr list").await?;
    if !output.status.success() {
        bail!("gh pr list failed: {}", stderr(&output));
    }
    serde_json::from_slice(&output.stdout).context("Unable to read the output of gh pr list")
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::process::Command as StdCommand;
    use std::slice;

    use super::*;
    use crate::message::PullRequestRef;

    /// A stand-in for `gh`: `repo view` prints `repo.json` from the checkout, or fails without
    /// it; `pr list` prints `prs.json` from the checkout. Every call is logged to `gh.log` in
    /// the checkout, with whether another token variable reached it.
    const FAKE_GH: &str = r#"#!/bin/bash
echo "$* other=$([ -n "$GITHUB_TOKEN" ] && echo set || echo none)" >> gh.log
case "$1 $2" in
"pr list")
    cat prs.json
    ;;
"repo view")
    if [ -e repo.json ]; then cat repo.json; else echo "no git remotes found" >&2; exit 1; fi
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
        // Listed with the agent's own token, and no other token variable.
        let log = fs::read_to_string(sherpa.join("gh.log")).unwrap();
        assert!(
            log.contains("repo view --json nameWithOwner,defaultBranchRef"),
            "{log}"
        );
        assert!(log.contains("pr list --state open --head=feat/x"), "{log}");
        assert!(log.contains("other=none"), "{log}");
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
}
