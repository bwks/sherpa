//! What a message says about the work its task is for, with no I/O: the refs it names and how
//! they were learned. `work` applies them to the record.
//!
//! A task's tags (`work` on a `task` message) are `brief` facts. An outcome's `ended` is a
//! `guest` fact: its branch, which is absent on the default branch, and its open pull requests.
//! An outcome without `ended`, from an older guest or recorded before it existed, has its
//! Generation line parsed instead (`parsed`): the line names the branch and the URLs of its
//! pull requests, not the branch's repository, which is taken from the first pull request,
//! else from the host's own repository.

use serde::Serialize;

use crate::message::{Kind, Message, RefKind, check_repo};

/// The start of the Generation line of an outcome whose pull requests were looked at.
const CHECKED_LINE: &str = "Generation section for branch ";
/// The Generation line of an outcome whose branch has no open pull request.
const NO_PULL_REQUEST_LINE: &str = "Generation section: no open pull request for branch ";
/// Branches a parsed line never names an item by. The line does not say which branch is the
/// repository's default, so the two usual names are taken for it: every task that ended on the
/// default branch would otherwise be one item.
const USUAL_DEFAULT_BRANCHES: [&str; 2] = ["main", "master"];

/// How a ref was learned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// The task's tag, from `send --issue/--pr`.
    Brief,
    /// An outcome's `ended`.
    Guest,
    /// An outcome's Generation line.
    Parsed,
    /// `link`.
    Manual,
}

/// An issue, a pull request or a branch of a repository.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
pub struct Ref {
    /// `owner/name`.
    pub repo: String,
    pub kind: RefKind,
    /// The number of an issue or pull request, the name of a branch.
    #[serde(rename = "ref")]
    pub name: String,
}

/// The refs one message names, and how.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Facts {
    pub source: Source,
    pub refs: Vec<Ref>,
}

impl Source {
    /// The source as the record stores it.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Brief => "brief",
            Self::Guest => "guest",
            Self::Parsed => "parsed",
            Self::Manual => "manual",
        }
    }
}

impl Ref {
    pub fn new(repo: &str, kind: RefKind, name: impl Into<String>) -> Self {
        Self {
            repo: repo.to_owned(),
            kind,
            name: name.into(),
        }
    }

    /// The ref as people write it: `#12`, `PR #34`, `feat/x`.
    pub fn label(&self) -> String {
        match self.kind {
            RefKind::Issue => format!("#{}", self.name),
            RefKind::PullRequest => format!("PR #{}", self.name),
            RefKind::Branch => self.name.clone(),
        }
    }

    /// The ref with its repository: `o/r #12`, `o/r PR #34`, `o/r branch feat/x`.
    pub fn qualified(&self) -> String {
        match self.kind {
            RefKind::Issue | RefKind::PullRequest => format!("{} {}", self.repo, self.label()),
            RefKind::Branch => format!("{} branch {}", self.repo, self.name),
        }
    }
}

/// What `message` says about its task's work, with `home_repo` the host's repository for a
/// parsed line's branch; `None` when it says nothing.
pub fn facts(message: &Message, home_repo: Option<&str>) -> Option<Facts> {
    let (source, refs) = match message.kind {
        Kind::Task => {
            let work = message.work.as_ref()?;
            let refs = work
                .refs
                .iter()
                .map(|tag| Ref::new(&tag.repo, tag.kind, tag.number.to_string()))
                .collect();
            (Source::Brief, refs)
        }
        Kind::Question | Kind::Completed | Kind::Failed | Kind::Canceled => match &message.ended {
            Some(ended) => {
                let branch = ended
                    .branch
                    .iter()
                    .map(|branch| Ref::new(&ended.repo, RefKind::Branch, branch.clone()));
                let pull_requests = ended.pull_requests.iter().map(|pull_request| {
                    Ref::new(
                        &pull_request.repo,
                        RefKind::PullRequest,
                        pull_request.number.to_string(),
                    )
                });
                (Source::Guest, branch.chain(pull_requests).collect())
            }
            None => (
                Source::Parsed,
                parse_generation_line(&message.text, home_repo),
            ),
        },
        Kind::Answer | Kind::Progress | Kind::Cancel => return None,
    };
    let mut refs: Vec<Ref> = refs;
    refs.dedup();
    (!refs.is_empty()).then_some(Facts { source, refs })
}

/// The refs the Generation line at the start of an outcome's `text` names: its branch and its
/// pull requests. The branch's repository is `home_repo`, else the first pull request's; without
/// either, or on a usual default branch, the branch is left out.
pub fn parse_generation_line(text: &str, home_repo: Option<&str>) -> Vec<Ref> {
    let line = text.lines().next().unwrap_or_default();
    let (branch, rest) = if let Some(rest) = line.strip_prefix(CHECKED_LINE) {
        match rest.split_once(':') {
            Some((branch, rest)) => (branch.trim(), rest),
            None => return Vec::new(),
        }
    } else if let Some(branch) = line.strip_prefix(NO_PULL_REQUEST_LINE) {
        (branch.trim(), "")
    } else {
        return Vec::new();
    };
    let pull_requests: Vec<Ref> = rest
        .split_whitespace()
        .filter_map(|word| pull_request_url(word.trim_end_matches([';', ':', ','])))
        .collect();
    let branch_repo = home_repo
        .map(str::to_owned)
        .or_else(|| pull_requests.first().map(|first| first.repo.clone()));
    let mut refs = Vec::new();
    if let Some(repo) = branch_repo
        && !branch.is_empty()
        && !USUAL_DEFAULT_BRANCHES.contains(&branch)
    {
        refs.push(Ref::new(&repo, RefKind::Branch, branch));
    }
    for pull_request in pull_requests {
        if !refs.contains(&pull_request) {
            refs.push(pull_request);
        }
    }
    refs
}

/// The pull request a URL such as `https://github.com/owner/name/pull/7` names.
fn pull_request_url(url: &str) -> Option<Ref> {
    let path = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))?;
    let mut parts = path.split('/');
    let (_host, owner, name, pull, number) = (
        parts.next()?,
        parts.next()?,
        parts.next()?,
        parts.next()?,
        parts.next()?,
    );
    let number: u64 = number.parse().ok()?;
    let repo = format!("{owner}/{name}");
    (pull == "pull" && parts.next().is_none() && check_repo(&repo).is_ok())
        .then(|| Ref::new(&repo, RefKind::PullRequest, number.to_string()))
}

/// The name of an item with `refs`: its lowest issue number, else its lowest pull request, else
/// its first branch; `None` without refs.
pub fn display_name(refs: &[Ref]) -> Option<String> {
    let lowest = |kind: RefKind| {
        refs.iter()
            .filter(|named| named.kind == kind)
            .min_by_key(|named| named.name.parse::<u64>().unwrap_or(u64::MAX))
    };
    lowest(RefKind::Issue)
        .or_else(|| lowest(RefKind::PullRequest))
        .or_else(|| {
            refs.iter()
                .filter(|named| named.kind == RefKind::Branch)
                .min_by(|a, b| a.name.cmp(&b.name))
        })
        .map(Ref::label)
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::message::{Ended, PullRequestRef, Work, WorkRef};

    fn message(kind: Kind, text: &str) -> Message {
        Message {
            task: "t1".to_owned(),
            from: "dev".to_owned(),
            to: "host".to_owned(),
            kind,
            text: text.to_owned(),
            usage: None,
            task_usage: None,
            github: false,
            model: None,
            work: None,
            ended: None,
        }
    }

    fn branch(repo: &str, name: &str) -> Ref {
        Ref::new(repo, RefKind::Branch, name)
    }

    fn pull_request(repo: &str, number: u64) -> Ref {
        Ref::new(repo, RefKind::PullRequest, number.to_string())
    }

    #[test]
    fn a_task_s_tags_are_brief_facts() {
        let mut task = message(Kind::Task, "fix it");
        assert_eq!(facts(&task, None), None);
        task.work = Some(Work {
            refs: vec![
                WorkRef {
                    repo: "o/r".to_owned(),
                    kind: RefKind::Issue,
                    number: 580,
                },
                WorkRef {
                    repo: "o/r".to_owned(),
                    kind: RefKind::PullRequest,
                    number: 612,
                },
            ],
            session: None,
        });
        assert_eq!(
            facts(&task, None),
            Some(Facts {
                source: Source::Brief,
                refs: vec![
                    Ref::new("o/r", RefKind::Issue, "580"),
                    pull_request("o/r", 612)
                ],
            })
        );
    }

    #[test]
    fn an_outcome_s_ended_is_a_guest_fact_and_the_default_branch_names_nothing() {
        let mut done = message(Kind::Completed, "done");
        done.ended = Some(Ended {
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
        });
        assert_eq!(
            facts(&done, Some("o/home")),
            Some(Facts {
                source: Source::Guest,
                refs: vec![
                    branch("o/r", "feat/x"),
                    pull_request("o/r", 7),
                    pull_request("o/r-docs", 3)
                ],
            })
        );
        // On the default branch the guest reports no branch, so the outcome names nothing, and
        // its text is not parsed either.
        let mut on_main = message(
            Kind::Completed,
            "Generation section for branch main: updated https://github.com/o/r/pull/1\n\ndone",
        );
        on_main.ended = Some(Ended {
            repo: "o/r".to_owned(),
            branch: None,
            pull_requests: Vec::new(),
        });
        assert_eq!(facts(&on_main, Some("o/r")), None);
        // Progress, answers and cancels name nothing.
        for kind in [Kind::Progress, Kind::Answer, Kind::Cancel] {
            assert_eq!(facts(&message(kind, CHECKED_LINE), Some("o/r")), None);
        }
    }

    #[test]
    fn the_checked_generation_line_names_its_branch_and_every_pull_request() {
        let text = "Generation section for branch feat/x: updated https://github.com/o/r/pull/7; \
                    up to date https://github.com/o/r-docs/pull/3; not updated: \
                    https://github.com/o/r/pull/9: gh pr edit failed: GraphQL: refused; \
                    not updated: unable to list the pull requests of /home/x/y: HTTP 401\n\ndone";
        assert_eq!(
            parse_generation_line(text, Some("o/r")),
            [
                branch("o/r", "feat/x"),
                pull_request("o/r", 7),
                pull_request("o/r-docs", 3),
                pull_request("o/r", 9)
            ]
        );
        // Without the host's repository, the branch is in the first pull request's.
        assert_eq!(
            parse_generation_line(text, None)[0],
            branch("o/r", "feat/x")
        );
        let done = message(Kind::Completed, text);
        assert_eq!(facts(&done, None).unwrap().source, Source::Parsed);
    }

    #[test]
    fn the_no_pull_request_line_names_its_branch_in_the_home_repository() {
        let text = "Generation section: no open pull request for branch fix/y\n\nQUESTION: ok?";
        assert_eq!(
            parse_generation_line(text, Some("o/r")),
            [branch("o/r", "fix/y")]
        );
        // No repository to put the branch in: nothing.
        assert_eq!(parse_generation_line(text, None), []);
    }

    #[test]
    fn the_not_updated_line_and_other_text_name_nothing() {
        for text in [
            "Generation section not updated: the checkout is on a detached HEAD\n\ndone",
            "Generation section not updated: Unable to find the branch of /w: not a repo",
            "done",
            "",
            "Generation section for branch feat/x",
            // Not on the first line.
            "done\nGeneration section for branch feat/x: updated https://github.com/o/r/pull/7",
        ] {
            assert_eq!(parse_generation_line(text, Some("o/r")), [], "{text}");
        }
    }

    #[test]
    fn a_parsed_line_on_a_usual_default_branch_names_only_its_pull_requests() {
        for name in USUAL_DEFAULT_BRANCHES {
            let text = format!(
                "Generation section for branch {name}: updated https://github.com/o/r/pull/7"
            );
            assert_eq!(
                parse_generation_line(&text, Some("o/r")),
                [pull_request("o/r", 7)]
            );
        }
    }

    #[test]
    fn only_pull_request_urls_are_pull_requests() {
        assert_eq!(
            pull_request_url("https://github.com/o/r/pull/7"),
            Some(pull_request("o/r", 7))
        );
        for url in [
            "https://github.com/o/r/issues/7",
            "https://github.com/o/r/pull/x",
            "https://github.com/o/r/pull/7/files",
            "https://github.com/o/pull/7",
            "github.com/o/r/pull/7",
            "/home/x/y",
        ] {
            assert_eq!(pull_request_url(url), None, "{url}");
        }
    }

    #[test]
    fn an_item_is_named_by_its_lowest_issue_then_pull_request_then_branch() {
        let issue = |number: &str| Ref::new("o/r", RefKind::Issue, number);
        assert_eq!(
            display_name(&[
                branch("o/r", "feat/x"),
                pull_request("o/r", 612),
                issue("600"),
                issue("580"),
            ])
            .as_deref(),
            Some("#580")
        );
        // Numbers compare as numbers, not text.
        assert_eq!(
            display_name(&[pull_request("o/r", 99), pull_request("o/r", 100)]).as_deref(),
            Some("PR #99")
        );
        assert_eq!(
            display_name(&[branch("o/r", "feat/y"), branch("o/r", "feat/x")]).as_deref(),
            Some("feat/x")
        );
        assert_eq!(display_name(&[]), None);
    }
}
