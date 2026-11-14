//! The messages agents exchange: a task, its progress, questions and answers, and its outcome.

use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// The longest agent name, in characters.
const MAX_AGENT_NAME: usize = 32;
/// The longest owner or name of a repository, in characters.
const MAX_REPO_PART: usize = 100;

/// What a message is for. The outcomes and `Question` follow A2A's task states.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    /// Work for the receiving agent.
    Task,
    /// The answer to a question the receiving agent asked.
    Answer,
    /// What the agent working on a task is doing.
    Progress,
    /// The task needs a decision before it can go on.
    Question,
    /// The task is done.
    Completed,
    /// The task could not be done.
    Failed,
    /// Host to guest: stop the task named in `task`, running or queued.
    Cancel,
    /// Guest to host: the task was canceled, after A2A's canceled state.
    Canceled,
}

/// One message on the bus.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Message {
    /// The task this message belongs to.
    pub task: String,
    pub from: String,
    pub to: String,
    pub kind: Kind,
    pub text: String,
    /// What the run cost. Only the outcome messages (completed, question, failed, and canceled
    /// when a result arrived) carry it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<Usage>,
    /// What the task has cost so far, this run included. Sent with `usage`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_usage: Option<TaskUsage>,
    /// Whether the task may use GitHub. Only a task sets it; an answer has the flag of its task.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub github: bool,
    /// The model the task runs on: an alias or a full ID, set by a task. An answer runs on the
    /// model of its task, whatever it says. The guest's replies carry the model they ran on.
    /// Without it the guest uses its own default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// What the task is for: the issues and pull requests `send --issue/--pr` tagged it with.
    /// Only a task carries it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub work: Option<Work>,
    /// Where the guest's checkout was when the run ended. Every outcome of a task that ran
    /// carries it, from a guest that reports it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ended: Option<Ended>,
}

/// The work a task is tagged with, and the host session that sent it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Work {
    #[serde(default)]
    pub refs: Vec<WorkRef>,
    /// The host's Claude Code session that sent the task (`CLAUDE_CODE_SESSION_ID`), when it
    /// ran inside one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<String>,
}

/// An issue or a pull request a task is tagged with.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkRef {
    /// The repository, `owner/name`.
    pub repo: String,
    pub kind: RefKind,
    pub number: u64,
}

/// What a reference names.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RefKind {
    Issue,
    PullRequest,
    Branch,
}

/// Where a guest's checkout was when a run ended.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ended {
    /// The checkout's repository, `owner/name`.
    pub repo: String,
    /// The branch checked out; absent on the repository's default branch or a detached HEAD.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    /// The open pull requests whose head is `branch`, in the checkout's repository and in the
    /// docs checkout's beside it.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pull_requests: Vec<PullRequestRef>,
}

/// An open pull request a guest found for its branch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PullRequestRef {
    /// Its repository, `owner/name`.
    pub repo: String,
    pub number: u64,
    pub url: String,
}

/// What one Claude Code run used, from the `result` event of its `stream-json` output.
///
/// The token counts are the run's own, across all its turns. The cost is the sum of the
/// per-model `costUSD` in `model_usage_since_last_result`, which is per run; `total_cost_usd`
/// and `modelUsage` are totals for the whole session, across resumes, so they are not used for a
/// run. The cache hit ratio is `cache_read / (input + cache_creation + cache_read)`: see
/// [`cache_hit_permille`].
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Usage {
    /// Input tokens that were neither read from nor written to the prompt cache.
    pub input: u64,
    pub output: u64,
    /// Input tokens written to the prompt cache: a cache miss that pays to fill the cache.
    pub cache_creation: u64,
    /// Input tokens read from the prompt cache: a cache hit.
    pub cache_read: u64,
    pub num_turns: u64,
    /// Wall-clock time of the run, in milliseconds.
    pub duration_ms: u64,
    /// Time this run spent in API calls, in milliseconds: the `duration_api_ms` of the event
    /// minus that of the session's previous run.
    pub duration_api_ms: u64,
    /// The API-equivalent cost of this run, in US dollars: the sum of its models' `costUSD`. It
    /// is NOT billed: guest agents run on the Claude subscription. Read it as a measure of
    /// efficiency, not as money spent.
    pub cost_usd: f64,
    /// The session total so far (`total_cost_usd` of the event), across its runs. Not this run's
    /// cost.
    pub session_cost_usd: f64,
    /// This run's figures per model, from `model_usage_since_last_result`. Empty when Claude
    /// Code gave none.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub models: Vec<ModelUsage>,
    /// The run's work time, in milliseconds: the model writing and the tool calls that are not
    /// testing, without the waits that are not counted (`timing`). The guest stamps the stream
    /// events as it reads them. Absent from an older guest; an older host ignores it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub work_ms: Option<u64>,
    /// The run's testing time, in milliseconds: tool calls that build or test.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub testing_ms: Option<u64>,
}

/// What one model used in a run, from an entry of `model_usage_since_last_result`. The field
/// names on the wire are the ones Claude Code writes.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ModelUsage {
    /// The model's key in the event (not a field of the entry itself).
    #[serde(default)]
    pub model: String,
    #[serde(default, rename = "inputTokens")]
    pub input: u64,
    #[serde(default, rename = "outputTokens")]
    pub output: u64,
    #[serde(default, rename = "cacheCreationInputTokens")]
    pub cache_creation: u64,
    #[serde(default, rename = "cacheReadInputTokens")]
    pub cache_read: u64,
    /// The API-equivalent cost for this model in this run, in US dollars. Not billed.
    #[serde(default, rename = "costUSD")]
    pub cost_usd: f64,
    #[serde(default, rename = "canonicalModel")]
    pub canonical_model: String,
    /// How the cost was worked out, for example `list` (list prices).
    #[serde(default, rename = "costBasis")]
    pub cost_basis: String,
}

/// What a task has used so far, over its runs: the first run and the answers.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct TaskUsage {
    /// Runs counted: a run that sent no result event is not counted.
    pub runs: u64,
    pub cost_usd: f64,
    pub input: u64,
    pub output: u64,
    pub cache_creation: u64,
    pub cache_read: u64,
    pub num_turns: u64,
    /// Wall-clock time, in milliseconds.
    pub duration_ms: u64,
    /// Time in API calls, in milliseconds.
    pub duration_api_ms: u64,
}

impl TaskUsage {
    /// Count one more run. The sums saturate rather than overflow.
    pub fn add(&mut self, run: &Usage) {
        self.runs = self.runs.saturating_add(1);
        self.cost_usd += run.cost_usd;
        self.input = self.input.saturating_add(run.input);
        self.output = self.output.saturating_add(run.output);
        self.cache_creation = self.cache_creation.saturating_add(run.cache_creation);
        self.cache_read = self.cache_read.saturating_add(run.cache_read);
        self.num_turns = self.num_turns.saturating_add(run.num_turns);
        self.duration_ms = self.duration_ms.saturating_add(run.duration_ms);
        self.duration_api_ms = self.duration_api_ms.saturating_add(run.duration_api_ms);
    }
}

/// A run's API time: the event's `duration_api_ms`, which counts the whole session, minus the
/// previous run's of the same session. The first run of a session (`previous` is `None`) takes
/// it as it is, and so does a figure that went down, which means the count started again.
pub fn api_time_of_run(reported: u64, previous: Option<u64>) -> u64 {
    previous.map_or(reported, |before| {
        reported.checked_sub(before).unwrap_or(reported)
    })
}

/// The share of input tokens served from the prompt cache, in tenths of a percent, rounded down
/// (0 to 1000), or `None` when there were no input tokens or the arithmetic overflows.
pub fn cache_hit_permille(input: u64, cache_creation: u64, cache_read: u64) -> Option<u64> {
    let total = input.checked_add(cache_creation)?.checked_add(cache_read)?;
    cache_read.checked_mul(1000)?.checked_div(total)
}

/// A new task ID: eight hexadecimal digits.
pub fn new_task_id() -> String {
    Uuid::new_v4().simple().to_string()[..8].to_owned()
}

/// Check an agent name: it names the agent's subject on the bus, so it is 1 to 32 lowercase
/// letters, digits and hyphens.
///
/// # Errors
///
/// Fails when the name is empty, too long or holds another character.
pub fn check_agent_name(name: &str) -> Result<()> {
    ensure!(
        !name.is_empty()
            && name.len() <= MAX_AGENT_NAME
            && name.chars().all(|character| character.is_ascii_lowercase()
                || character.is_ascii_digit()
                || character == '-'),
        "Agent name '{name}' must be 1 to {MAX_AGENT_NAME} lowercase letters, digits and hyphens"
    );
    Ok(())
}

/// Check a repository name: `owner/name`, each part 1 to 100 ASCII letters, digits, `-`, `_` or
/// `.`.
///
/// # Errors
///
/// Fails when the name is not of that form.
pub fn check_repo(repo: &str) -> Result<()> {
    let part = |text: &str| {
        !text.is_empty()
            && text.len() <= MAX_REPO_PART
            && text.chars().all(|character| {
                character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.')
            })
    };
    ensure!(
        repo.split_once('/')
            .is_some_and(|(owner, name)| part(owner) && part(name)),
        "Repository '{repo}' must be owner/name, such as bwks/sherpa"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A run's usage as an agent-bus from before the time buckets reads it, in part.
    #[derive(Deserialize)]
    struct OldUsage {
        input: u64,
    }

    /// A message as an agent-bus from before `work` and `ended` reads it.
    #[derive(Deserialize)]
    struct OldMessage {
        task: String,
        kind: Kind,
        text: String,
        #[serde(default)]
        github: bool,
    }

    #[test]
    fn messages_use_snake_case_kinds_on_the_wire() {
        let message = Message {
            task: "1a2b3c4d".to_owned(),
            from: "host".to_owned(),
            to: "dev".to_owned(),
            kind: Kind::Question,
            text: "Add the dependency?".to_owned(),
            usage: None,
            task_usage: None,
            github: false,
            model: None,
            work: None,
            ended: None,
        };
        let json = serde_json::to_value(&message).unwrap();
        assert_eq!(json["kind"], "question");
        assert!(json.get("usage").is_none());
        assert!(json.get("github").is_none());
        assert!(json.get("model").is_none());
        assert_eq!(serde_json::from_value::<Message>(json).unwrap(), message);
    }

    #[test]
    fn github_round_trips_and_defaults_to_false() {
        let message = Message {
            task: "1a2b3c4d".to_owned(),
            from: "host".to_owned(),
            to: "dev".to_owned(),
            kind: Kind::Task,
            text: "Open a PR".to_owned(),
            usage: None,
            task_usage: None,
            github: true,
            model: None,
            work: None,
            ended: None,
        };
        let json = serde_json::to_value(&message).unwrap();
        assert_eq!(json["github"], true);
        assert_eq!(serde_json::from_value::<Message>(json).unwrap(), message);
        let old = r#"{"task":"t","from":"a","to":"b","kind":"task","text":"x"}"#;
        assert!(!serde_json::from_str::<Message>(old).unwrap().github);
    }

    #[test]
    fn the_model_round_trips_and_is_absent_when_unset() {
        let message = Message {
            task: "1a2b3c4d".to_owned(),
            from: "host".to_owned(),
            to: "dev".to_owned(),
            kind: Kind::Task,
            text: "Run the tests".to_owned(),
            usage: None,
            task_usage: None,
            github: false,
            model: Some("claude-sonnet-5-5".to_owned()),
            work: None,
            ended: None,
        };
        let json = serde_json::to_value(&message).unwrap();
        assert_eq!(json["model"], "claude-sonnet-5-5");
        assert_eq!(serde_json::from_value::<Message>(json).unwrap(), message);

        let unset = Message {
            model: None,
            ..message
        };
        let json = serde_json::to_value(&unset).unwrap();
        assert!(json.get("model").is_none());
        assert_eq!(serde_json::from_value::<Message>(json).unwrap(), unset);
        let old = r#"{"task":"t","from":"a","to":"b","kind":"task","text":"x"}"#;
        assert_eq!(serde_json::from_str::<Message>(old).unwrap().model, None);
    }

    #[test]
    fn usage_round_trips_on_the_wire_under_claude_codes_model_field_names() {
        let model = ModelUsage {
            model: "claude-sonnet-5-5".to_owned(),
            input: 10,
            output: 20,
            cache_creation: 30,
            cache_read: 60,
            cost_usd: 0.25,
            canonical_model: "claude-sonnet-5-5".to_owned(),
            cost_basis: "list".to_owned(),
        };
        let usage = Usage {
            input: 10,
            output: 20,
            cache_creation: 30,
            cache_read: 60,
            num_turns: 2,
            duration_ms: 1500,
            duration_api_ms: 1200,
            cost_usd: 0.25,
            session_cost_usd: 0.5,
            models: vec![model],
            work_ms: Some(60_000),
            testing_ms: Some(30_000),
        };
        let message = Message {
            task: "1a2b3c4d".to_owned(),
            from: "dev".to_owned(),
            to: "host".to_owned(),
            kind: Kind::Completed,
            text: "Done".to_owned(),
            usage: Some(usage.clone()),
            task_usage: Some(TaskUsage {
                runs: 1,
                ..TaskUsage::default()
            }),
            github: false,
            model: None,
            work: None,
            ended: None,
        };
        let json = serde_json::to_value(&message).unwrap();
        assert_eq!(json["usage"]["cache_read"], 60);
        assert_eq!(json["usage"]["session_cost_usd"], 0.5);
        assert_eq!(json["usage"]["models"][0]["cacheReadInputTokens"], 60);
        assert_eq!(json["usage"]["models"][0]["costUSD"], 0.25);
        assert_eq!(json["usage"]["models"][0]["costBasis"], "list");
        assert_eq!(json["task_usage"]["runs"], 1);
        assert_eq!(json["usage"]["work_ms"], 60_000);
        assert_eq!(json["usage"]["testing_ms"], 30_000);
        assert_eq!(serde_json::from_value::<Message>(json).unwrap(), message);
    }

    #[test]
    fn a_runs_time_is_optional_both_ways() {
        // From an older guest: no time.
        let old = r#"{"input":1,"output":2,"cache_creation":0,"cache_read":0,"num_turns":1,
            "duration_ms":5,"duration_api_ms":4,"cost_usd":0.1,"session_cost_usd":0.1}"#;
        let usage: Usage = serde_json::from_str(old).unwrap();
        assert_eq!((usage.work_ms, usage.testing_ms), (None, None));
        let json = serde_json::to_value(&usage).unwrap();
        assert!(json.get("work_ms").is_none());
        // To an older host: a struct without the fields reads it and ignores them.
        let new = Usage {
            input: 3,
            work_ms: Some(1),
            testing_ms: Some(2),
            ..Usage::default()
        };
        let read: OldUsage = serde_json::from_value(serde_json::to_value(&new).unwrap()).unwrap();
        assert_eq!(read.input, 3);
    }

    fn tagged_task() -> Message {
        Message {
            task: "1a2b3c4d".to_owned(),
            from: "host".to_owned(),
            to: "dev".to_owned(),
            kind: Kind::Task,
            text: "Fix it".to_owned(),
            usage: None,
            task_usage: None,
            github: false,
            model: None,
            work: Some(Work {
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
                session: Some("sess-host".to_owned()),
            }),
            ended: None,
        }
    }

    #[test]
    fn work_and_ended_round_trip_and_are_absent_when_unset() {
        let task = tagged_task();
        let json = serde_json::to_value(&task).unwrap();
        assert_eq!(json["work"]["refs"][0]["kind"], "issue");
        assert_eq!(json["work"]["refs"][1]["kind"], "pull_request");
        assert_eq!(json["work"]["refs"][1]["number"], 612);
        assert_eq!(json["work"]["session"], "sess-host");
        assert!(json.get("ended").is_none());
        assert_eq!(serde_json::from_value::<Message>(json).unwrap(), task);

        let outcome = Message {
            kind: Kind::Completed,
            work: None,
            ended: Some(Ended {
                repo: "o/r".to_owned(),
                branch: Some("feat/x".to_owned()),
                pull_requests: vec![PullRequestRef {
                    repo: "o/r-docs".to_owned(),
                    number: 7,
                    url: "https://github.com/o/r-docs/pull/7".to_owned(),
                }],
            }),
            ..task
        };
        let json = serde_json::to_value(&outcome).unwrap();
        assert!(json.get("work").is_none());
        assert_eq!(json["ended"]["branch"], "feat/x");
        assert_eq!(json["ended"]["pull_requests"][0]["number"], 7);
        assert_eq!(serde_json::from_value::<Message>(json).unwrap(), outcome);

        // On the default branch: no branch and no pull requests on the wire.
        let on_default = Ended {
            repo: "o/r".to_owned(),
            branch: None,
            pull_requests: Vec::new(),
        };
        assert_eq!(
            serde_json::to_value(&on_default).unwrap(),
            serde_json::json!({"repo": "o/r"})
        );
    }

    #[test]
    fn an_old_message_parses_without_work_or_ended() {
        let old = r#"{"task":"t","from":"a","to":"b","kind":"completed","text":"x"}"#;
        let message = serde_json::from_str::<Message>(old).unwrap();
        assert_eq!((message.work, message.ended), (None, None));
    }

    #[test]
    fn a_message_with_fields_this_agent_bus_does_not_know_parses() {
        let newer = r#"{"task":"t","from":"a","to":"b","kind":"task","text":"x",
            "work":{"refs":[{"repo":"o/r","kind":"issue","number":1,"title":"new"}],"focus":true},
            "ended":{"repo":"o/r","head":"abc"},"priority":3}"#;
        let message = serde_json::from_str::<Message>(newer).unwrap();
        assert_eq!(message.work.unwrap().refs[0].number, 1);
        assert_eq!(message.ended.unwrap().branch, None);
    }

    /// What an agent-bus from before `work` and `ended` reads of a message that has them: the
    /// fields it knows, the same as before.
    #[test]
    fn an_older_peer_reads_a_new_message_as_before() {
        let json = serde_json::to_string(&tagged_task()).unwrap();
        let old: OldMessage = serde_json::from_str(&json).unwrap();
        assert_eq!((old.task.as_str(), old.kind), ("1a2b3c4d", Kind::Task));
        assert_eq!(old.text, "Fix it");
        assert!(!old.github);
    }

    #[test]
    fn a_repository_is_owner_slash_name() {
        for repo in ["bwks/sherpa", "o-1/r.name_2"] {
            check_repo(repo).unwrap();
        }
        for repo in ["", "sherpa", "a/b/c", "/r", "o/", "o r/x", "o/r;x"] {
            assert!(check_repo(repo).is_err(), "{repo}");
        }
    }

    #[test]
    fn cache_hit_permille_is_cache_read_over_all_input() {
        assert_eq!(cache_hit_permille(10, 30, 60), Some(600));
        assert_eq!(cache_hit_permille(0, 0, 0), None);
        assert_eq!(cache_hit_permille(1, 0, u64::MAX), None);
    }

    #[test]
    fn a_runs_api_time_is_the_increase_over_the_sessions_previous_run() {
        assert_eq!(api_time_of_run(1556, None), 1556);
        assert_eq!(api_time_of_run(2937, Some(1556)), 1381);
        assert_eq!(api_time_of_run(4218, Some(2937)), 1281);
        // A figure that went down means the count started again.
        assert_eq!(api_time_of_run(500, Some(2937)), 500);
    }

    #[test]
    fn a_task_adds_up_its_runs() {
        let run = |cost_usd, input, api| Usage {
            input,
            output: 1,
            cache_creation: 2,
            cache_read: 3,
            num_turns: 1,
            duration_ms: 10,
            duration_api_ms: api,
            cost_usd,
            ..Usage::default()
        };
        let mut task = TaskUsage::default();
        task.add(&run(0.5, 4, 7));
        task.add(&run(0.25, u64::MAX, 8));
        assert_eq!(task.runs, 2);
        assert!((task.cost_usd - 0.75).abs() < 1e-9);
        assert_eq!(task.input, u64::MAX);
        assert_eq!(task.output, 2);
        assert_eq!(task.num_turns, 2);
        assert_eq!(task.duration_ms, 20);
        assert_eq!(task.duration_api_ms, 15);
    }

    #[test]
    fn task_ids_are_eight_hex_digits() {
        let id = new_task_id();
        assert_eq!(id.len(), 8);
        assert!(id.chars().all(|character| character.is_ascii_hexdigit()));
        assert_ne!(new_task_id(), id);
    }

    #[test]
    fn agent_names_must_be_valid_subject_tokens() {
        for name in ["dev", "host", "guest-2"] {
            check_agent_name(name).unwrap();
        }
        for name in ["", "Dev", "a.b", "a*", "a b", &"a".repeat(33)] {
            assert!(check_agent_name(name).is_err(), "{name}");
        }
    }
}
