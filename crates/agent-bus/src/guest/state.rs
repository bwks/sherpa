//! The guest agent's state, kept across restarts in `$XDG_STATE_HOME/agent-bus/guest-<name>.toml`:
//! the Claude session and the totals of each task, the queue, the running task and the primed
//! bases. Claude Code keeps the sessions themselves; this file only says which is whose.
//!
//! The file is TOML, like every other file of the project. The one value TOML cannot hold as an
//! integer, a `u64` hash, is kept as hex. It is written after every change, to a temporary file
//! that is then renamed over it, so a crash leaves the old state or the new one, never half of
//! one. It holds no secrets; the only message text is that of the queued tasks and answers,
//! which they need to run.

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use tracing::instrument;

use super::base::Base;
use super::engine::Session;
use crate::files::{
    FILE_RETENTION, PRIVATE_FILE_MODE, millis, private_dir, standard_state_dir, write_atomic,
};
use crate::message::{Kind, Message, TaskUsage};

/// Where a task's session's API time stood after its last run. The figure counts the whole
/// session, so a run's own is the increase.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(super) struct ApiMark {
    pub(super) session: String,
    pub(super) api_ms: u64,
}

/// What the guest agent knows of one task.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub(super) struct TaskRecord {
    /// The runs so far, for the record of result events.
    #[serde(default)]
    pub(super) runs: u32,
    /// When its last run ended, in milliseconds since the Unix epoch. `None` while the first
    /// runs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) finished_at_ms: Option<u64>,
    /// Its Claude session and whether it may use GitHub, once Claude Code named the session.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) session: Option<Session>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) api: Option<ApiMark>,
    /// What it has used, over the runs that sent a result event.
    #[serde(default)]
    pub(super) usage: TaskUsage,
}

/// A task or an answer waiting in the queue: what it needs to run.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(super) struct Queued {
    pub(super) task: String,
    pub(super) from: String,
    pub(super) kind: Kind,
    #[serde(default)]
    pub(super) github: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) model: Option<String>,
    pub(super) text: String,
}

/// The state file's content.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub(super) struct Saved {
    /// The task Claude Code was working on when the file was written.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) running: Option<String>,
    /// The waiting tasks and answers, in order.
    #[serde(default)]
    pub(super) queue: Vec<Queued>,
    #[serde(default)]
    pub(super) bases: Vec<Base>,
    #[serde(default)]
    pub(super) tasks: BTreeMap<String, TaskRecord>,
}

/// What the guest agent starts from after a restart.
#[derive(Debug, Default)]
pub(super) struct Restart {
    pub(super) tasks: HashMap<String, TaskRecord>,
    pub(super) queue: VecDeque<Message>,
    /// By rules variant (whether it allows GitHub) and model.
    pub(super) bases: HashMap<(bool, String), Base>,
    /// The task that was running when the agent stopped without reporting it: it is reported
    /// failed, and its session kept for an answer.
    pub(super) interrupted: Option<String>,
}

/// The state file.
#[derive(Debug, Clone)]
pub(super) struct StateFile {
    path: PathBuf,
}

impl Queued {
    pub(super) fn of(message: &Message) -> Self {
        Self {
            task: message.task.clone(),
            from: message.from.clone(),
            kind: message.kind,
            github: message.github,
            model: message.model.clone(),
            text: message.text.clone(),
        }
    }

    /// The message again, addressed to `to`, the guest agent.
    pub(super) fn into_message(self, to: &str) -> Message {
        Message {
            task: self.task,
            from: self.from,
            to: to.to_owned(),
            kind: self.kind,
            text: self.text,
            usage: None,
            task_usage: None,
            github: self.github,
            model: self.model,
            work: None,
            ended: None,
        }
    }
}

impl StateFile {
    pub(super) fn new(path: PathBuf) -> Self {
        Self { path }
    }

    /// The file of guest agent `name` in the user's state directory.
    ///
    /// # Errors
    ///
    /// Fails when neither `XDG_STATE_HOME` nor `HOME` gives a state directory.
    pub(super) fn standard(name: &str) -> Result<Self> {
        Ok(Self::new(
            standard_state_dir()?.join(format!("guest-{name}.toml")),
        ))
    }

    pub(super) fn path(&self) -> &Path {
        &self.path
    }

    /// The saved state. A missing file is a fresh start. A file that cannot be read or parsed
    /// is reported, moved aside to `<file>.corrupt-<unix seconds>`, and is a fresh start too.
    #[instrument(level = "debug", skip_all, fields(path = %self.path.display()))]
    pub(super) fn load(&self, now_ms: u64) -> Saved {
        let parsed = match fs::read_to_string(&self.path) {
            Ok(text) => toml::from_str::<Saved>(&text).map_err(anyhow::Error::from),
            Err(error) if error.kind() == ErrorKind::NotFound => return Saved::default(),
            Err(error) => Err(error.into()),
        };
        match parsed {
            Ok(saved) => saved,
            Err(error) => {
                let aside = self.corrupt_path(now_ms);
                eprintln!(
                    "Unable to read the state file {}: {error:#}: moving it to {} and starting fresh",
                    self.path.display(),
                    aside.display()
                );
                if let Err(error) = fs::rename(&self.path, &aside) {
                    eprintln!("Unable to move the state file aside: {error}");
                }
                Saved::default()
            }
        }
    }

    fn corrupt_path(&self, now_ms: u64) -> PathBuf {
        let mut name = self.path.as_os_str().to_owned();
        name.push(format!(".corrupt-{}", now_ms / 1000));
        PathBuf::from(name)
    }

    /// Write the state: to a temporary file next to it, flushed to disk, then renamed over it.
    ///
    /// # Errors
    ///
    /// Fails when the state cannot be rendered or the directory or the file cannot be written.
    #[instrument(level = "debug", skip_all, fields(path = %self.path.display()))]
    pub(super) fn save(&self, saved: &Saved) -> Result<()> {
        let text = toml::to_string(saved).context("Unable to render the guest agent's state")?;
        let dir = self
            .path
            .parent()
            .context("The state file has no directory")?;
        private_dir(dir)?;
        write_atomic(&self.path, &text, PRIVATE_FILE_MODE)
    }
}

/// What to do with the saved state on start: the queue goes on in its order, a task that was
/// running is to be reported failed (its session kept), the bases come back to be checked
/// against the inputs at the next fork, and old tasks are dropped.
pub(super) fn restart(saved: Saved, name: &str, now_ms: u64) -> Restart {
    let mut tasks: HashMap<String, TaskRecord> = saved.tasks.into_iter().collect();
    if let Some(task) = &saved.running {
        tasks.entry(task.clone()).or_default().finished_at_ms = Some(now_ms);
    }
    let queue: VecDeque<Message> = saved
        .queue
        .into_iter()
        .map(|queued| queued.into_message(name))
        .collect();
    prune(&mut tasks, now_ms, |task| {
        queue.iter().any(|queued| queued.task == task)
    });
    Restart {
        tasks,
        queue,
        bases: saved
            .bases
            .into_iter()
            .map(|base| ((base.github, base.model.clone()), base))
            .collect(),
        interrupted: saved.running,
    }
}

/// Drop the tasks that finished longer ago than `FILE_RETENTION`, unless `keep` says a
/// task is still needed (queued or running).
pub(super) fn prune(
    tasks: &mut HashMap<String, TaskRecord>,
    now_ms: u64,
    keep: impl Fn(&str) -> bool,
) {
    let cutoff = now_ms.saturating_sub(millis(FILE_RETENTION));
    tasks.retain(|task, record| keep(task) || record.finished_at_ms.is_none_or(|at| at >= cutoff));
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::env;
    use std::os::unix::fs::PermissionsExt;

    use crate::files::DIR_MODE;
    use crate::guest::base::Inputs;

    const NOW: u64 = 1_800_000_000_000;
    const DAY: u64 = 24 * 60 * 60 * 1000;

    fn dir(label: &str) -> PathBuf {
        let dir = env::temp_dir().join(format!("agent-bus-state-{label}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    fn queued(task: &str, kind: Kind, text: &str) -> Queued {
        Queued {
            task: task.to_owned(),
            from: "host".to_owned(),
            kind,
            github: false,
            model: None,
            text: text.to_owned(),
        }
    }

    fn finished(at: u64) -> TaskRecord {
        TaskRecord {
            runs: 1,
            finished_at_ms: Some(at),
            session: Some(Session {
                id: "s".to_owned(),
                github: false,
                model: None,
            }),
            ..TaskRecord::default()
        }
    }

    fn inputs(agents: &str) -> Inputs {
        Inputs::new("rules", "m", Some(agents.as_bytes()), 100)
    }

    fn full() -> Saved {
        let mut tasks = BTreeMap::new();
        tasks.insert(
            "t1".to_owned(),
            TaskRecord {
                runs: 2,
                finished_at_ms: Some(NOW),
                session: Some(Session {
                    id: "sess-1".to_owned(),
                    github: true,
                    model: Some("claude-sonnet-5-5".to_owned()),
                }),
                api: Some(ApiMark {
                    session: "sess-1".to_owned(),
                    api_ms: 1500,
                }),
                usage: TaskUsage {
                    runs: 2,
                    cost_usd: 0.057,
                    cache_read: 18775,
                    ..TaskUsage::default()
                },
            },
        );
        // A task whose priming runs are counted but whose session is not named yet.
        tasks.insert(
            "t2".to_owned(),
            TaskRecord {
                usage: TaskUsage {
                    runs: 1,
                    ..TaskUsage::default()
                },
                ..TaskRecord::default()
            },
        );
        Saved {
            running: Some("t2".to_owned()),
            queue: vec![
                Queued {
                    model: Some("haiku".to_owned()),
                    ..queued("t3", Kind::Task, "line one\nline \"two\"\n")
                },
                queued("t1", Kind::Answer, "go on"),
            ],
            bases: vec![
                Base::new(false, "m", "base-0".to_owned(), &inputs("abc"), NOW),
                Base::new(true, "m", "base-1".to_owned(), &inputs("abc"), NOW),
                Base::new(true, "other", "base-2".to_owned(), &inputs("abc"), NOW),
            ],
            tasks,
        }
    }

    #[test]
    fn the_state_survives_a_round_trip() {
        let dir = dir("round");
        let file = StateFile::new(dir.join("agent-bus").join("guest-dev.toml"));
        for saved in [Saved::default(), full()] {
            file.save(&saved).unwrap();
            assert_eq!(file.load(NOW), saved);
        }
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_write_is_atomic_and_private() {
        let dir = dir("atomic");
        let file = StateFile::new(dir.join("agent-bus").join("guest-dev.toml"));
        file.save(&full()).unwrap();
        // A second write replaces the file whole, and leaves no temporary file behind.
        let mut changed = full();
        changed.queue.clear();
        file.save(&changed).unwrap();
        assert_eq!(file.load(NOW), changed);
        let names: Vec<String> = fs::read_dir(dir.join("agent-bus"))
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, ["guest-dev.toml"]);
        // A leftover temporary file from a crash mid-write does not get in the way.
        fs::write(dir.join("agent-bus").join("guest-dev.toml.tmp"), "half").unwrap();
        file.save(&full()).unwrap();
        assert_eq!(file.load(NOW), full());

        let mode = |path: &Path| fs::metadata(path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(file.path()), PRIVATE_FILE_MODE);
        assert_eq!(mode(&dir.join("agent-bus")), DIR_MODE);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_missing_file_is_a_fresh_start() {
        let file = StateFile::new(dir("missing").join("guest-dev.toml"));
        assert_eq!(file.load(NOW), Saved::default());
    }

    #[test]
    fn a_corrupt_file_is_moved_aside_and_the_agent_starts_fresh() {
        let dir = dir("corrupt");
        fs::create_dir_all(&dir).unwrap();
        let file = StateFile::new(dir.join("guest-dev.toml"));
        for text in ["queue = [", "running = 5", "[tasks.t1]\nruns = -1\n"] {
            fs::write(file.path(), text).unwrap();
            assert_eq!(file.load(NOW), Saved::default(), "{text}");
            assert!(!file.path().exists());
            let aside = dir.join(format!("guest-dev.toml.corrupt-{}", NOW / 1000));
            assert_eq!(fs::read_to_string(&aside).unwrap(), text);
            fs::remove_file(&aside).unwrap();
        }
        // A file that is not text at all.
        fs::write(file.path(), [0xff, 0xfe, 0x00]).unwrap();
        assert_eq!(file.load(NOW), Saved::default());
        assert!(!file.path().exists());
        // The fresh state is written over it as usual.
        file.save(&full()).unwrap();
        assert_eq!(file.load(NOW), full());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_queue_resumes_in_its_order() {
        let restart = restart(full(), "dev", NOW);
        let queue: Vec<(&str, Kind, &str)> = restart
            .queue
            .iter()
            .map(|message| (message.task.as_str(), message.kind, message.to.as_str()))
            .collect();
        assert_eq!(
            queue,
            [("t3", Kind::Task, "dev"), ("t1", Kind::Answer, "dev")]
        );
        assert_eq!(restart.queue[0].text, "line one\nline \"two\"\n");
        assert_eq!(restart.queue[0].model.as_deref(), Some("haiku"));
        assert_eq!(restart.queue[1].model, None);
    }

    #[test]
    fn the_running_task_is_failed_and_its_session_kept() {
        let mut saved = full();
        saved.running = Some("t1".to_owned());
        let restart = restart(saved, "dev", NOW + DAY);
        assert_eq!(restart.interrupted.as_deref(), Some("t1"));
        let record = &restart.tasks["t1"];
        assert_eq!(record.session.as_ref().unwrap().id, "sess-1");
        assert!(record.session.as_ref().unwrap().github);
        assert_eq!(record.usage.runs, 2);
        // It counts as finished now, for the retention.
        assert_eq!(record.finished_at_ms, Some(NOW + DAY));

        let mut idle = full();
        idle.running = None;
        assert_eq!(restart_of(idle).interrupted, None);
    }

    fn restart_of(saved: Saved) -> Restart {
        restart(saved, "dev", NOW)
    }

    #[test]
    fn bases_are_reused_while_their_inputs_match() {
        let restart = restart_of(full());
        // One per rules variant and model.
        assert_eq!(restart.bases.len(), 3);
        assert_eq!(restart.bases[&(true, "other".to_owned())].session, "base-2");
        let base = &restart.bases[&(true, "m".to_owned())];
        assert_eq!(base.session, "base-1");
        assert!(!base.needs_rebuild(&inputs("abc"), NOW + 60_000));
        // AGENTS.md changed since it was primed: rebuilt.
        assert!(base.needs_rebuild(&inputs("def"), NOW + 60_000));
        // Idle across a long restart: rebuilt, the cache has expired.
        assert!(base.needs_rebuild(&inputs("abc"), NOW + DAY));
    }

    #[test]
    fn tasks_finished_over_a_week_ago_are_dropped() {
        let mut tasks = HashMap::from([
            ("old".to_owned(), finished(NOW - 8 * DAY)),
            ("recent".to_owned(), finished(NOW - 6 * DAY)),
            ("queued".to_owned(), finished(NOW - 30 * DAY)),
            ("unfinished".to_owned(), TaskRecord::default()),
        ]);
        prune(&mut tasks, NOW, |task| task == "queued");
        let mut left: Vec<&str> = tasks.keys().map(String::as_str).collect();
        left.sort_unstable();
        assert_eq!(left, ["queued", "recent", "unfinished"]);
    }

    #[test]
    fn a_restart_prunes_but_keeps_what_the_queue_needs() {
        let mut saved = full();
        saved.running = None;
        saved
            .tasks
            .insert("old".to_owned(), finished(NOW - 8 * DAY));
        // t1 finished long ago, but an answer for it is queued.
        saved.tasks.get_mut("t1").unwrap().finished_at_ms = Some(NOW - 8 * DAY);
        let restart = restart_of(saved);
        assert!(!restart.tasks.contains_key("old"));
        assert!(restart.tasks.contains_key("t1"));
    }
}
