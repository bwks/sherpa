//! Writing the Generation sections of pull requests: which are due, and writing them with `gh`.
//!
//! A sync renders, under the record's lock, the section of each pull request it looks at from
//! the record's rows for that pull request (`placement` decides which pull request counts each
//! task and each response of the host) merged into the section it last wrote, and compares the
//! two: unchanged
//! text makes no `gh` call at all. It then lets the record go before any network call, reads
//! each due body and state with `gh pr view`, rewrites the section (`section::update_body`)
//! and sets the body with `gh pr edit --body-file -`, then takes the record again to keep the
//! text written, the state and any error (`generation_pr`).
//!
//! - A change in the host's rows alone is written at most once every `DEBOUNCE` per pull
//!   request; a change in a task's row (a new outcome) is written at once, and so is every
//!   change when the sync is forced (`generation sync`, the `SessionEnd` hook).
//! - A pull request seen merged or closed gets that one last write, and is frozen from then on;
//!   `generation sync --pr N --frozen` rewrites one frozen section under the current rules,
//!   keeping the time of its final write.
//! - A pull request never written is looked at only when its item was active since the first
//!   sync, so the first sync does not rewrite the bodies of long-merged pull requests;
//!   `generation sync --all` and `--pr` look at it anyway.
//!
//! `gh` is `AGENT_BUS_GH`, else `gh` from `PATH`, with `GH_TOKEN` from agent-bus's own
//! environment and no other token variable. Without `GH_TOKEN` nothing is written: the writes
//! stay pending.

use std::collections::HashMap;
use std::env;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use jiff::Timestamp;
use serde::Deserialize;
use tracing::instrument;

use crate::files::millis;
use crate::generation::section::{Rows, hand_written, read, section_of, update_body};
use crate::gh::{GH, GH_TOKEN_VARIABLE, gh_command, run, stderr};
use crate::record::{Paths, Record};
use crate::store::generation::{
    LastWrite, PullRequest, placements, pull_requests, rows, save, since, written,
};

/// The variable naming the `gh` to run instead of the one on `PATH`.
pub const GH_VARIABLE: &str = "AGENT_BUS_GH";
/// How long a change in the host's rows alone waits after the last write.
const DEBOUNCE: Duration = Duration::from_mins(10);
/// The states of a pull request whose section is frozen.
const FROZEN: [&str; 2] = ["MERGED", "CLOSED"];

/// The GitHub CLI and the token it runs with. No `Debug`: it holds the token.
pub struct Github {
    pub gh: String,
    /// `GH_TOKEN`; `None` leaves every write pending.
    pub token: Option<String>,
}

/// Which pull requests a sync looks at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Scope {
    /// Those written before, and those whose item was active since the first sync.
    Due,
    /// Every pull request that names an item.
    All,
    /// This one only.
    One { repo: String, number: u64 },
}

/// What a sync does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    pub scope: Scope,
    /// Write at once, without the debounce of the host's rows.
    pub force: bool,
    /// Rewrite the section of a merged or closed pull request (`--pr N --frozen`), which is
    /// otherwise left as its final write made it.
    pub frozen: bool,
    /// The host's repository (`AGENT_BUS_REPO`): its pull request counts a task that named one
    /// in it and one in another repository.
    pub home_repo: Option<String>,
}

/// What happened to one pull request's section.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// Written, or found already as it should be, in a pull request in this state.
    Written { state: String },
    /// The text is what was last written: no `gh` call.
    Unchanged,
    /// Only the host's rows changed, and the last write is less than `DEBOUNCE` ago.
    Debounced,
    /// The pull request was merged or closed, and has had its last write.
    Frozen,
    /// Due, but there is no `GH_TOKEN` to write with.
    Pending,
    /// The last attempt failed less than `DEBOUNCE` ago: tried again after it, or when forced.
    Backoff(String),
    /// Not written: why.
    Failed(String),
}

/// What one sync did.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Synced {
    /// Each pull request looked at, with what happened to it.
    pub outcomes: Vec<(String, u64, Outcome)>,
    /// The pull requests whose body has a Generation heading or table written by hand outside
    /// the markers.
    pub hand_written: Vec<(String, u64)>,
    /// Why the outcomes could not be kept in the record: the next sync tries them again.
    pub unsaved: Option<String>,
}

/// A pull request due a write, with the rows it gets.
struct Due {
    prior: LastWrite,
    rows: Rows,
    /// When the first sync ran, for an automatic sync: a pull request never written that was
    /// merged or closed before it is frozen as it is. `None` for `--pr` and `--all`.
    since_ms: Option<u64>,
    /// Whether a frozen section is rewritten (`--frozen`).
    frozen: bool,
}

/// What a pull request needs, from what was last written and the record's rows.
#[derive(Debug, PartialEq, Eq)]
enum Decision {
    Frozen,
    Unchanged,
    Debounced,
    Backoff(String),
    Write,
}

/// A pull request as `gh pr view --json body,state,closedAt` gives it.
#[derive(Debug, Deserialize)]
struct View {
    body: String,
    state: String,
    /// When it was merged or closed; `None` while it is open.
    #[serde(default, rename = "closedAt")]
    closed_at: Option<Timestamp>,
}

impl Github {
    /// `gh` from `AGENT_BUS_GH`, else `PATH`, and `GH_TOKEN`, from agent-bus's environment.
    pub fn from_environment() -> Self {
        Self {
            gh: env::var(GH_VARIABLE)
                .ok()
                .filter(|gh| !gh.is_empty())
                .unwrap_or_else(|| GH.to_owned()),
            token: env::var(GH_TOKEN_VARIABLE)
                .ok()
                .filter(|token| !token.is_empty()),
        }
    }
}

impl Synced {
    /// How many pull requests are left to write for want of `GH_TOKEN`.
    pub fn pending(&self) -> usize {
        self.outcomes
            .iter()
            .filter(|(_, _, outcome)| *outcome == Outcome::Pending)
            .count()
    }
}

/// Sync the sections `request` names, with the record at `paths` (waiting up to `wait` for
/// it) and `github`, at `now_ms`.
///
/// # Errors
///
/// Fails when the record is busy for longer than `wait` (`Busy`) or cannot be read, or `--pr`
/// names a pull request no item has. A pull request that cannot be written is an outcome, not
/// an error.
#[instrument(skip_all, fields(scope = ?request.scope, force = request.force))]
pub async fn sync(
    paths: &Paths,
    request: &Request,
    github: &Github,
    wait: Duration,
    now_ms: u64,
) -> Result<Synced> {
    let record = Record::open_recorded(paths, wait).await?;
    let conn = record.conn();
    let since_ms = since(conn, now_ms).await?;
    let mut prior = written(conn).await?;
    let placements = placements(conn, request.home_repo.as_deref()).await?;
    let looked_at: Vec<PullRequest> = pull_requests(conn)
        .await?
        .into_iter()
        .filter(|pull_request| in_scope(&request.scope, pull_request, &prior, since_ms))
        .collect();
    if let Scope::One { repo, number } = &request.scope
        && looked_at.is_empty()
    {
        bail!(
            "No work item has pull request {repo}#{number}: link a task to it with `link <task> \
             --pr {number}`"
        );
    }
    let mut synced = Synced::default();
    let mut due = Vec::new();
    for pull_request in looked_at {
        let key = (pull_request.repo.clone(), pull_request.number);
        let mut last = prior.remove(&key).unwrap_or(LastWrite {
            repo: pull_request.repo.clone(),
            number: pull_request.number,
            ..LastWrite::default()
        });
        last.work = pull_request.work;
        let ours = rows(conn, &pull_request, &placements).await?;
        let outcome = match decide(&last, &ours, request, now_ms) {
            Ok(Decision::Write) if github.token.is_none() => Outcome::Pending,
            Ok(Decision::Write) => {
                due.push(Due {
                    prior: last,
                    rows: ours,
                    since_ms: (request.scope == Scope::Due).then_some(since_ms),
                    frozen: request.frozen,
                });
                continue;
            }
            Ok(Decision::Frozen) => Outcome::Frozen,
            Ok(Decision::Unchanged) => Outcome::Unchanged,
            Ok(Decision::Debounced) => Outcome::Debounced,
            Ok(Decision::Backoff(error)) => Outcome::Backoff(error),
            Err(error) => Outcome::Failed(format!("{error:#}")),
        };
        synced.outcomes.push((key.0, key.1, outcome));
    }
    // No network call while the record is held.
    drop(record);
    if due.is_empty() {
        return Ok(synced);
    }
    let mut results = Vec::new();
    for item in due {
        let seen = item.prior.written_at_ms;
        let (written, hand, outcome) = write(github, item, now_ms).await;
        if hand {
            synced
                .hand_written
                .push((written.repo.clone(), written.number));
        }
        synced
            .outcomes
            .push((written.repo.clone(), written.number, outcome));
        results.push((written, seen));
    }
    if let Err(error) = keep(paths, wait, &results).await {
        synced.unsaved = Some(format!("{error:#}"));
    }
    Ok(synced)
}

/// Whether `scope` looks at `pull_request`, with `prior` the pull requests written before and
/// `since_ms` when the first sync ran.
fn in_scope(
    scope: &Scope,
    pull_request: &PullRequest,
    prior: &HashMap<(String, u64), LastWrite>,
    since_ms: u64,
) -> bool {
    match scope {
        Scope::Due => {
            prior.contains_key(&(pull_request.repo.clone(), pull_request.number))
                || pull_request.active_ms.is_some_and(|at| at >= since_ms)
        }
        Scope::All => true,
        Scope::One { repo, number } => *repo == pull_request.repo && *number == pull_request.number,
    }
}

/// Keep `results` in the record at `paths`, taken again within `wait`. Each comes with the
/// `written_at_ms` this sync read before it wrote; when the record now holds a later write,
/// another sync wrote the body after this one read it, and its result is the one kept.
async fn keep(paths: &Paths, wait: Duration, results: &[(LastWrite, Option<u64>)]) -> Result<()> {
    let record = Record::open_recorded(paths, wait).await?;
    let stored = written(record.conn()).await?;
    for (result, seen) in results {
        let now = stored
            .get(&(result.repo.clone(), result.number))
            .and_then(|stored| stored.written_at_ms);
        if now > *seen {
            continue;
        }
        save(record.conn(), result).await?;
    }
    Ok(())
}

/// What the pull request last written as `prior` needs for the record's rows `ours` at
/// `now_ms`, for `request`:
///
/// - a frozen one nothing, except for `--pr`, which looks at it again;
/// - one whose last attempt failed within `DEBOUNCE`, nothing yet unless the sync is forced;
/// - unchanged text nothing;
/// - a change in the host's rows alone within `DEBOUNCE` of the last write, nothing yet
///   unless forced;
/// - anything else a write.
///
/// # Errors
///
/// Fails when the section last written cannot be read or the rows cannot be shown.
fn decide(prior: &LastWrite, ours: &Rows, request: &Request, now_ms: u64) -> Result<Decision> {
    let force = request.force;
    if is_frozen(prior.state.as_deref()) {
        return Ok(if matches!(request.scope, Scope::One { .. }) {
            Decision::Write
        } else {
            Decision::Frozen
        });
    }
    if let Some(error) = &prior.last_error
        && !force
        && within_debounce(prior.attempted_at_ms, now_ms)
    {
        return Ok(Decision::Backoff(error.clone()));
    }
    let Some(last) = prior.section.as_deref() else {
        return Ok(Decision::Write);
    };
    let next = update_body(last, ours)?;
    if next == last {
        return Ok(Decision::Unchanged);
    }
    if !force
        && within_debounce(prior.written_at_ms, now_ms)
        && read(last)?.tasks == read(&next)?.tasks
    {
        return Ok(Decision::Debounced);
    }
    Ok(Decision::Write)
}

/// Whether `at` is less than `DEBOUNCE` before `now_ms`.
fn within_debounce(at: Option<u64>, now_ms: u64) -> bool {
    at.is_some_and(|at| now_ms < at.saturating_add(millis(DEBOUNCE)))
}

/// Whether a pull request in `state` is frozen: merged or closed.
pub fn is_frozen(state: Option<&str>) -> bool {
    state.is_some_and(|state| FROZEN.contains(&state))
}

/// Write `due`'s section into its pull request's body with `github` at `now_ms`. Returns what
/// to keep (the text written and the state, or the error with the last write kept), whether
/// the body has a Generation section written by hand outside the markers, and what happened.
///
/// A pull request that was frozen and is still merged or closed is left alone (`--pr` looked
/// at it again), unless `--frozen` asked for it to be rewritten: then the time of its final
/// write is kept, so the host's work after it still goes on to the item's next pull request.
/// One never written, merged or closed before the first sync, is frozen as it is by an
/// automatic sync.
#[instrument(level = "debug", skip_all, fields(repo = %due.prior.repo, number = due.prior.number))]
async fn write(github: &Github, due: Due, now_ms: u64) -> (LastWrite, bool, Outcome) {
    let Due {
        prior,
        rows,
        since_ms,
        frozen,
    } = due;
    let mut kept = prior.clone();
    kept.attempted_at_ms = Some(now_ms);
    let (repo, number) = (prior.repo.as_str(), prior.number);
    let view = match view(github, repo, number).await {
        Ok(view) => view,
        Err(error) => {
            let error = format!("{error:#}");
            kept.last_error = Some(error.clone());
            return (kept, false, Outcome::Failed(error));
        }
    };
    let closed_before = view
        .closed_at
        .and_then(|at| u64::try_from(at.as_millisecond()).ok())
        .zip(since_ms)
        .is_some_and(|(closed_ms, since_ms)| closed_ms < since_ms);
    let was_frozen = is_frozen(prior.state.as_deref());
    let left_as_it_is = (was_frozen && !frozen) || (prior.section.is_none() && closed_before);
    if is_frozen(Some(&view.state)) && left_as_it_is {
        kept.state = Some(view.state);
        kept.last_error = None;
        return (kept, false, Outcome::Frozen);
    }
    let hand = hand_written(&view.body).unwrap_or_default();
    let written = match update_body(&view.body, &rows) {
        Ok(body) if body == view.body => Ok(body),
        Ok(body) => edit(github, repo, number, &body).await.map(|()| body),
        Err(error) => Err(error),
    };
    let outcome = match written.and_then(|body| written_section(&body)) {
        Ok(section) => {
            kept.section = Some(section);
            // A frozen section rewritten keeps the time of its final write.
            if !(was_frozen && is_frozen(Some(&view.state)) && kept.written_at_ms.is_some()) {
                kept.written_at_ms = Some(now_ms);
            }
            kept.state = Some(view.state.clone());
            kept.last_error = None;
            Outcome::Written { state: view.state }
        }
        Err(error) => {
            let error = format!("{error:#}");
            kept.last_error = Some(error.clone());
            Outcome::Failed(error)
        }
    };
    (kept, hand, outcome)
}

/// The section of `body`, which was just rewritten.
///
/// # Errors
///
/// Fails when it has none, which a rewritten body always has.
fn written_section(body: &str) -> Result<String> {
    section_of(body)?
        .map(str::to_owned)
        .context("The rewritten body has no Generation section")
}

/// The body and state of pull request `number` of `repo`.
#[instrument(level = "debug", skip(github))]
async fn view(github: &Github, repo: &str, number: u64) -> Result<View> {
    let mut command = gh_command(&github.gh);
    if let Some(token) = &github.token {
        command.env(GH_TOKEN_VARIABLE, token);
    }
    command
        .args(["pr", "view", &number.to_string()])
        .args(["--repo", repo])
        .args(["--json", "body,state,closedAt"]);
    let output = run(&mut command, None, "gh pr view").await?;
    if !output.status.success() {
        bail!("gh pr view failed: {}", stderr(&output));
    }
    serde_json::from_slice(&output.stdout).context("Unable to read the output of gh pr view")
}

/// Set the body of pull request `number` of `repo` to `body`, given on stdin.
#[instrument(level = "debug", skip(github, body))]
async fn edit(github: &Github, repo: &str, number: u64, body: &str) -> Result<()> {
    let mut command = gh_command(&github.gh);
    if let Some(token) = &github.token {
        command.env(GH_TOKEN_VARIABLE, token);
    }
    command
        .args(["pr", "edit", &number.to_string()])
        .args(["--repo", repo])
        .args(["--body-file", "-"]);
    let output = run(&mut command, Some(body), "gh pr edit").await?;
    if !output.status.success() {
        bail!("gh pr edit failed: {}", stderr(&output));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::path::PathBuf;
    use std::{env, process};

    use serde_json::json;

    use super::*;
    use crate::generation::section::{END, START};
    use crate::host::transcript::Response;
    use crate::message::{
        Ended, Kind, Message, ModelUsage, PullRequestRef, RefKind, TaskUsage, Usage, Work, WorkRef,
    };
    use crate::price::Tokens;
    use crate::record::{Busy, LOCK_WAIT};
    use crate::store::host::{Focus, add_focus, attribute, keep_prompt, keep_response};
    use crate::store::{Recorded, Ref, link, record, ref_item};

    const REPO: &str = "o/r";
    const NOW: u64 = 1_800_000_000_000;
    const MINUTE: u64 = 60_000;
    const MODEL: &str = "claude-opus-5-5";
    const SESSION: &str = "host-session";

    /// A stand-in for `gh`: `pr view <n>` prints `view-<n>.json` from its directory, or fails
    /// when `view-fails` is there; `pr edit <n>` saves the body it is given as `body-<n>.md`.
    /// Every call is logged to `gh.log`, with whether it had `GH_TOKEN` and another token
    /// variable, never their values.
    const FAKE_GH: &str = r#"#!/bin/bash
cd "$(dirname "$0")"
echo "$* token=$([ -n "$GH_TOKEN" ] && echo set || echo none) other=$([ -n "$GITHUB_TOKEN" ] && echo set || echo none)" >> gh.log
case "$1 $2" in
"pr view")
    if [ -e view-fails ]; then echo "HTTP 401: Bad credentials" >&2; exit 1; fi
    cat "view-$3.json"
    ;;
"pr edit")
    cat > "body-$3.md"
    ;;
*)
    exit 2
    ;;
esac
"#;

    /// A record in a scratch state directory with one task, t1, on the bench's pull request of
    /// `REPO`, and a stand-in `gh`.
    struct Bench {
        dir: PathBuf,
        paths: Paths,
        github: Github,
        seq: u64,
    }

    impl Bench {
        async fn new(name: &str) -> Self {
            let dir = env::temp_dir().join(format!("agent-bus-sync-{name}-{}", process::id()));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).unwrap();
            let gh = dir.join("fake-gh");
            fs::write(&gh, FAKE_GH).unwrap();
            fs::set_permissions(&gh, fs::Permissions::from_mode(0o755)).unwrap();
            let mut bench = Self {
                paths: Paths::in_dir(&dir.join("state")),
                github: Github {
                    gh: gh.to_string_lossy().into_owned(),
                    token: Some("test-token".to_owned()),
                },
                dir,
                seq: 0,
            };
            bench.message(&task("t1"), NOW).await;
            bench.outcome("t1", 1, 0.5, NOW, &[7]).await;
            bench.view(7, "Body", "OPEN");
            bench
        }

        async fn message(&mut self, message: &Message, at_ms: u64) {
            self.seq += 1;
            let mut opened = Record::open_recorded(&self.paths, LOCK_WAIT).await.unwrap();
            let recorded = Recorded {
                seq: self.seq,
                at_ms,
                message,
            };
            assert!(
                record(opened.conn_mut(), &recorded, Some(REPO))
                    .await
                    .unwrap()
            );
        }

        /// An outcome of `task`, which has had `runs` runs costing `cost_usd` in all, ended on
        /// a branch whose open pull requests are `numbers`.
        async fn outcome(
            &mut self,
            task: &str,
            runs: u64,
            cost_usd: f64,
            at_ms: u64,
            numbers: &[u64],
        ) {
            let outcome = Message {
                task: task.to_owned(),
                from: "dev01".to_owned(),
                to: "host".to_owned(),
                kind: Kind::Completed,
                text: "done".to_owned(),
                usage: Some(Usage {
                    input: 10,
                    output: 100,
                    cost_usd: 0.5,
                    models: vec![ModelUsage {
                        model: MODEL.to_owned(),
                        input: 10,
                        output: 100,
                        cost_usd: 0.5,
                        ..ModelUsage::default()
                    }],
                    work_ms: Some(2 * MINUTE),
                    testing_ms: Some(MINUTE),
                    ..Usage::default()
                }),
                task_usage: Some(TaskUsage {
                    runs,
                    cost_usd,
                    input: 10 * runs,
                    output: 100 * runs,
                    ..TaskUsage::default()
                }),
                github: true,
                model: Some(MODEL.to_owned()),
                work: None,
                ended: Some(Ended {
                    repo: REPO.to_owned(),
                    branch: Some("feat/x".to_owned()),
                    pull_requests: numbers
                        .iter()
                        .map(|number| PullRequestRef {
                            repo: REPO.to_owned(),
                            number: *number,
                            url: format!("https://github.com/o/r/pull/{number}"),
                        })
                        .collect(),
                }),
            };
            self.message(&outcome, at_ms).await;
        }

        /// A response of the host at `at_ms` at `speed`, after a prompt of the user a minute
        /// before, charged to the item of the bench's pull request.
        async fn host(&self, id: &str, at_ms: u64, speed: &str) {
            let record = Record::open_recorded(&self.paths, LOCK_WAIT).await.unwrap();
            let conn = record.conn();
            let item = ref_item(conn, &Ref::new(REPO, RefKind::PullRequest, "7"))
                .await
                .unwrap()
                .unwrap();
            add_focus(
                conn,
                &Focus {
                    from_ms: NOW - 60 * MINUTE,
                    session: None,
                    work: Some(item),
                    source: "use".to_owned(),
                    task: None,
                    pull_request: None,
                },
            )
            .await
            .unwrap();
            keep_prompt(
                conn,
                &format!("p-{id}"),
                SESSION,
                None,
                at_ms - MINUTE,
                true,
            )
            .await
            .unwrap();
            keep_response(
                conn,
                &Response {
                    message_id: id.to_owned(),
                    session: SESSION.to_owned(),
                    agent_id: None,
                    at_ms,
                    model: MODEL.to_owned(),
                    speed: speed.to_owned(),
                    tokens: Tokens {
                        input: 5,
                        output: 50,
                        ..Tokens::default()
                    },
                    calls: Vec::new(),
                },
            )
            .await
            .unwrap();
            attribute(conn).await.unwrap();
        }

        /// What `gh pr view <number>` says: `body`, in `state`.
        fn view(&self, number: u64, body: &str, state: &str) {
            let view = json!({"body": body, "state": state});
            fs::write(
                self.dir.join(format!("view-{number}.json")),
                view.to_string(),
            )
            .unwrap();
        }

        /// The body last given to `gh pr edit 7`, which `gh pr view 7` gives from now on.
        fn edited(&self) -> String {
            let body = fs::read_to_string(self.dir.join("body-7.md")).unwrap();
            self.view(7, &body, "OPEN");
            body
        }

        /// The calls made to `gh` so far, and forget them.
        fn calls(&self) -> Vec<String> {
            let log = self.dir.join("gh.log");
            let calls = fs::read_to_string(&log).unwrap_or_default();
            let _ = fs::remove_file(&log);
            calls.lines().map(str::to_owned).collect()
        }

        async fn sync(&self, scope: Scope, force: bool, now_ms: u64) -> Synced {
            let request = Request {
                scope,
                force,
                frozen: false,
                home_repo: Some(REPO.to_owned()),
            };
            sync(&self.paths, &request, &self.github, LOCK_WAIT, now_ms)
                .await
                .unwrap()
        }

        /// `agent-bus link <task> --pr <number> --repo <repo>`: the pull request names the
        /// task's item, and the task names it.
        async fn link(&self, task: &str, repo: &str, number: u64) {
            let mut opened = Record::open_recorded(&self.paths, LOCK_WAIT).await.unwrap();
            let target = Ref::new(repo, RefKind::PullRequest, number.to_string());
            link(opened.conn_mut(), task, &target, NOW).await.unwrap();
            self.view(number, "Body", "OPEN");
        }

        /// The pull request names `task`'s item and no task names it, as an older `link`
        /// left it: one that named the item but recorded no pull request for the task.
        async fn name_item(&self, task: &str, repo: &str, number: u64) {
            let opened = Record::open_recorded(&self.paths, LOCK_WAIT).await.unwrap();
            opened
                .conn()
                .execute(
                    "INSERT INTO work_ref (repo, kind, ref, work, source, at_ms) \
                     SELECT ?1, 'pull_request', ?2, work, 'manual', ?3 \
                     FROM task_work WHERE task = ?4",
                    (repo, number.to_string(), 1, task),
                )
                .await
                .unwrap();
            self.view(number, "Body", "OPEN");
        }

        async fn stored(&self) -> Option<LastWrite> {
            let record = Record::open_recorded(&self.paths, LOCK_WAIT).await.unwrap();
            written(record.conn())
                .await
                .unwrap()
                .remove(&(REPO.to_owned(), 7))
        }
    }

    impl Drop for Bench {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.dir);
        }
    }

    fn task(id: &str) -> Message {
        Message {
            task: id.to_owned(),
            from: "host".to_owned(),
            to: "dev01".to_owned(),
            kind: Kind::Task,
            text: "work on it".to_owned(),
            usage: None,
            task_usage: None,
            github: true,
            model: None,
            work: None,
            ended: None,
        }
    }

    fn written_once() -> Vec<String> {
        vec![
            "pr view 7 --repo o/r --json body,state,closedAt token=set other=none".to_owned(),
            "pr edit 7 --repo o/r --body-file - token=set other=none".to_owned(),
        ]
    }

    fn outcome_of(synced: &Synced) -> &Outcome {
        &synced.outcomes[0].2
    }

    #[tokio::test]
    async fn the_section_gets_task_and_host_rows_with_their_time() {
        let bench = Bench::new("rows").await;
        bench.host("m1", NOW + MINUTE, "standard").await;
        let synced = bench.sync(Scope::Due, false, NOW + MINUTE).await;
        assert_eq!(
            synced.outcomes,
            [(
                REPO.to_owned(),
                7,
                Outcome::Written {
                    state: "OPEN".to_owned()
                }
            )]
        );
        assert_eq!(bench.calls(), written_once());
        let body = bench.edited();
        assert!(body.starts_with(&format!("Body\n\n{START}\n")), "{body}");
        assert!(body.trim_end().ends_with(END), "{body}");
        for row in [
            "| t1 | dev01 | claude-opus-5-5 | 1 | $0.5000 | 0.0% | 10 | 100 | 0 | 0 | 2m 00s | 1m 00s | 0s |",
            "| host | host | claude-opus-5-5 | | $0.0010 | 0.0% | 5 | 50 | 0 | 0 | 1m 00s | 0s | 0s |",
            "| **Total** | | | **1** | **$0.5010** | **0.0%** | **15** | **150** | **0** | **0** | **3m 00s** | **1m 00s** | **0s** |",
        ] {
            assert!(body.contains(row), "{row}\n{body}");
        }
        let stored = bench.stored().await.unwrap();
        assert_eq!(stored.state.as_deref(), Some("OPEN"));
        assert_eq!(stored.written_at_ms, Some(NOW + MINUTE));
        assert_eq!(stored.last_error, None);
        assert!(body.contains(stored.section.as_deref().unwrap()));
    }

    #[tokio::test]
    async fn unchanged_text_makes_no_gh_call() {
        let bench = Bench::new("unchanged").await;
        bench.sync(Scope::Due, false, NOW).await;
        assert_eq!(bench.calls(), written_once());
        bench.edited();
        for force in [false, true] {
            let synced = bench.sync(Scope::Due, force, NOW + 30 * MINUTE).await;
            assert_eq!(outcome_of(&synced), &Outcome::Unchanged);
            assert_eq!(bench.calls(), Vec::<String>::new());
        }
    }

    #[tokio::test]
    async fn host_rows_alone_wait_ten_minutes_and_an_outcome_or_a_forced_sync_does_not() {
        let mut bench = Bench::new("debounce").await;
        bench.sync(Scope::Due, false, NOW).await;
        bench.calls();
        bench.edited();

        // Only the host's rows changed, a minute after the write: not yet.
        bench.host("m1", NOW + MINUTE, "standard").await;
        let synced = bench.sync(Scope::Due, false, NOW + MINUTE).await;
        assert_eq!(outcome_of(&synced), &Outcome::Debounced);
        assert_eq!(bench.calls(), Vec::<String>::new());

        // A new outcome is written at once, host rows and all.
        bench.outcome("t1", 2, 1.0, NOW + 2 * MINUTE, &[7]).await;
        let synced = bench.sync(Scope::Due, false, NOW + 2 * MINUTE).await;
        assert!(matches!(outcome_of(&synced), Outcome::Written { .. }));
        assert_eq!(bench.calls(), written_once());
        let body = bench.edited();
        assert!(
            body.contains("| t1 | dev01 | claude-opus-5-5 | 2 |"),
            "{body}"
        );
        assert!(body.contains("| host | host |"), "{body}");

        // More host rows: they wait, until ten minutes after the last write.
        bench.host("m2", NOW + 3 * MINUTE, "standard").await;
        let synced = bench.sync(Scope::Due, false, NOW + 11 * MINUTE).await;
        assert_eq!(outcome_of(&synced), &Outcome::Debounced);
        let synced = bench.sync(Scope::Due, false, NOW + 12 * MINUTE).await;
        assert!(matches!(outcome_of(&synced), Outcome::Written { .. }));
        assert_eq!(bench.calls(), written_once());
        bench.edited();

        // `generation sync` and the SessionEnd hook do not wait.
        bench.host("m3", NOW + 13 * MINUTE, "standard").await;
        let synced = bench.sync(Scope::Due, true, NOW + 13 * MINUTE).await;
        assert!(matches!(outcome_of(&synced), Outcome::Written { .. }));
        assert_eq!(bench.calls(), written_once());
    }

    #[tokio::test]
    async fn a_merged_pull_request_is_written_once_then_frozen() {
        let mut bench = Bench::new("merged").await;
        bench.view(7, "Body", "MERGED");
        let synced = bench.sync(Scope::Due, false, NOW).await;
        assert_eq!(
            outcome_of(&synced),
            &Outcome::Written {
                state: "MERGED".to_owned()
            }
        );
        assert_eq!(bench.calls(), written_once());
        assert_eq!(
            bench.stored().await.unwrap().state.as_deref(),
            Some("MERGED")
        );

        // Later work on the item stays in the record; the body is left alone.
        bench.outcome("t1", 2, 1.0, NOW + MINUTE, &[7]).await;
        bench.host("m1", NOW + MINUTE, "standard").await;
        for scope in [Scope::Due, Scope::All] {
            let synced = bench.sync(scope, true, NOW + 60 * MINUTE).await;
            assert_eq!(outcome_of(&synced), &Outcome::Frozen);
        }
        assert_eq!(bench.calls(), Vec::<String>::new());
        // `--pr` looks at it again, finds it still merged, and edits nothing.
        let one = Scope::One {
            repo: REPO.to_owned(),
            number: 7,
        };
        let synced = bench.sync(one, true, NOW + 60 * MINUTE).await;
        assert_eq!(outcome_of(&synced), &Outcome::Frozen);
        assert_eq!(
            bench.calls(),
            ["pr view 7 --repo o/r --json body,state,closedAt token=set other=none"]
        );
    }

    #[tokio::test]
    async fn a_hand_written_section_outside_the_markers_is_named_and_left_alone() {
        let bench = Bench::new("hand-written").await;
        let by_hand = "Body\n\n## Generation\n\n| host (Opus 5.5) | not measured |\n";
        bench.view(7, by_hand, "OPEN");
        let synced = bench.sync(Scope::Due, true, NOW).await;
        assert_eq!(synced.hand_written, [(REPO.to_owned(), 7)]);
        let body = bench.edited();
        assert!(body.starts_with(by_hand), "{body}");
        assert!(body.contains(START), "{body}");
        // A body without one is not named.
        bench.host("m1", NOW + MINUTE, "standard").await;
        let synced = bench.sync(Scope::Due, true, NOW + MINUTE).await;
        assert_eq!(synced.hand_written, [(REPO.to_owned(), 7)]);
        bench.view(7, "Body", "OPEN");
        bench.host("m2", NOW + 2 * MINUTE, "standard").await;
        let synced = bench.sync(Scope::Due, true, NOW + 2 * MINUTE).await;
        assert_eq!(synced.hand_written, Vec::<(String, u64)>::new());
    }

    #[tokio::test]
    async fn rows_of_tasks_the_record_does_not_know_are_carried_over() {
        let bench = Bench::new("carried").await;
        // The section a guest wrote before the record: an older task, and t1 with older
        // figures.
        let guest = format!(
            r#"Body

{START}
## Generation

| old table |

<!-- agent-bus:generation-data
[[task]]
task = "0ld7a5k"
agent = "dev02"
models = ["claude-sonnet-5-5"]

[task.usage]
runs = 2
cost_usd = 3.25
input = 1
output = 2
cache_creation = 0
cache_read = 0
num_turns = 0
duration_ms = 0
duration_api_ms = 0

[[task]]
task = "t1"
agent = "dev01"
models = ["claude-opus-5-5"]

[task.usage]
runs = 0
cost_usd = 0.1
input = 0
output = 0
cache_creation = 0
cache_read = 0
num_turns = 0
duration_ms = 0
duration_api_ms = 0
-->
{END}
"#
        );
        bench.view(7, &guest, "OPEN");
        bench.sync(Scope::Due, true, NOW).await;
        let body = bench.edited();
        assert!(
            body.contains(
                "| 0ld7a5k | dev02 | claude-sonnet-5-5 | 2 | $3.2500 | 0.0% | 1 | 2 | 0 | 0 | n/a | n/a | n/a |"
            ),
            "{body}"
        );
        assert!(
            body.contains("| t1 | dev01 | claude-opus-5-5 | 1 | $0.5000 |"),
            "{body}"
        );
        assert!(body.contains("**$3.7500**"), "{body}");
        assert!(!body.contains("old table"), "{body}");
        let at = body.find("0ld7a5k").unwrap();
        assert!(at < body.find("| t1 |").unwrap(), "{body}");
    }

    #[tokio::test]
    async fn without_gh_token_the_writes_stay_pending() {
        let mut bench = Bench::new("no-token").await;
        bench.github.token = None;
        let synced = bench.sync(Scope::Due, true, NOW).await;
        assert_eq!(outcome_of(&synced), &Outcome::Pending);
        assert_eq!(synced.pending(), 1);
        assert_eq!(bench.calls(), Vec::<String>::new());
        assert_eq!(bench.stored().await, None);
        // With the token, the next sync writes it.
        bench.github.token = Some("test-token".to_owned());
        let synced = bench.sync(Scope::Due, false, NOW + MINUTE).await;
        assert!(matches!(outcome_of(&synced), Outcome::Written { .. }));
        assert_eq!(bench.calls(), written_once());
    }

    #[tokio::test]
    async fn a_failed_gh_call_is_kept_and_tried_again() {
        let bench = Bench::new("fails").await;
        fs::write(bench.dir.join("view-fails"), "").unwrap();
        let synced = bench.sync(Scope::Due, false, NOW).await;
        assert!(
            matches!(outcome_of(&synced), Outcome::Failed(why) if why.contains("Bad credentials")),
            "{synced:?}"
        );
        let stored = bench.stored().await.unwrap();
        assert_eq!(stored.section, None);
        assert!(stored.last_error.unwrap().contains("Bad credentials"));
        fs::remove_file(bench.dir.join("view-fails")).unwrap();
        bench.calls();
        let synced = bench.sync(Scope::Due, true, NOW + MINUTE).await;
        assert!(matches!(outcome_of(&synced), Outcome::Written { .. }));
        assert_eq!(bench.stored().await.unwrap().last_error, None);
    }

    #[tokio::test]
    async fn a_pull_request_never_written_waits_for_activity_since_the_first_sync() {
        let mut bench = Bench::new("since").await;
        // The first sync runs a day after the item's last activity: it is left alone.
        let later = NOW + 24 * 60 * MINUTE;
        let synced = bench.sync(Scope::Due, false, later).await;
        assert_eq!(synced.outcomes, []);
        assert_eq!(bench.calls(), Vec::<String>::new());
        // `--pr` and `--all` look at it anyway.
        let one = Scope::One {
            repo: REPO.to_owned(),
            number: 7,
        };
        let synced = bench.sync(one, true, later).await;
        assert!(matches!(outcome_of(&synced), Outcome::Written { .. }));
        bench.calls();
        // A pull request no item has is refused by name.
        let request = Request {
            scope: Scope::One {
                repo: REPO.to_owned(),
                number: 8,
            },
            force: true,
            frozen: false,
            home_repo: None,
        };
        let error = sync(&bench.paths, &request, &bench.github, LOCK_WAIT, later)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("o/r#8"), "{error}");
        // New activity on an item makes its pull request due.
        bench.message(&task("t2"), later + MINUTE).await;
        bench.outcome("t2", 1, 0.25, later + MINUTE, &[7]).await;
        let synced = bench.sync(Scope::Due, false, later + MINUTE).await;
        assert!(matches!(outcome_of(&synced), Outcome::Written { .. }));
    }

    #[tokio::test]
    async fn a_busy_record_fails_the_sync_with_busy() {
        let bench = Bench::new("busy").await;
        let held = Record::open_recorded(&bench.paths, LOCK_WAIT)
            .await
            .unwrap();
        let request = Request {
            scope: Scope::Due,
            force: false,
            frozen: false,
            home_repo: None,
        };
        let error = sync(
            &bench.paths,
            &request,
            &bench.github,
            Duration::from_millis(100),
            NOW,
        )
        .await
        .err()
        .unwrap();
        assert!(error.downcast_ref::<Busy>().is_some(), "{error:#}");
        drop(held);
        assert_eq!(bench.calls(), Vec::<String>::new());
    }

    #[tokio::test]
    async fn a_pull_request_merged_before_the_first_sync_is_frozen_without_a_write() {
        let mut bench = Bench::new("merged-before").await;
        // The item's branch also had an earlier task's pull request, merged a day before the
        // first sync.
        bench.outcome("t0", 1, 0.5, NOW, &[8]).await;
        bench.outcome("t1", 1, 0.5, NOW, &[7]).await;
        let merged = json!({
            "body": "Old body",
            "state": "MERGED",
            "closedAt": "2027-01-14T08:00:00Z",
        });
        fs::write(bench.dir.join("view-8.json"), merged.to_string()).unwrap();
        bench.sync(Scope::Due, false, NOW).await;
        let calls = bench.calls();
        assert!(
            calls.iter().any(|call| call.starts_with("pr edit 7")),
            "{calls:?}"
        );
        assert!(
            !calls.iter().any(|call| call.starts_with("pr edit 8")),
            "{calls:?}"
        );
        let record = Record::open_recorded(&bench.paths, LOCK_WAIT)
            .await
            .unwrap();
        let eight = written(record.conn())
            .await
            .unwrap()
            .remove(&(REPO.to_owned(), 8))
            .unwrap();
        drop(record);
        assert_eq!(eight.state.as_deref(), Some("MERGED"));
        assert_eq!(eight.section, None);

        // New activity on the item does not reach it.
        bench.outcome("t1", 2, 1.0, NOW + MINUTE, &[7]).await;
        bench.sync(Scope::Due, false, NOW + MINUTE).await;
        let calls = bench.calls();
        assert!(
            calls.iter().any(|call| call.starts_with("pr edit 7")),
            "{calls:?}"
        );
        assert!(!calls.iter().any(|call| call.contains(" 8 ")), "{calls:?}");
    }

    #[tokio::test]
    async fn all_writes_a_pull_request_merged_before_the_first_sync() {
        let mut bench = Bench::new("merged-before-all").await;
        bench.outcome("t1", 1, 0.5, NOW, &[7, 8]).await;
        let merged = json!({
            "body": "Old body",
            "state": "MERGED",
            "closedAt": "2027-01-14T08:00:00Z",
        });
        fs::write(bench.dir.join("view-8.json"), merged.to_string()).unwrap();
        bench.sync(Scope::All, true, NOW).await;
        let calls = bench.calls();
        assert!(
            calls.iter().any(|call| call.starts_with("pr edit 8")),
            "{calls:?}"
        );
    }

    #[tokio::test]
    async fn an_unpriced_host_response_marks_the_cost() {
        let bench = Bench::new("unpriced").await;
        bench.host("m1", NOW, "standard").await;
        bench.host("m2", NOW, "fast").await;
        bench.sync(Scope::Due, false, NOW).await;
        let body = bench.edited();
        assert!(
            body.contains("| host | host | claude-opus-5-5 | | $0.0010 + 1 unpriced |"),
            "{body}"
        );
        assert!(body.contains("**$0.5010 (incomplete)**"), "{body}");
    }

    #[tokio::test]
    async fn a_row_of_a_task_the_record_has_in_another_item_leaves_the_section() {
        let mut bench = Bench::new("moved").await;
        // t2 was tagged with another issue, so it stays in that item though it ended on the
        // branch of the bench's pull request, whose item is t1's.
        let mut tagged = task("t2");
        tagged.work = Some(Work {
            refs: vec![WorkRef {
                repo: REPO.to_owned(),
                kind: RefKind::Issue,
                number: 999,
            }],
            session: None,
        });
        bench.message(&tagged, NOW).await;
        bench.outcome("t2", 1, 0.25, NOW, &[7]).await;
        // An older guest, counting by branch, wrote both into the section of the bench's pull
        // request.
        let guest = format!(
            r#"Body

{START}
## Generation

<!-- agent-bus:generation-data
[[task]]
task = "t1"
agent = "dev01"
models = []

[task.usage]
runs = 1
cost_usd = 0.1
input = 0
output = 0
cache_creation = 0
cache_read = 0
num_turns = 0
duration_ms = 0
duration_api_ms = 0

[[task]]
task = "t2"
agent = "dev01"
models = []

[task.usage]
runs = 1
cost_usd = 0.25
input = 0
output = 0
cache_creation = 0
cache_read = 0
num_turns = 0
duration_ms = 0
duration_api_ms = 0
-->
{END}
"#
        );
        bench.view(7, &guest, "OPEN");
        bench.sync(Scope::Due, true, NOW).await;
        let body = bench.edited();
        assert!(!body.contains("| t2 |"), "{body}");
        assert!(
            body.contains("| t1 | dev01 | claude-opus-5-5 | 1 |"),
            "{body}"
        );
    }

    #[tokio::test]
    async fn a_failed_pull_request_is_tried_again_after_the_debounce_or_when_forced() {
        let bench = Bench::new("backoff").await;
        fs::write(bench.dir.join("view-fails"), "").unwrap();
        bench.sync(Scope::Due, false, NOW).await;
        assert_eq!(bench.calls().len(), 1);
        // Within ten minutes, an automatic sync leaves it.
        let synced = bench.sync(Scope::Due, false, NOW + MINUTE).await;
        assert_eq!(bench.calls(), Vec::<String>::new());
        assert!(
            matches!(outcome_of(&synced), Outcome::Backoff(why) if why.contains("Bad credentials")),
            "{synced:?}"
        );
        // After them, it tries again.
        bench.sync(Scope::Due, false, NOW + 11 * MINUTE).await;
        assert_eq!(bench.calls().len(), 1);
        // `--pr` does not wait.
        let one = Scope::One {
            repo: REPO.to_owned(),
            number: 7,
        };
        bench.sync(one, true, NOW + 12 * MINUTE).await;
        assert_eq!(bench.calls().len(), 1);
    }

    #[tokio::test]
    async fn a_reopened_pull_request_is_written_again_by_pr() {
        let mut bench = Bench::new("reopened").await;
        bench.view(7, "Body", "CLOSED");
        bench.sync(Scope::Due, false, NOW).await;
        bench.calls();
        // Reopened, with more work: automatic syncs leave a frozen one alone.
        bench.view(7, "Body", "OPEN");
        bench.outcome("t1", 2, 1.0, NOW + MINUTE, &[7]).await;
        let synced = bench.sync(Scope::Due, true, NOW + MINUTE).await;
        assert_eq!(outcome_of(&synced), &Outcome::Frozen);
        assert_eq!(bench.calls(), Vec::<String>::new());
        // `--pr` looks again, finds it open and writes it.
        let one = Scope::One {
            repo: REPO.to_owned(),
            number: 7,
        };
        let synced = bench.sync(one, true, NOW + 2 * MINUTE).await;
        assert_eq!(
            outcome_of(&synced),
            &Outcome::Written {
                state: "OPEN".to_owned()
            }
        );
        assert_eq!(bench.calls(), written_once());
        assert_eq!(bench.stored().await.unwrap().state.as_deref(), Some("OPEN"));
        // A frozen one still merged is looked at by `--pr` and left alone.
        bench.view(7, "Body", "MERGED");
        bench.outcome("t1", 3, 1.5, NOW + 3 * MINUTE, &[7]).await;
        bench.sync(Scope::Due, false, NOW + 3 * MINUTE).await;
        bench.calls();
        bench.outcome("t1", 4, 2.0, NOW + 4 * MINUTE, &[7]).await;
        let one = Scope::One {
            repo: REPO.to_owned(),
            number: 7,
        };
        let synced = bench.sync(one, true, NOW + 4 * MINUTE).await;
        assert_eq!(outcome_of(&synced), &Outcome::Frozen);
        let calls = bench.calls();
        assert_eq!(calls.len(), 1, "{calls:?}");
        assert!(calls[0].starts_with("pr view 7"), "{calls:?}");
    }

    const DOCS: &str = "o/r-docs";

    /// Each task of a section with its brought-forward mark and cost, and each pointer with the
    /// pull request it names.
    type EditedRows = (Vec<(String, bool, f64)>, Vec<(String, u64)>);
    /// A task of the worked example: its ID, cost, branch and pull requests.
    type Example<'a> = (&'a str, f64, Option<&'a str>, &'a [(&'a str, u64)]);

    /// A task of the host tagged with the worked example's issue.
    fn tagged(id: &str) -> Message {
        Message {
            work: Some(Work {
                refs: vec![WorkRef {
                    repo: REPO.to_owned(),
                    kind: RefKind::Issue,
                    number: 648,
                }],
                session: None,
            }),
            ..task(id)
        }
    }

    /// The outcome of `id`, costing `cost_usd`, ended on `branch` (`None`: the default) with the
    /// open pull requests `pull_requests`.
    fn done(
        id: &str,
        cost_usd: f64,
        branch: Option<&str>,
        pull_requests: &[(&str, u64)],
    ) -> Message {
        Message {
            task: id.to_owned(),
            from: "dev01".to_owned(),
            to: "host".to_owned(),
            kind: Kind::Completed,
            text: "done".to_owned(),
            usage: Some(Usage {
                cost_usd,
                work_ms: Some(MINUTE),
                testing_ms: Some(0),
                ..Usage::default()
            }),
            task_usage: Some(TaskUsage {
                runs: 1,
                cost_usd,
                ..TaskUsage::default()
            }),
            github: true,
            model: Some(MODEL.to_owned()),
            work: None,
            ended: Some(Ended {
                repo: REPO.to_owned(),
                branch: branch.map(str::to_owned),
                pull_requests: pull_requests
                    .iter()
                    .map(|(repo, number)| PullRequestRef {
                        repo: (*repo).to_owned(),
                        number: *number,
                        url: format!("https://github.com/{repo}/pull/{number}"),
                    })
                    .collect(),
            }),
        }
    }

    /// The cost of each task in the data of the section last given to `gh pr edit <number>`,
    /// with its brought-forward mark, and the pointers.
    fn edited_rows(bench: &Bench, number: u64) -> EditedRows {
        let body = fs::read_to_string(bench.dir.join(format!("body-{number}.md"))).unwrap();
        let rows = read(section_of(&body).unwrap().unwrap()).unwrap();
        (
            rows.tasks
                .into_iter()
                .map(|row| (row.task, row.brought_forward, row.usage.cost_usd))
                .collect(),
            rows.counted
                .into_iter()
                .map(|pointer| (pointer.task, pointer.number))
                .collect(),
        )
    }

    #[tokio::test]
    async fn each_pull_request_of_an_item_counts_its_own_tasks_and_the_sections_add_up() {
        // One item from the host's record: two design tasks that ended on main, then a task for
        // each of the plan's three pull requests, the second and third each with a docs pull
        // request, with their real costs.
        let mut bench = Bench::new("per-pr").await;
        let tasks: [Example<'_>; 5] = [
            ("ddf64ef9", 2.752_657_6, None, &[]),
            ("296e6cf5", 2.839_870_2, None, &[]),
            (
                "e5d5a5bb",
                2.095_247_2,
                Some("chore/648-dev-login"),
                &[(REPO, 665)],
            ),
            (
                "1d6ad1ad",
                5.267_347_6,
                Some("fix/648-file-modes"),
                &[(REPO, 666), (DOCS, 142)],
            ),
            (
                "c9bf620b",
                3.220_726,
                Some("fix/648-remove-ztp-server"),
                &[(REPO, 667), (DOCS, 143)],
            ),
        ];
        for (index, (id, cost, branch, pull_requests)) in (0_u64..).zip(tasks) {
            let at_ms = NOW + index * 10 * MINUTE;
            bench.message(&tagged(id), at_ms).await;
            bench
                .message(&done(id, cost, branch, pull_requests), at_ms + 5 * MINUTE)
                .await;
        }
        for number in [665, 666, 667, 142, 143] {
            bench.view(number, "Body", "OPEN");
        }
        bench.sync(Scope::All, true, NOW + 60 * MINUTE).await;
        let design = |id: &str, cost| (id.to_owned(), true, cost);
        let own = |id: &str, cost| (id.to_owned(), false, cost);
        assert_eq!(
            edited_rows(&bench, 665),
            (
                vec![
                    own("e5d5a5bb", 2.095_247_2),
                    design("ddf64ef9", 2.752_657_6),
                    design("296e6cf5", 2.839_870_2),
                ],
                vec![]
            )
        );
        assert_eq!(
            edited_rows(&bench, 666),
            (vec![own("1d6ad1ad", 5.267_347_6)], vec![])
        );
        assert_eq!(
            edited_rows(&bench, 667),
            (vec![own("c9bf620b", 3.220_726)], vec![])
        );
        // The docs pull requests point to their sherpa pull request, in one line.
        assert_eq!(
            edited_rows(&bench, 142),
            (vec![], vec![("1d6ad1ad".to_owned(), 666)])
        );
        for (number, home) in [(142, 666), (143, 667)] {
            let docs = fs::read_to_string(bench.dir.join(format!("body-{number}.md"))).unwrap();
            let section = section_of(&docs).unwrap().unwrap();
            assert!(
                section.contains(&format!("## Generation\n\nCounted in o/r#{home}.\n\n")),
                "{docs}"
            );
            assert!(!section.contains("**Total**"), "{docs}");
        }
        // Summing the sections is the item's cost, each task once.
        let total: f64 = [665, 666, 667, 142, 143]
            .into_iter()
            .flat_map(|number| edited_rows(&bench, number).0)
            .map(|(_, _, cost)| cost)
            .sum();
        assert!((total - 16.175_848_6).abs() < 1e-9, "{total}");
    }

    #[tokio::test]
    async fn host_work_goes_to_the_pull_request_of_the_last_task_event_before_it() {
        let mut bench = Bench::new("host-split").await;
        // t1, on the bench's pull request, reported at NOW; t2 of the same branch, on a second
        // pull request, at NOW + 10 minutes.
        bench.message(&task("t2"), NOW + 9 * MINUTE).await;
        bench.outcome("t2", 1, 0.25, NOW + 10 * MINUTE, &[9]).await;
        bench.view(9, "Body", "OPEN");
        // One response after t1's result, one after t2's.
        bench.host("m1", NOW + 5 * MINUTE, "standard").await;
        bench.host("m2", NOW + 20 * MINUTE, "standard").await;
        bench.sync(Scope::All, true, NOW + 30 * MINUTE).await;
        let host_cost = |number: u64| {
            let body = fs::read_to_string(bench.dir.join(format!("body-{number}.md"))).unwrap();
            read(section_of(&body).unwrap().unwrap())
                .unwrap()
                .host
                .iter()
                .map(|row| row.usage.output)
                .sum::<u64>()
        };
        assert_eq!((host_cost(7), host_cost(9)), (50, 50));
        assert_eq!(edited_rows(&bench, 7).0.len(), 1);
        assert_eq!(edited_rows(&bench, 9).0.len(), 1);
    }

    #[tokio::test]
    async fn a_frozen_section_is_rewritten_by_pr_with_frozen_only_keeping_its_final_write() {
        let mut bench = Bench::new("frozen").await;
        bench.view(7, "Body", "MERGED");
        bench.sync(Scope::Due, false, NOW).await;
        bench.calls();
        let final_ms = bench.stored().await.unwrap().written_at_ms;
        assert_eq!(final_ms, Some(NOW));
        bench.outcome("t1", 2, 1.0, NOW + MINUTE, &[7]).await;
        let one = Scope::One {
            repo: REPO.to_owned(),
            number: 7,
        };
        let request = |frozen| Request {
            scope: one.clone(),
            force: true,
            frozen,
            home_repo: Some(REPO.to_owned()),
        };
        // `--pr` alone looks and leaves it.
        let synced = sync(
            &bench.paths,
            &request(false),
            &bench.github,
            LOCK_WAIT,
            NOW + MINUTE,
        )
        .await
        .unwrap();
        assert_eq!(outcome_of(&synced), &Outcome::Frozen);
        assert_eq!(bench.calls().len(), 1);
        // `--pr --frozen` rewrites it under the current rules and keeps its final write's time.
        let synced = sync(
            &bench.paths,
            &request(true),
            &bench.github,
            LOCK_WAIT,
            NOW + 2 * MINUTE,
        )
        .await
        .unwrap();
        assert_eq!(
            outcome_of(&synced),
            &Outcome::Written {
                state: "MERGED".to_owned()
            }
        );
        assert_eq!(bench.calls(), written_once());
        assert!(
            bench
                .edited()
                .contains("| t1 | dev01 | claude-opus-5-5 | 2 | $1.0000 |")
        );
        let stored = bench.stored().await.unwrap();
        assert_eq!(stored.written_at_ms, final_ms);
        assert_eq!(stored.state.as_deref(), Some("MERGED"));
    }

    #[tokio::test]
    async fn a_newer_write_by_another_sync_is_not_overwritten() {
        let bench = Bench::new("overlap").await;
        let record = Record::open_recorded(&bench.paths, LOCK_WAIT)
            .await
            .unwrap();
        let work = written(record.conn()).await.unwrap().len();
        assert_eq!(work, 0);
        let item = ref_item(record.conn(), &Ref::new(REPO, RefKind::PullRequest, "7"))
            .await
            .unwrap()
            .unwrap();
        // Another sync wrote at 200, after this one read the row as written at 100.
        let newer = LastWrite {
            repo: REPO.to_owned(),
            number: 7,
            work: item,
            state: Some("OPEN".to_owned()),
            section: Some("newer".to_owned()),
            written_at_ms: Some(200),
            last_error: None,
            attempted_at_ms: Some(200),
        };
        save(record.conn(), &newer).await.unwrap();
        drop(record);
        let older = LastWrite {
            section: Some("older".to_owned()),
            written_at_ms: Some(150),
            attempted_at_ms: Some(150),
            ..newer.clone()
        };
        keep(&bench.paths, LOCK_WAIT, &[(older, Some(100))])
            .await
            .unwrap();
        assert_eq!(bench.stored().await.unwrap(), newer);
    }

    const LAB: &str = "o/k8s-lab";

    #[tokio::test]
    async fn a_pull_request_no_task_named_names_the_pull_requests_that_count_its_item() {
        // t1 counted on the bench's pull request and t2 of the same branch on a second; a docs
        // and a third repository's pull request that name the item, which no task named (an
        // older `link`).
        let mut bench = Bench::new("linked").await;
        bench.message(&task("t2"), NOW + 9 * MINUTE).await;
        bench.outcome("t2", 1, 0.25, NOW + 10 * MINUTE, &[9]).await;
        bench.view(9, "Body", "OPEN");
        bench.host("m1", NOW + 5 * MINUTE, "standard").await;
        bench.name_item("t1", DOCS, 141).await;
        bench.name_item("t1", LAB, 1).await;
        bench.sync(Scope::All, true, NOW + 30 * MINUTE).await;
        for number in [141, 1] {
            let body = fs::read_to_string(bench.dir.join(format!("body-{number}.md"))).unwrap();
            let section = section_of(&body).unwrap().unwrap();
            assert!(
                section.contains("## Generation\n\nCounted in o/r#7, o/r#9.\n\n"),
                "{body}"
            );
            assert!(!section.contains("**Total**"), "{body}");
            assert_eq!(edited_rows(&bench, number), (vec![], vec![]));
        }
        // The pull requests that count the work are unchanged.
        assert_eq!(edited_rows(&bench, 7).0.len(), 1);
        assert_eq!(edited_rows(&bench, 9).0.len(), 1);
    }

    #[tokio::test]
    async fn an_item_with_only_pull_requests_no_task_named_says_none_counts_its_work_yet() {
        // A task of the worked example's issue that ended on the default branch, with no pull
        // request to count it on, and a pull request that names its item, which no task named.
        let mut bench = Bench::new("linked-only").await;
        bench.message(&tagged("d1"), NOW).await;
        bench
            .message(&done("d1", 1.5, None, &[]), NOW + 5 * MINUTE)
            .await;
        bench.name_item("d1", LAB, 2).await;
        bench.sync(Scope::All, true, NOW + 30 * MINUTE).await;
        let body = fs::read_to_string(bench.dir.join("body-2.md")).unwrap();
        let section = section_of(&body).unwrap().unwrap();
        assert!(
            section.contains(
                "## Generation\n\nNo work of this item is counted in a pull request yet.\n\n"
            ),
            "{body}"
        );
        assert!(!section.contains("**Total**"), "{body}");
        assert_eq!(edited_rows(&bench, 2), (vec![], vec![]));
    }

    #[tokio::test]
    async fn a_pull_request_linked_by_hand_counts_the_task() {
        // d1 ended on the default branch, with no pull request; `link d1 --pr 2` makes that pull
        // request its own, and t1, counted on the bench's pull request, points from the docs pull
        // request it is linked to.
        let mut bench = Bench::new("linked-counts").await;
        bench.message(&tagged("d1"), NOW).await;
        bench
            .message(&done("d1", 1.5, None, &[]), NOW + 5 * MINUTE)
            .await;
        bench.link("d1", LAB, 2).await;
        bench.link("t1", DOCS, 141).await;
        bench.sync(Scope::All, true, NOW + 30 * MINUTE).await;
        let body = fs::read_to_string(bench.dir.join("body-2.md")).unwrap();
        let section = section_of(&body).unwrap().unwrap();
        assert!(section.contains("**Total**"), "{body}");
        assert_eq!(edited_rows(&bench, 2).0.len(), 1, "{body}");
        let body = fs::read_to_string(bench.dir.join("body-141.md")).unwrap();
        let section = section_of(&body).unwrap().unwrap();
        assert!(
            section.contains("## Generation\n\nCounted in o/r#7.\n\n"),
            "{body}"
        );
    }
}
