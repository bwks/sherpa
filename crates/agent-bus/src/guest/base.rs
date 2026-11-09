//! Base sessions: a primed Claude session that new tasks fork from, so the task text comes after
//! a prefix the prompt cache already holds. One base per rules variant (GitHub or not) and model,
//! because a fork keeps its base's system prompt and model. Pure logic, except reading AGENTS.md.
//!
//! The git branch and HEAD are not inputs: a commit would otherwise re-prime every base. The
//! git status in a base's context can be out of date, and the rules say so.
//!
//! A base is kept in the guest agent's state file, so its times are wall-clock milliseconds and
//! its inputs are one hash, which compares the same after a restart.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use tokio::fs;

const AGENTS_FILE: &str = "AGENTS.md";
/// The prompt cache keeps a prefix for an hour after it was last used. A base idle for longer is
/// rebuilt, with a margin, rather than found cold by the next task.
const MAX_IDLE: Duration = Duration::from_mins(55);
const SECONDS_PER_DAY: u64 = 24 * 60 * 60;

/// What a base session's context was built from. When any of it changes, a fork would carry a
/// stale prefix, so the base is rebuilt.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(super) struct Inputs {
    /// Hash of the rules text.
    rules: u64,
    /// Hash of AGENTS.md, or `None` when the checkout has none.
    agents: Option<u64>,
    /// The model: a fork keeps its base's model, so another model needs its own base.
    model: String,
    /// The day (UTC, days since the epoch): the date is in Claude's context.
    day: u64,
}

impl Inputs {
    pub(super) fn new(rules: &str, model: &str, agents: Option<&[u8]>, day: u64) -> Self {
        Self {
            rules: hash_of(rules.as_bytes()),
            agents: agents.map(hash_of),
            model: model.to_owned(),
            day,
        }
    }

    /// The inputs of the checkout in `workdir` now. An AGENTS.md that cannot be read is left
    /// out, which only matters as a change.
    pub(super) async fn current(workdir: &Path, rules: &str, model: &str) -> Self {
        let agents = fs::read(workdir.join(AGENTS_FILE)).await.ok();
        let day = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |since| since.as_secs() / SECONDS_PER_DAY);
        Self::new(rules, model, agents.as_deref(), day)
    }

    /// One hash of all the inputs, as hex: what the state file keeps. A TOML integer is signed,
    /// so a `u64` hash would not always fit.
    pub(super) fn digest(&self) -> String {
        let mut hasher = DefaultHasher::new();
        self.hash(&mut hasher);
        format!("{:016x}", hasher.finish())
    }
}

/// A primed base session.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(super) struct Base {
    /// The rules variant: whether the base allows GitHub.
    pub(super) github: bool,
    /// The model it runs on. Empty in a state file from before bases were kept per model; such
    /// a base matches no model and is replaced.
    #[serde(default)]
    pub(super) model: String,
    pub(super) session: String,
    /// The digest of the inputs it was primed from.
    pub(super) inputs: String,
    /// When it was primed, in milliseconds since the Unix epoch.
    pub(super) primed_at_ms: u64,
    /// When it was primed or last forked, in milliseconds since the Unix epoch: each use
    /// refreshes the cached prefix.
    pub(super) last_used_ms: u64,
}

impl Base {
    pub(super) fn new(
        github: bool,
        model: &str,
        session: String,
        inputs: &Inputs,
        now_ms: u64,
    ) -> Self {
        Self {
            github,
            model: model.to_owned(),
            session,
            inputs: inputs.digest(),
            primed_at_ms: now_ms,
            last_used_ms: now_ms,
        }
    }

    /// Whether the base must be built again: its inputs changed, or it sat unused long enough
    /// for the cache to have expired. A clock that went back counts as no time idle.
    pub(super) fn needs_rebuild(&self, current: &Inputs, now_ms: u64) -> bool {
        let idle = Duration::from_millis(now_ms.saturating_sub(self.last_used_ms));
        self.inputs != current.digest() || idle >= MAX_IDLE
    }

    pub(super) fn touch(&mut self, now_ms: u64) {
        self.last_used_ms = now_ms;
    }
}

fn hash_of(bytes: &[u8]) -> u64 {
    let mut hasher = DefaultHasher::new();
    bytes.hash(&mut hasher);
    hasher.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A second short of the idle limit, in milliseconds.
    const ALMOST: u64 = (55 * 60 - 1) * 1000;
    const MAX_IDLE_MS: u64 = 55 * 60 * 1000;
    const NOW: u64 = 1_800_000_000_000;

    fn inputs() -> Inputs {
        Inputs::new("rules", "m", Some(b"agents"), 100)
    }

    fn base(now: u64) -> Base {
        Base::new(false, "m", "b1".to_owned(), &inputs(), now)
    }

    #[test]
    fn unchanged_inputs_keep_the_base() {
        let now = NOW;
        assert!(!base(now).needs_rebuild(&inputs(), now));
        assert!(!base(now).needs_rebuild(&inputs(), now + 60_000));
    }

    #[test]
    fn each_input_changing_rebuilds_the_base() {
        let now = NOW;
        let base = base(now);
        for changed in [
            Inputs::new("other rules", "m", Some(b"agents"), 100),
            Inputs::new("rules", "m", Some(b"agents changed"), 100),
            Inputs::new("rules", "m", None, 100),
            Inputs::new("rules", "m", Some(b"agents"), 101),
            Inputs::new("rules", "other", Some(b"agents"), 100),
        ] {
            assert!(base.needs_rebuild(&changed, now), "{changed:?}");
        }
    }

    #[test]
    fn the_hash_follows_the_content_not_the_instance() {
        assert_eq!(inputs(), inputs());
        assert_eq!(inputs().digest(), inputs().digest());
        assert_eq!(inputs().digest().len(), 16);
        assert_eq!(hash_of(b"AGENTS"), hash_of(b"AGENTS"));
        assert_ne!(hash_of(b"AGENTS"), hash_of(b"AGENTS "));
    }

    #[test]
    fn a_base_idle_for_nearly_the_cache_ttl_is_rebuilt() {
        let now = NOW;
        let base = base(now);
        assert!(!base.needs_rebuild(&inputs(), now + ALMOST));
        assert!(base.needs_rebuild(&inputs(), now + MAX_IDLE_MS));
    }

    #[test]
    fn using_a_base_keeps_it_for_another_ttl() {
        let now = NOW;
        let mut base = base(now);
        let later = now + ALMOST;
        base.touch(later);
        assert!(!base.needs_rebuild(&inputs(), later + ALMOST));
    }

    #[test]
    fn a_clock_that_went_back_does_not_rebuild() {
        let base = base(NOW);
        assert!(!base.needs_rebuild(&inputs(), NOW - 60_000));
    }

    fn git(dir: &Path, args: &[&str]) {
        let status = std::process::Command::new("git")
            .args(["-c", "user.name=t", "-c", "user.email=t@example.com"])
            .args(args)
            .current_dir(dir)
            .status()
            .unwrap();
        assert!(status.success(), "git {args:?}");
    }

    #[tokio::test]
    #[ignore = "runs git in a temporary checkout; run with -- --ignored"]
    async fn a_commit_or_another_branch_keeps_the_base() {
        let dir = std::env::temp_dir().join(format!("agent-bus-base-git-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(AGENTS_FILE), "rules of the checkout").unwrap();
        git(&dir, &["init", "-q", "-b", "main"]);
        git(&dir, &["add", "."]);
        git(&dir, &["commit", "-q", "-m", "one"]);
        let before = Inputs::current(&dir, "rules", "m").await;
        let base = Base::new(false, "m", "b1".to_owned(), &before, NOW);

        std::fs::write(dir.join("file"), "change").unwrap();
        git(&dir, &["add", "."]);
        git(&dir, &["commit", "-q", "-m", "two"]);
        git(&dir, &["checkout", "-q", "-b", "feat"]);
        let after = Inputs::current(&dir, "rules", "m").await;
        assert_eq!(after, before);
        assert!(!base.needs_rebuild(&after, NOW));

        // AGENTS.md is still an input.
        std::fs::write(dir.join(AGENTS_FILE), "other rules").unwrap();
        assert!(base.needs_rebuild(&Inputs::current(&dir, "rules", "m").await, NOW));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
