//! `generation sync`, and the sync every record command runs after catching up and importing:
//! what each says about the pull requests it looked at.

use std::time::Duration;

use anyhow::Result;
use clap::Subcommand;
use tracing::instrument;

use crate::files::{now_ms, standard_state_dir};
use crate::generation::sync::{Github, Outcome, Request, Scope, Synced, sync};
use crate::record::{Busy, HOOK_LOCK_WAIT, Paths};
use crate::repo_or_environment;

/// What `generation` does.
#[derive(Subcommand)]
pub enum GenerationAction {
    /// Write the `## Generation` section of the pull requests of the work items now, after
    /// catching the record up and importing the host's transcripts: those written before and
    /// those whose item was active since the first sync. Changes in the host's rows are
    /// written at once, without the 10-minute wait. Runs `gh` (`AGENT_BUS_GH`, else from PATH)
    /// with `GH_TOKEN`; without it nothing is written
    Sync {
        /// The bus to catch the record up from, for example `nats://192.0.2.1:4222`. Without
        /// it: `AGENT_BUS_URL`, then where `bus start` recorded the bus
        #[arg(long)]
        server: Option<String>,
        /// Only this pull request, of --repo
        #[arg(long, conflicts_with = "all")]
        pr: Option<u64>,
        /// Every pull request of every work item, also those never written whose item was not
        /// active since the first sync
        #[arg(long)]
        all: bool,
        /// With --pr: rewrite its section even when it is merged or closed, under the current
        /// rules, keeping the time of its final write. Frozen sections are otherwise left as
        /// they are
        #[arg(long, requires = "pr")]
        frozen: bool,
        /// The repository of --pr, `owner/name`. Without it: `AGENT_BUS_REPO`
        #[arg(long)]
        repo: Option<String>,
    },
}

/// Sync the sections `request` names with the record at `paths`, waiting up to `wait` for it,
/// and say what happened: every pull request looked at on stdout when `verbose`, else only
/// writes, failures and notices, on stderr. A busy record is a warning.
///
/// # Errors
///
/// Fails, when `verbose`, if the record cannot be read or `--pr` names a pull request no item
/// has; otherwise every failure is a warning.
#[instrument(skip_all, fields(scope = ?request.scope, force = request.force))]
pub async fn sync_now(
    paths: &Paths,
    request: &Request,
    wait: Duration,
    verbose: bool,
) -> Result<()> {
    let github = Github::from_environment();
    match sync(paths, request, &github, wait, now_ms()).await {
        Ok(synced) => {
            for (line, notable) in lines(&synced) {
                if verbose {
                    println!("{line}");
                } else if notable {
                    eprintln!("{line}");
                }
            }
            Ok(())
        }
        Err(error) if error.downcast_ref::<Busy>().is_some() => {
            eprintln!("Generation sections not synced: {error:#}");
            Ok(())
        }
        Err(error) if verbose => Err(error),
        Err(error) => {
            eprintln!("Generation sections not synced: {error:#}");
            Ok(())
        }
    }
}

/// After a record command: sync the sections that are due, with the debounce, waiting
/// `HOOK_LOCK_WAIT` for the record, and only warn.
#[instrument]
pub async fn sync_after_command() {
    let request = Request {
        scope: Scope::Due,
        force: false,
        frozen: false,
        home_repo: repo_or_environment(None),
    };
    match standard_state_dir() {
        // Not verbose: failures are warnings, so there is no error to report.
        Ok(dir) => {
            let _ = sync_now(&Paths::in_dir(&dir), &request, HOOK_LOCK_WAIT, false).await;
        }
        Err(error) => eprintln!("Generation sections not synced: {error:#}"),
    }
}

/// What `synced` says, one line each, with whether it is worth saying after another command:
/// writes, failures, notices and pending writes are; pull requests left as they are, not.
fn lines(synced: &Synced) -> Vec<(String, bool)> {
    let mut lines = Vec::new();
    for (repo, number, outcome) in &synced.outcomes {
        let line = match outcome {
            Outcome::Written { state } if state == "OPEN" => {
                (format!("Generation section written: {repo}#{number}"), true)
            }
            Outcome::Written { state } => (
                format!(
                    "Generation section written: {repo}#{number} ({}: its last write)",
                    state.to_lowercase()
                ),
                true,
            ),
            Outcome::Unchanged => (format!("{repo}#{number}: up to date"), false),
            Outcome::Debounced => (
                format!(
                    "{repo}#{number}: only the host's rows changed, written 10 minutes after \
                     the last write"
                ),
                false,
            ),
            Outcome::Frozen => (
                format!("{repo}#{number}: merged or closed, left as last written"),
                false,
            ),
            Outcome::Pending => (format!("{repo}#{number}: due, not written"), false),
            Outcome::Backoff(why) => (
                format!(
                    "{repo}#{number}: the last write failed ({why}); tried again 10 minutes \
                     after it, or by `generation sync`"
                ),
                false,
            ),
            Outcome::Failed(why) => (
                format!("Generation section of {repo}#{number} not written: {why}"),
                true,
            ),
        };
        lines.push(line);
    }
    for (repo, number) in &synced.hand_written {
        lines.push((
            format!(
                "{repo}#{number} has a Generation heading or table outside the agent-bus \
                 markers, written by hand: tidy it; agent-bus does not edit it"
            ),
            true,
        ));
    }
    let pending = synced.pending();
    if pending > 0 {
        lines.push((
            format!(
                "GH_TOKEN is not set: {pending} Generation sections are left to write until a \
                 command runs with it"
            ),
            true,
        ));
    }
    if let Some(why) = &synced.unsaved {
        lines.push((
            format!(
                "Generation sections written but not kept in the record, so they are checked \
                 again next time: {why}"
            ),
            true,
        ));
    }
    if synced.outcomes.is_empty() {
        lines.push(("No Generation section to look at".to_owned(), false));
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_failures_notices_and_pending_writes_are_said_after_a_command() {
        let synced = Synced {
            outcomes: vec![
                (
                    "o/r".to_owned(),
                    7,
                    Outcome::Written {
                        state: "OPEN".to_owned(),
                    },
                ),
                (
                    "o/r".to_owned(),
                    8,
                    Outcome::Written {
                        state: "MERGED".to_owned(),
                    },
                ),
                ("o/r".to_owned(), 9, Outcome::Unchanged),
                ("o/r".to_owned(), 10, Outcome::Debounced),
                ("o/r".to_owned(), 11, Outcome::Frozen),
                ("o/r".to_owned(), 12, Outcome::Failed("HTTP 401".to_owned())),
                ("o/r".to_owned(), 13, Outcome::Pending),
            ],
            hand_written: vec![("o/r".to_owned(), 7)],
            unsaved: None,
        };
        let notable: Vec<String> = lines(&synced)
            .into_iter()
            .filter(|(_, notable)| *notable)
            .map(|(line, _)| line)
            .collect();
        assert_eq!(
            notable,
            [
                "Generation section written: o/r#7",
                "Generation section written: o/r#8 (merged: its last write)",
                "Generation section of o/r#12 not written: HTTP 401",
                "o/r#7 has a Generation heading or table outside the agent-bus markers, written \
                 by hand: tidy it; agent-bus does not edit it",
                "GH_TOKEN is not set: 1 Generation sections are left to write until a command \
                 runs with it",
            ]
        );
        assert_eq!(lines(&synced).len(), 9);
    }
}
