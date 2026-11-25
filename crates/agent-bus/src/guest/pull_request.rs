//! Where a run ended, for every outcome (`ended`): the checkout's repository, its branch, and
//! the open pull requests of that branch, then those of the branch each other checkout has
//! checked out: `../sherpa-docs`, and every checkout the run's tool calls worked in (such as a
//! third repository it cloned). The host keeps their `## Generation` sections (`generation`);
//! the guest only reports them.
//!
//! Finding where a run ended uses the agent's own `GH_TOKEN`, the read-only one. Each command
//! has a time limit and is killed if the agent drops it.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::Deserialize;
use tokio::process::Command;
use tracing::instrument;

use crate::error::AgentBusError;
use crate::gh::{gh_command, run, stderr};
use crate::message::{Ended, PullRequestRef, check_repo};

const GIT: &str = "git";
/// The docs checkout, beside this one.
const DOCS_CHECKOUT: &str = "sherpa-docs";
/// The most open pull requests of one branch looked at in one checkout.
const MAX_PULL_REQUESTS: &str = "20";
/// The most checkouts looked at besides the run's own.
const MAX_CHECKOUTS: usize = 8;

/// An open pull request, as `gh pr list --json` gives it.
#[derive(Debug, Deserialize)]
struct PullRequest {
    number: u64,
    url: String,
    #[serde(rename = "isCrossRepository")]
    cross_repository: bool,
}

/// A repository, as `gh repo view --json nameWithOwner,defaultBranchRef` gives it.
#[derive(Debug, Deserialize)]
struct Repository {
    #[serde(rename = "nameWithOwner")]
    name: String,
    #[serde(rename = "defaultBranchRef")]
    default_branch: Option<BranchName>,
}

/// A branch, as `gh` names it in JSON.
#[derive(Debug, Deserialize)]
struct BranchName {
    name: String,
}

/// The checkouts besides `workdir` whose own branch's pull requests are looked at:
/// `../sherpa-docs` beside it when that is a git checkout, then the checkout holding each path
/// of `worked_in` (the places the run's tool calls named), each once and at most
/// `MAX_CHECKOUTS`. A path in no checkout, or gone, is left out.
pub(super) async fn checkouts(workdir: &Path, worked_in: &BTreeSet<PathBuf>) -> Vec<PathBuf> {
    let own = tokio::fs::canonicalize(workdir)
        .await
        .unwrap_or_else(|_| workdir.to_owned());
    let mut found: Vec<PathBuf> = Vec::new();
    if let Some(parent) = workdir.parent() {
        let docs = parent.join(DOCS_CHECKOUT);
        if docs != workdir && docs.join(".git").exists() {
            found.push(tokio::fs::canonicalize(&docs).await.unwrap_or(docs));
        }
    }
    for path in worked_in {
        if found.len() >= MAX_CHECKOUTS {
            break;
        }
        if let Some(top) = toplevel(path).await
            && top != own
            && !found.contains(&top)
        {
            found.push(top);
        }
    }
    found
}

/// The top of the checkout holding `path`, resolved; `None` when it is in none or is gone.
async fn toplevel(path: &Path) -> Option<PathBuf> {
    let dir = if tokio::fs::metadata(path).await.ok()?.is_dir() {
        path
    } else {
        path.parent()?
    };
    let mut command = Command::new(GIT);
    command
        .args(["rev-parse", "--show-toplevel"])
        .current_dir(dir);
    let output = run(&mut command, None, "git rev-parse").await.ok()?;
    if !output.status.success() {
        return None;
    }
    let top = PathBuf::from(String::from_utf8_lossy(&output.stdout).trim());
    tokio::fs::canonicalize(&top).await.ok()
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
/// open pull requests whose head is that branch; then, for each of `checkouts(workdir,
/// worked_in)`, the open pull requests whose head is the branch that checkout has checked out,
/// unless it is its repository's default or detached. Those from forks are left out. Another
/// checkout whose pull requests cannot be listed is reported on stderr and left out.
///
/// # Errors
///
/// Fails when `workdir` is not a git checkout, or its repository cannot be read.
#[instrument(level = "debug", skip_all, fields(workdir = %workdir.display()))]
pub(super) async fn ended(
    gh: &str,
    workdir: &Path,
    worked_in: &BTreeSet<PathBuf>,
) -> Result<Ended> {
    let branch = current_branch(workdir).await?;
    let home = repository(gh, workdir).await?;
    let default_branch = home.default_branch.map(|branch| branch.name);
    let branch = branch.filter(|branch| Some(branch) != default_branch.as_ref());
    let mut pull_requests = Vec::new();
    if let Some(branch) = &branch {
        match list(gh, workdir, branch).await {
            Ok(listed) => pull_requests.extend(open_of(&home.name, listed)),
            Err(error) => eprintln!(
                "Unable to list the pull requests of {}: {error:#}",
                workdir.display()
            ),
        }
    }
    for checkout in checkouts(workdir, worked_in).await {
        match own_branch_pull_requests(gh, &checkout).await {
            Ok(found) => {
                for pull_request in found {
                    let known = pull_requests.iter().any(|known: &PullRequestRef| {
                        known.repo == pull_request.repo && known.number == pull_request.number
                    });
                    if !known {
                        pull_requests.push(pull_request);
                    }
                }
            }
            Err(error) => eprintln!(
                "Unable to list the pull requests of {}: {error:#}",
                checkout.display()
            ),
        }
    }
    Ok(Ended {
        repo: home.name,
        branch,
        pull_requests,
    })
}

/// The open pull requests of the branch `checkout` has checked out, in its repository; none on
/// its default branch or a detached HEAD.
async fn own_branch_pull_requests(gh: &str, checkout: &Path) -> Result<Vec<PullRequestRef>> {
    let Some(branch) = current_branch(checkout).await? else {
        return Ok(Vec::new());
    };
    let repository = repository(gh, checkout).await?;
    if repository
        .default_branch
        .is_some_and(|default| default.name == branch)
    {
        return Ok(Vec::new());
    }
    let listed = list(gh, checkout, &branch).await?;
    Ok(open_of(&repository.name, listed))
}

/// The pull requests of `listed` in `repo`, those from forks left out.
fn open_of(repo: &str, listed: Vec<PullRequest>) -> Vec<PullRequestRef> {
    listed
        .into_iter()
        .filter(|pull_request| !pull_request.cross_repository)
        .map(|pull_request| PullRequestRef {
            repo: repo.to_owned(),
            number: pull_request.number,
            url: pull_request.url,
        })
        .collect()
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
        bail!(AgentBusError::CommandFailed {
            what: "gh repo view".to_owned(),
            stderr: stderr(&output),
        });
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
        bail!(AgentBusError::CommandFailed {
            what: "gh pr list".to_owned(),
            stderr: stderr(&output),
        });
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

    #[tokio::test]
    async fn the_docs_checkout_and_the_checkouts_worked_in_are_looked_at() {
        let dir = fs::canonicalize(scratch("checkouts")).unwrap();
        let sherpa = dir.join("sherpa");
        fs::create_dir_all(&sherpa).unwrap();
        git(&sherpa, &["init", "-q", "-b", "main"]);
        let none = BTreeSet::new();
        assert_eq!(checkouts(&sherpa, &none).await, Vec::<PathBuf>::new());
        fs::create_dir_all(dir.join("sherpa-docs").join(".git")).unwrap();
        assert_eq!(
            checkouts(&sherpa, &none).await,
            slice::from_ref(&dir.join("sherpa-docs"))
        );
        // A clone the run worked in, by a file in it and by a directory; the run's own
        // checkout, a path in no checkout and one that is gone are left out.
        let lab = dir.join("lab");
        fs::create_dir_all(lab.join("k8s")).unwrap();
        git(&lab, &["init", "-q", "-b", "main"]);
        fs::write(lab.join("README.md"), "lab").unwrap();
        let outside = dir.join("outside");
        fs::create_dir_all(&outside).unwrap();
        let worked_in: BTreeSet<PathBuf> = [
            lab.join("README.md"),
            lab.join("k8s"),
            sherpa.clone(),
            outside,
            dir.join("gone"),
        ]
        .into_iter()
        .collect();
        assert_eq!(
            checkouts(&sherpa, &worked_in).await,
            [dir.join("sherpa-docs"), lab]
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
        let found = ended(&gh, &sherpa, &BTreeSet::new()).await.unwrap();
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
        let found = ended(&gh, &sherpa, &BTreeSet::new()).await.unwrap();
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
        let on_main = ended(&gh, &sherpa, &BTreeSet::new()).await.unwrap();
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
        assert_eq!(
            ended(&gh, &sherpa, &BTreeSet::new()).await.unwrap().branch,
            None
        );
        fs::remove_dir_all(&dir).unwrap();
    }

    #[tokio::test]
    async fn other_checkouts_report_the_pull_requests_of_their_own_branch() {
        // The run ended with sherpa on its own branch, the docs on another, and a third
        // repository it cloned and worked in on a third.
        let dir = fs::canonicalize(scratch("ended-own-branch")).unwrap();
        let gh = fake_gh(&dir);
        let pr = |repo: &str, number: u64| serde_json::json!([{"number": number, "url": format!("https://github.com/{repo}/pull/{number}"), "isCrossRepository": false}]);
        let sherpa = dir.join("sherpa");
        fake_checkout(&sherpa, "o/r", "test/57-talos", &pr("o/r", 664));
        fake_checkout(
            &dir.join("sherpa-docs"),
            "o/r-docs",
            "docs/57-example",
            &pr("o/r-docs", 141),
        );
        let lab = dir.join("lab");
        fake_checkout(&lab, "o/lab", "feat/k8s-lab", &pr("o/lab", 1));
        // Worked in, but on its default branch: nothing to report.
        let other = dir.join("other");
        fake_checkout(&other, "o/other", "main", &pr("o/other", 5));
        let worked_in: BTreeSet<PathBuf> =
            [lab.join("repo.json"), other.clone()].into_iter().collect();
        let found = ended(&gh, &sherpa, &worked_in).await.unwrap();
        let named: Vec<(&str, u64)> = found
            .pull_requests
            .iter()
            .map(|pull_request| (pull_request.repo.as_str(), pull_request.number))
            .collect();
        assert_eq!(named, [("o/r", 664), ("o/r-docs", 141), ("o/lab", 1)]);
        assert_eq!(found.branch.as_deref(), Some("test/57-talos"));
        // On main in the run's own checkout, the others still report theirs.
        git(&sherpa, &["checkout", "-q", "-b", "main"]);
        let found = ended(&gh, &sherpa, &worked_in).await.unwrap();
        assert_eq!(found.branch, None);
        assert_eq!(found.pull_requests.len(), 2);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[tokio::test]
    async fn ended_fails_outside_a_checkout_or_without_the_repository() {
        let dir = scratch("ended-fails");
        let gh = fake_gh(&dir);
        assert!(ended(&gh, &dir, &BTreeSet::new()).await.is_err());
        let sherpa = dir.join("sherpa");
        fake_checkout(&sherpa, "o/r", "feat/x", &serde_json::json!([]));
        fs::remove_file(sherpa.join("repo.json")).unwrap();
        let error = ended(&gh, &sherpa, &BTreeSet::new()).await.unwrap_err();
        assert!(
            matches!(
                error.downcast_ref::<AgentBusError>(),
                Some(AgentBusError::CommandFailed { what, stderr })
                    if what == "gh repo view" && stderr == "no git remotes found"
            ),
            "{error:#}"
        );
        fs::remove_dir_all(&dir).unwrap();
    }
}
