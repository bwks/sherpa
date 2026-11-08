//! Base sessions: a primed Claude session that new tasks fork from, so the task text comes after
//! a prefix the prompt cache already holds. One base per rules variant (GitHub or not), because a
//! fork keeps its base's system prompt. Pure logic, except reading the checkout's inputs.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::Path;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use tokio::fs;
use tokio::process::Command;

const AGENTS_FILE: &str = "AGENTS.md";
/// The prompt cache keeps a prefix for an hour after it was last used. A base idle for longer is
/// rebuilt, with a margin, rather than found cold by the next task.
const MAX_IDLE: Duration = Duration::from_mins(55);
const SECONDS_PER_DAY: u64 = 24 * 60 * 60;

/// What a base session's context was built from. When any of it changes, a fork would carry a
/// stale prefix, so the base is rebuilt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Inputs {
    /// Hash of the rules text.
    rules: u64,
    /// Hash of AGENTS.md, or `None` when the checkout has none.
    agents: Option<u64>,
    branch: String,
    head: String,
    /// The model: a fork keeps its base's model, so another model needs its own base.
    model: String,
    /// The day (UTC, days since the epoch): the date is in Claude's context.
    day: u64,
}

impl Inputs {
    pub(super) fn new(
        rules: &str,
        model: &str,
        agents: Option<&[u8]>,
        branch: &str,
        head: &str,
        day: u64,
    ) -> Self {
        Self {
            rules: hash_of(rules.as_bytes()),
            agents: agents.map(hash_of),
            branch: branch.to_owned(),
            head: head.to_owned(),
            model: model.to_owned(),
            day,
        }
    }

    /// The inputs of the checkout in `workdir` now. What cannot be read (no git, no AGENTS.md)
    /// is left empty, which only matters as a change.
    pub(super) async fn current(workdir: &Path, rules: &str, model: &str) -> Self {
        let agents = fs::read(workdir.join(AGENTS_FILE)).await.ok();
        let branch = git(workdir, &["rev-parse", "--abbrev-ref", "HEAD"]).await;
        let head = git(workdir, &["rev-parse", "HEAD"]).await;
        let day = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |since| since.as_secs() / SECONDS_PER_DAY);
        Self::new(rules, model, agents.as_deref(), &branch, &head, day)
    }
}

/// A primed base session.
#[derive(Debug)]
pub(super) struct Base {
    pub(super) session: String,
    inputs: Inputs,
    /// When the session was primed or last forked: each use refreshes the cached prefix.
    last_used: Instant,
}

impl Base {
    pub(super) fn new(session: String, inputs: Inputs, now: Instant) -> Self {
        Self {
            session,
            inputs,
            last_used: now,
        }
    }

    /// Whether the base must be built again: its inputs changed, or it sat unused long enough
    /// for the cache to have expired.
    pub(super) fn needs_rebuild(&self, current: &Inputs, now: Instant) -> bool {
        self.inputs != *current || now.saturating_duration_since(self.last_used) >= MAX_IDLE
    }

    pub(super) fn touch(&mut self, now: Instant) {
        self.last_used = now;
    }
}

fn hash_of(bytes: &[u8]) -> u64 {
    let mut hasher = DefaultHasher::new();
    bytes.hash(&mut hasher);
    hasher.finish()
}

/// The trimmed output of a git command, empty when it fails.
async fn git(workdir: &Path, args: &[&str]) -> String {
    Command::new("git")
        .args(args)
        .current_dir(workdir)
        .output()
        .await
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_owned())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A second short of the idle limit.
    const ALMOST: Duration = Duration::from_secs(55 * 60 - 1);

    fn inputs() -> Inputs {
        Inputs::new("rules", "m", Some(b"agents"), "main", "abc", 100)
    }

    fn base(now: Instant) -> Base {
        Base::new("b1".to_owned(), inputs(), now)
    }

    #[test]
    fn unchanged_inputs_keep_the_base() {
        let now = Instant::now();
        assert!(!base(now).needs_rebuild(&inputs(), now));
        assert!(!base(now).needs_rebuild(&inputs(), now + Duration::from_secs(60)));
    }

    #[test]
    fn each_input_changing_rebuilds_the_base() {
        let now = Instant::now();
        let base = base(now);
        for changed in [
            Inputs::new("other rules", "m", Some(b"agents"), "main", "abc", 100),
            Inputs::new("rules", "m", Some(b"agents changed"), "main", "abc", 100),
            Inputs::new("rules", "m", None, "main", "abc", 100),
            Inputs::new("rules", "m", Some(b"agents"), "feat", "abc", 100),
            Inputs::new("rules", "m", Some(b"agents"), "main", "def", 100),
            Inputs::new("rules", "m", Some(b"agents"), "main", "abc", 101),
            Inputs::new("rules", "other", Some(b"agents"), "main", "abc", 100),
        ] {
            assert!(base.needs_rebuild(&changed, now), "{changed:?}");
        }
    }

    #[test]
    fn the_hash_follows_the_content_not_the_instance() {
        assert_eq!(inputs(), inputs());
        assert_eq!(hash_of(b"AGENTS"), hash_of(b"AGENTS"));
        assert_ne!(hash_of(b"AGENTS"), hash_of(b"AGENTS "));
    }

    #[test]
    fn a_base_idle_for_nearly_the_cache_ttl_is_rebuilt() {
        let now = Instant::now();
        let base = base(now);
        assert!(!base.needs_rebuild(&inputs(), now + ALMOST));
        assert!(base.needs_rebuild(&inputs(), now + MAX_IDLE));
    }

    #[test]
    fn using_a_base_keeps_it_for_another_ttl() {
        let now = Instant::now();
        let mut base = base(now);
        let later = now + ALMOST;
        base.touch(later);
        assert!(!base.needs_rebuild(&inputs(), later + ALMOST));
    }
}
