//! The generation ledger: `$XDG_STATE_HOME/agent-bus/generation.toml` in the guest. For each
//! task it keeps the agent, the branch its last run ended on, the models its runs used and its
//! cumulative usage, so the `## Generation` section of a pull request counts every task that
//! worked on its branch, with or without `--github`.
//!
//! It holds no secrets. It is shared by every guest agent of the user, whatever its name, so it
//! outlives a rename; each change is made under an exclusive lock on `generation.lock` beside it
//! and written atomically. Entries are dropped 30 days after their last run, longer than the
//! stream keeps messages, as pull requests stay open longer than that.

use std::collections::BTreeMap;
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use tracing::instrument;

use super::generation::Row;
use crate::files::{
    PRIVATE_FILE_MODE, lock_exclusive, millis, private_dir, standard_state_dir, write_atomic,
};
use crate::message::TaskUsage;

/// The file in the state directory.
const FILE_NAME: &str = "generation.toml";
/// How long an entry is kept after its last run.
const KEEP: Duration = Duration::from_hours(30 * 24);

/// What the ledger knows of one task.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct Entry {
    /// The guest agent that ran it.
    agent: String,
    /// The branch its last run ended on; `None` while none was known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    branch: Option<String>,
    /// Every model its runs used, sorted.
    #[serde(default)]
    models: Vec<String>,
    /// When its first and last runs were recorded, in milliseconds since the Unix epoch.
    first_at_ms: u64,
    updated_at_ms: u64,
    /// Its totals over all its counted runs.
    usage: TaskUsage,
}

/// The file's content.
#[derive(Debug, Default, PartialEq, Serialize, Deserialize)]
struct Book {
    #[serde(default)]
    tasks: BTreeMap<String, Entry>,
}

/// One finished run, as the ledger records it.
#[derive(Debug)]
pub(super) struct Run<'a> {
    pub(super) task: &'a str,
    pub(super) agent: &'a str,
    /// The branch the run ended on; `None` keeps the one recorded before.
    pub(super) branch: Option<&'a str>,
    /// The models the run used.
    pub(super) models: &'a [String],
    /// The task's totals, this run included.
    pub(super) usage: &'a TaskUsage,
}

/// The ledger file.
#[derive(Debug, Clone)]
pub(super) struct Ledger {
    path: PathBuf,
}

impl Book {
    /// Take in `run`: its totals replace the task's, its models join the task's, and its branch,
    /// when known, replaces the task's.
    fn record(&mut self, run: &Run<'_>, now_ms: u64) {
        let entry = self
            .tasks
            .entry(run.task.to_owned())
            .or_insert_with(|| Entry {
                agent: run.agent.to_owned(),
                branch: None,
                models: Vec::new(),
                first_at_ms: now_ms,
                updated_at_ms: now_ms,
                usage: TaskUsage::default(),
            });
        run.agent.clone_into(&mut entry.agent);
        if let Some(branch) = run.branch {
            entry.branch = Some(branch.to_owned());
        }
        for model in run.models {
            if !entry.models.contains(model) {
                entry.models.push(model.clone());
            }
        }
        entry.models.sort();
        entry.usage = run.usage.clone();
        entry.updated_at_ms = now_ms;
    }

    /// Drop the entries whose last run was longer ago than `KEEP`.
    fn prune(&mut self, now_ms: u64) {
        let cutoff = now_ms.saturating_sub(millis(KEEP));
        self.tasks.retain(|_, entry| entry.updated_at_ms >= cutoff);
    }

    /// The tasks whose last run ended on `branch`, in the order of their first run.
    fn rows_on(&self, branch: &str) -> Vec<Row> {
        let mut entries: Vec<(&String, &Entry)> = self
            .tasks
            .iter()
            .filter(|(_, entry)| entry.branch.as_deref() == Some(branch))
            .collect();
        entries.sort_by_key(|(task, entry)| (entry.first_at_ms, *task));
        entries
            .into_iter()
            .map(|(task, entry)| Row {
                task: task.clone(),
                agent: entry.agent.clone(),
                models: entry.models.clone(),
                usage: entry.usage.clone(),
            })
            .collect()
    }
}

impl Ledger {
    pub(super) fn new(path: PathBuf) -> Self {
        Self { path }
    }

    /// The ledger in the user's state directory.
    ///
    /// # Errors
    ///
    /// Fails when neither `XDG_STATE_HOME` nor `HOME` gives a state directory.
    pub(super) fn standard() -> Result<Self> {
        Ok(Self::new(standard_state_dir()?.join(FILE_NAME)))
    }

    pub(super) fn path(&self) -> &Path {
        &self.path
    }

    /// Record `run`, drop the old entries and save, under the ledger's lock. Returns the rows of
    /// the tasks on the run's branch, its own included; none when its branch is not known.
    ///
    /// # Errors
    ///
    /// Fails when the lock cannot be taken or the file cannot be written.
    #[instrument(level = "debug", skip_all, fields(path = %self.path.display(), task = %run.task))]
    pub(super) fn record(&self, run: &Run<'_>, now_ms: u64) -> Result<Vec<Row>> {
        let dir = self
            .path
            .parent()
            .context("The generation ledger has no directory")?;
        private_dir(dir)?;
        let _lock = lock_exclusive(&self.path.with_extension("lock"))?;
        let mut book = self.load(now_ms);
        book.record(run, now_ms);
        book.prune(now_ms);
        let text = toml::to_string(&book).context("Unable to write the generation ledger")?;
        write_atomic(&self.path, &text, PRIVATE_FILE_MODE)?;
        Ok(run
            .branch
            .map(|branch| book.rows_on(branch))
            .unwrap_or_default())
    }

    /// The ledger's content. A missing file is empty; one that cannot be read or parsed is
    /// reported, moved aside to `<file>.corrupt-<unix seconds>`, and is empty too.
    fn load(&self, now_ms: u64) -> Book {
        let parsed = match fs::read_to_string(&self.path) {
            Ok(text) => toml::from_str::<Book>(&text).map_err(anyhow::Error::from),
            Err(error) if error.kind() == ErrorKind::NotFound => return Book::default(),
            Err(error) => Err(error.into()),
        };
        parsed.unwrap_or_else(|error| {
            let mut aside = self.path.as_os_str().to_owned();
            aside.push(format!(".corrupt-{}", now_ms / 1000));
            let aside = PathBuf::from(aside);
            eprintln!(
                "Unable to read the generation ledger {}: {error:#}: moving it to {} and starting fresh",
                self.path.display(),
                aside.display()
            );
            if let Err(error) = fs::rename(&self.path, &aside) {
                eprintln!("Unable to move the generation ledger aside: {error}");
            }
            Book::default()
        })
    }
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt;

    use super::*;

    const NOW: u64 = 1_800_000_000_000;
    const DAY: u64 = 24 * 60 * 60 * 1000;

    fn scratch(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("agent-bus-ledger-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    fn usage(runs: u64, cost_usd: f64) -> TaskUsage {
        TaskUsage {
            runs,
            cost_usd,
            input: runs * 10,
            ..TaskUsage::default()
        }
    }

    fn models(names: &[&str]) -> Vec<String> {
        names.iter().map(|name| (*name).to_owned()).collect()
    }

    #[test]
    fn a_run_replaces_the_totals_moves_the_branch_and_adds_models() {
        let mut book = Book::default();
        let opus = models(&["claude-opus-5-5"]);
        book.record(
            &Run {
                task: "t1",
                agent: "dev01",
                branch: Some("main"),
                models: &opus,
                usage: &usage(1, 0.5),
            },
            NOW,
        );
        let both = models(&["claude-opus-5-5", "claude-haiku-5-5"]);
        book.record(
            &Run {
                task: "t1",
                agent: "dev01",
                branch: Some("feat/x"),
                models: &both,
                usage: &usage(2, 1.25),
            },
            NOW + 1,
        );
        assert_eq!(book.rows_on("main"), Vec::<Row>::new());
        let rows = book.rows_on("feat/x");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].models, ["claude-haiku-5-5", "claude-opus-5-5"]);
        assert_eq!(rows[0].usage, usage(2, 1.25));
        // An unknown branch keeps the one recorded.
        book.record(
            &Run {
                task: "t1",
                agent: "dev01",
                branch: None,
                models: &opus,
                usage: &usage(3, 2.0),
            },
            NOW + 2,
        );
        assert_eq!(book.rows_on("feat/x")[0].usage.runs, 3);
        assert_eq!(book.tasks["t1"].first_at_ms, NOW);
    }

    #[test]
    fn rows_on_a_branch_come_in_the_order_of_their_first_run() {
        let mut book = Book::default();
        let none = Vec::new();
        for (task, at, branch) in [("b", NOW, "x"), ("a", NOW + 5, "x"), ("c", NOW + 1, "y")] {
            book.record(
                &Run {
                    task,
                    agent: "dev01",
                    branch: Some(branch),
                    models: &none,
                    usage: &usage(1, 0.0),
                },
                at,
            );
        }
        let tasks: Vec<String> = book.rows_on("x").into_iter().map(|row| row.task).collect();
        assert_eq!(tasks, ["b", "a"]);
    }

    #[test]
    fn entries_older_than_30_days_are_dropped() {
        let mut book = Book::default();
        let none = Vec::new();
        for (task, at) in [("old", NOW - 31 * DAY), ("recent", NOW - 29 * DAY)] {
            book.record(
                &Run {
                    task,
                    agent: "dev01",
                    branch: Some("x"),
                    models: &none,
                    usage: &usage(1, 0.0),
                },
                at,
            );
        }
        book.prune(NOW);
        assert_eq!(book.tasks.keys().collect::<Vec<_>>(), ["recent"]);
    }

    #[test]
    fn the_file_round_trips_0600_and_shares_tasks_of_every_agent() {
        let dir = scratch("file");
        let ledger = Ledger::new(dir.join("generation.toml"));
        let opus = models(&["claude-opus-5-5"]);
        let first = Run {
            task: "t1",
            agent: "dev",
            branch: Some("feat/x"),
            models: &opus,
            usage: &usage(1, 0.25),
        };
        assert_eq!(ledger.record(&first, NOW).unwrap().len(), 1);
        let second = Run {
            task: "t2",
            agent: "dev01",
            branch: Some("feat/x"),
            models: &opus,
            usage: &usage(1, 0.5),
        };
        let rows = ledger.record(&second, NOW + 1).unwrap();
        let agents: Vec<&str> = rows.iter().map(|row| row.agent.as_str()).collect();
        assert_eq!(agents, ["dev", "dev01"]);
        let detached = Run {
            branch: None,
            ..second
        };
        assert_eq!(
            ledger.record(&detached, NOW + 2).unwrap(),
            Vec::<Row>::new()
        );
        let mode = fs::metadata(ledger.path()).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, PRIVATE_FILE_MODE);
        let text = fs::read_to_string(ledger.path()).unwrap();
        assert!(text.contains("[tasks.t1]"), "{text}");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_corrupt_file_is_moved_aside_and_the_ledger_starts_fresh() {
        let dir = scratch("corrupt");
        fs::create_dir_all(&dir).unwrap();
        let ledger = Ledger::new(dir.join("generation.toml"));
        fs::write(ledger.path(), "not = [toml").unwrap();
        let none = Vec::new();
        let run = Run {
            task: "t1",
            agent: "dev01",
            branch: Some("x"),
            models: &none,
            usage: &usage(1, 0.0),
        };
        assert_eq!(ledger.record(&run, NOW).unwrap().len(), 1);
        assert!(
            dir.join(format!("generation.toml.corrupt-{}", NOW / 1000))
                .exists()
        );
        fs::remove_dir_all(&dir).unwrap();
    }
}
