//! Claude Code as a process: its command, environment and deny lists, the rules it is given, and
//! its process group.

use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use nix::errno::Errno;
use nix::sys::signal::{Signal, killpg};
use nix::unistd::Pid;
use tokio::process::{Child, Command};
use tokio::time::timeout;

use crate::keys::SEED_VARIABLE;

pub(super) const CLAUDE: &str = "claude";
/// The model guests run Claude Code on unless told otherwise.
pub const DEFAULT_MODEL: &str = "claude-opus-5-5";
/// How long Claude Code's process group has to exit after SIGTERM before SIGKILL.
const KILL_GRACE: Duration = Duration::from_secs(2);
/// Removed from Claude Code's environment, so it can only use the subscription.
const API_KEY_VARIABLE: &str = "ANTHROPIC_API_KEY";
/// GitHub tokens. Claude Code only has them for a task that was given GitHub access, so without
/// it `curl` with the token cannot work around the deny list either.
const GITHUB_TOKEN_VARIABLES: [&str; 3] = ["GH_TOKEN", "GITHUB_TOKEN", "GH_ENTERPRISE_TOKEN"];
/// Permission checks are skipped: the guest is a disposable lab VM and the agent has root in it.
/// These are still refused. Committing, pushing and GitHub are the user's to decide; the host
/// agent asks them. Commands that throw away uncommitted work (stash, restore, clean, and checkout
/// of paths) are refused as well: the working tree is the only copy of a branch's uncommitted
/// changes.
///
/// The patterns match command text, so they cannot tell `git checkout <branch>` from
/// `git checkout <path>`. Only the forms that name paths unambiguously are refused: `--`, `.`,
/// and `-f`/`--force`. `git checkout <branch>` keeps working.
const DISALLOWED_TOOLS: &str = "Bash(git commit:*),Bash(git push:*),Bash(git reset:*),Bash(gh:*),\
Bash(git stash:*),Bash(git restore:*),Bash(git clean:*),Bash(git checkout*-- *),\
Bash(git checkout* --),Bash(git checkout* .),Bash(git checkout* ./*),Bash(git checkout*--force*),\
Bash(git checkout* -f),Bash(git checkout* -f *)";
/// The refused commands for a task given GitHub access: it may commit, push and open pull
/// requests, but merging and destructive operations stay with the user.
///
/// With the token present this list is a guardrail, not a security boundary: it matches command
/// text, and a command written another way (`curl` with the token, a script, an alias) is not
/// caught. The token's own scopes are the boundary.
const GITHUB_DISALLOWED_TOOLS: &str = "Bash(gh pr merge:*),Bash(gh api:*),Bash(gh repo delete:*),\
Bash(gh release:*),Bash(git reset:*),Bash(git push*--force*),Bash(git push* -f),\
Bash(git push* -f *),Bash(git push* --delete*),Bash(git push* -d),Bash(git push* -d *),\
Bash(git push* :*),Bash(git push* +*),Bash(git branch* -D*),Bash(git stash:*),\
Bash(git restore:*),Bash(git clean:*),Bash(git checkout*-- *),Bash(git checkout* --),\
Bash(git checkout* .),Bash(git checkout* ./*),Bash(git checkout*--force*),Bash(git checkout* -f),\
Bash(git checkout* -f *)";
const RULES: &str = "You are a guest agent in a Sherpa lab VM. A host agent sends you tasks over \
a message bus; you do the work in this checkout and report back. Follow AGENTS.md. Do not add \
dependencies. If you need a decision before you can go on, end your reply with one line starting \
with `QUESTION:` and stop; the answer comes as your next message. Otherwise end with a short \
summary of what you did and the results. The git status in your context may be out of date; run \
git status yourself when it matters. Never use git stash, git restore, git clean or \
git checkout -- <path>: the working tree is the only copy of uncommitted work. To see how things \
were before, use `git show HEAD:<path>` or `git diff`.";
/// What a base session is primed with. Nothing is done with the reply.
pub(super) const PRIME_PROMPT: &str = "You are a guest agent. Tasks will follow, one per session. \
Reply with exactly: ready.";
/// The second priming turn, a resume of the base. Claude Code seems to rearrange its context
/// after the first turn and settle after the second, so a fork should come after both.
pub(super) const PRIME_CONFIRM: &str = "Confirm once more: reply with exactly: ready.";
/// Added to the rules for a task without GitHub access.
const RULES_NO_GITHUB: &str = " This task has no GitHub access. Do not commit, push, merge, open \
pull requests or run gh.";
/// Added to the rules for a task with GitHub access.
const RULES_GITHUB: &str = " This task may use GitHub. Do only what the task asks. Commit \
messages end with the Co-Authored-By line from your own attribution instructions, never one \
copied from the task text, and have no session links and no \"Generated with\" footers. Never merge pull requests or delete anything; the user does that.";

/// Claude Code's process group. Dropping it while armed kills everything in the group, so no
/// path out of a run can leave Claude Code or the commands it started behind.
pub(super) struct Group {
    pgid: Option<Pid>,
}

impl Group {
    pub(super) fn new(pid: Option<u32>) -> Self {
        Self {
            pgid: pid
                .and_then(|pid| i32::try_from(pid).ok())
                .map(Pid::from_raw),
        }
    }

    fn signal(&self, signal: Signal) {
        if let Some(pgid) = self.pgid {
            match killpg(pgid, signal) {
                // Nothing in the group is left.
                Ok(()) | Err(Errno::ESRCH) => {}
                Err(error) => eprintln!("Unable to signal Claude Code's process group: {error}"),
            }
        }
    }

    pub(super) fn disarm(&mut self) {
        self.pgid = None;
    }
}

impl Drop for Group {
    fn drop(&mut self) {
        self.signal(Signal::SIGKILL);
    }
}

/// Stop Claude Code and everything it started: SIGTERM to the group, SIGKILL after the grace
/// period, and SIGKILL once more for any command that ignored SIGTERM after Claude Code left.
pub(super) async fn terminate(child: &mut Child, group: &mut Group) {
    group.signal(Signal::SIGTERM);
    if timeout(KILL_GRACE, child.wait()).await.is_err() {
        eprintln!("Claude Code ignored SIGTERM: killing it");
    }
    group.signal(Signal::SIGKILL);
    if let Err(error) = child.wait().await {
        eprintln!("Claude Code did not exit: {error}");
    }
    group.disarm();
}

/// How a Claude Code run relates to the sessions before it.
#[derive(Debug, Clone, PartialEq)]
pub(super) enum Launch {
    /// A new session: a base session being primed, or a task whose fork failed.
    Fresh,
    /// A new task: a copy of the base session with this id, so the task text comes after the
    /// base's cached prefix. The base itself is never resumed.
    Fork(String),
    /// An answer: the task's own session with this id.
    Resume(String),
}

/// The rules Claude Code is given, with or without GitHub access.
pub(super) fn rules(github: bool) -> String {
    if github {
        format!("{RULES}{RULES_GITHUB}")
    } else {
        format!("{RULES}{RULES_NO_GITHUB}")
    }
}

/// The Claude Code command for a run: its tool limits, its rules, its session and its environment.
pub(super) fn claude_command(
    program: &str,
    workdir: &Path,
    model: &str,
    github: bool,
    launch: &Launch,
) -> Command {
    let disallowed = if github {
        GITHUB_DISALLOWED_TOOLS
    } else {
        DISALLOWED_TOOLS
    };
    let rules = rules(github);
    let mut command = Command::new(program);
    command
        .args(["-p", "--output-format", "stream-json", "--verbose"])
        .args(["--model", model])
        .arg("--dangerously-skip-permissions")
        .arg(format!("--disallowedTools={disallowed}"))
        .args(["--append-system-prompt", &rules]);
    match launch {
        Launch::Fresh => {}
        Launch::Fork(base) => {
            command.args(["--resume", base, "--fork-session"]);
        }
        Launch::Resume(session) => {
            command.args(["--resume", session]);
        }
    }
    command
        .current_dir(workdir)
        // Its own process group, so a cancel reaches the commands it started too.
        .process_group(0)
        .kill_on_drop(true)
        .env_remove(API_KEY_VARIABLE)
        .env_remove(SEED_VARIABLE)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped());
    if !github {
        for variable in GITHUB_TOKEN_VARIABLES {
            command.env_remove(variable);
        }
    }
    command
}

#[cfg(test)]
mod tests {
    use super::*;

    const MODEL: &str = "test-model";

    fn removed(command: &Command) -> Vec<String> {
        command
            .as_std()
            .get_envs()
            .filter(|(_, value)| value.is_none())
            .map(|(key, _)| key.to_string_lossy().into_owned())
            .collect()
    }

    fn args(command: &Command) -> Vec<String> {
        command
            .as_std()
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect()
    }

    #[test]
    fn a_task_without_github_loses_the_tokens_and_keeps_the_full_deny_list() {
        let command = claude_command(CLAUDE, Path::new("/work"), MODEL, false, &Launch::Fresh);
        let removed = removed(&command);
        for variable in [
            "ANTHROPIC_API_KEY",
            "AGENT_BUS_SEED",
            "GH_TOKEN",
            "GITHUB_TOKEN",
            "GH_ENTERPRISE_TOKEN",
        ] {
            assert!(removed.iter().any(|name| name == variable), "{variable}");
        }
        let args = args(&command);
        assert!(args.contains(&format!("--disallowedTools={DISALLOWED_TOOLS}")));
        assert!(args.iter().any(|arg| arg.contains("no GitHub access")));
    }

    #[test]
    fn a_task_with_github_keeps_the_tokens_and_a_narrower_deny_list() {
        let command = claude_command(CLAUDE, Path::new("/work"), MODEL, true, &Launch::Fresh);
        let removed = removed(&command);
        for variable in GITHUB_TOKEN_VARIABLES {
            assert!(!removed.iter().any(|name| name == variable), "{variable}");
        }
        for variable in ["ANTHROPIC_API_KEY", "AGENT_BUS_SEED"] {
            assert!(removed.iter().any(|name| name == variable), "{variable}");
        }
        let args = args(&command);
        assert!(args.contains(&format!("--disallowedTools={GITHUB_DISALLOWED_TOOLS}")));
        let rules = args
            .iter()
            .find(|arg| arg.contains("Co-Authored-By"))
            .unwrap();
        assert!(rules.contains("from your own attribution instructions"));
        assert!(rules.contains("never one copied from the task text"));
        for refused in [
            "gh pr merge",
            "gh api",
            "gh repo delete",
            "gh release",
            "git reset",
            "--force",
            "-f)",
            "--delete",
            "git push* :*",
            "git branch* -D",
        ] {
            assert!(GITHUB_DISALLOWED_TOOLS.contains(refused), "{refused}");
        }
        for allowed in [
            "Bash(git commit",
            "Bash(git push:*)",
            "Bash(gh:*)",
            "gh pr create",
        ] {
            assert!(!GITHUB_DISALLOWED_TOOLS.contains(allowed), "{allowed}");
        }
    }

    fn launched(launch: &Launch) -> Vec<String> {
        args(&claude_command(
            CLAUDE,
            Path::new("/work"),
            MODEL,
            false,
            launch,
        ))
    }

    #[test]
    fn the_model_reaches_every_command_line() {
        for launch in [
            Launch::Fresh,
            Launch::Fork("base-1".to_owned()),
            Launch::Resume("task-1".to_owned()),
        ] {
            for github in [false, true] {
                let args = args(&claude_command(
                    CLAUDE,
                    Path::new("/work"),
                    MODEL,
                    github,
                    &launch,
                ));
                let at = args.iter().position(|arg| arg == "--model").unwrap();
                assert_eq!(args[at + 1], MODEL);
            }
        }
    }

    #[test]
    fn the_default_model_is_opus_5_5() {
        assert_eq!(DEFAULT_MODEL, "claude-opus-5-5");
    }

    #[test]
    fn a_fresh_run_and_a_base_have_no_session_arguments() {
        let args = launched(&Launch::Fresh);
        assert!(!args.iter().any(|arg| arg == "--resume"));
        assert!(!args.iter().any(|arg| arg == "--fork-session"));
    }

    #[test]
    fn a_new_task_forks_the_base_session() {
        let args = launched(&Launch::Fork("base-1".to_owned()));
        let at = args.iter().position(|arg| arg == "--resume").unwrap();
        assert_eq!(args[at + 1], "base-1");
        assert_eq!(args[at + 2], "--fork-session");
    }

    #[test]
    fn an_answer_resumes_its_own_session_without_forking() {
        let args = launched(&Launch::Resume("task-1".to_owned()));
        let at = args.iter().position(|arg| arg == "--resume").unwrap();
        assert_eq!(args[at + 1], "task-1");
        assert!(!args.iter().any(|arg| arg == "--fork-session"));
    }

    /// Refused in both deny lists: they throw away uncommitted work.
    const DISCARDING: [&str; 10] = [
        "Bash(git stash:*)",
        "Bash(git restore:*)",
        "Bash(git clean:*)",
        "Bash(git checkout*-- *)",
        "Bash(git checkout* --)",
        "Bash(git checkout* .)",
        "Bash(git checkout* ./*)",
        "Bash(git checkout*--force*)",
        "Bash(git checkout* -f)",
        "Bash(git checkout* -f *)",
    ];

    fn patterns(list: &str) -> Vec<&str> {
        list.split(',').collect()
    }

    #[test]
    fn both_deny_lists_refuse_commands_that_discard_uncommitted_work() {
        for list in [DISALLOWED_TOOLS, GITHUB_DISALLOWED_TOOLS] {
            let patterns = patterns(list);
            for refused in DISCARDING {
                assert!(patterns.contains(&refused), "{refused}");
            }
        }
    }

    #[test]
    fn both_deny_lists_leave_checkout_of_a_branch_alone() {
        for list in [DISALLOWED_TOOLS, GITHUB_DISALLOWED_TOOLS] {
            for pattern in patterns(list) {
                assert_ne!(pattern, "Bash(git checkout:*)");
                assert_ne!(pattern, "Bash(git checkout*)");
                assert_ne!(pattern, "Bash(git checkout *)");
            }
        }
    }

    #[test]
    fn the_rules_forbid_stash_and_point_at_show_and_diff() {
        for github in [false, true] {
            let rules = rules(github);
            assert!(rules.contains("Never use git stash"));
            assert!(rules.contains("git show HEAD:<path>"));
            assert!(rules.contains("git diff"));
        }
    }

    #[test]
    fn the_rules_tell_the_agent_its_git_status_may_be_old() {
        for github in [false, true] {
            assert!(rules(github).contains("run git status yourself"));
        }
    }
}
