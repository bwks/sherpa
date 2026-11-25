use std::path::PathBuf;
use std::process::ExitStatus;
#[cfg(feature = "record")]
use std::time::Duration;

use shared::konst::{LAB_FILE_NAME, LAB_STATE_DIR};
use thiserror::Error;

use crate::REPO_ENV;
#[cfg(feature = "record")]
use crate::generation::section::{END, START};
use crate::gh::GH_TOKEN_VARIABLE;
use crate::keys::SEED_VARIABLE;
use crate::state::SERVER_ENV;

/// An agent-bus failure that a caller or a test tells apart from the others.
#[derive(Debug, Error)]
pub enum AgentBusError {
    #[error(
        "Set {} to this agent's NKey seed (see `agent-bus enroll` for a guest, `agent-bus key new` for the host)",
        SEED_VARIABLE
    )]
    SeedUnset,

    #[error("AGENT_BUS_SEED is not a valid NKey seed")]
    SeedInvalid,

    #[error("AGENT_BUS_SEED is not a user seed (it must start with SU)")]
    SeedNotUser,

    #[error("The value is not a valid NKey public key")]
    PublicKeyInvalid,

    #[error("The bus seed is empty")]
    BusSeedEmpty,

    #[error("The bus seed is not a valid NKey seed")]
    BusSeedInvalid,

    #[error("The bus seed is not a user seed (it must start with SU)")]
    BusSeedNotUser,

    #[error("Unable to parse {what} at line {line}: {message}")]
    Unparsable {
        what: String,
        line: usize,
        message: String,
    },

    #[error("The key of agent '{agent}' is not valid")]
    AgentKeyInvalid { agent: String },

    #[error("Agent '{agent}' uses the key of another agent; every agent needs its own key")]
    KeyShared { agent: String },

    #[error("The agents file and the registry of enrolled guests clash")]
    RegistryClash,

    #[error("The Claude subscription token is empty")]
    ClaudeTokenEmpty,

    #[error(
        "The Claude subscription token line holds another secret: start the agent with a start-guest.sh of this version"
    )]
    ClaudeTokenHoldsAnotherSecret,

    #[error("The Claude subscription token has spaces or control characters")]
    ClaudeTokenNotPrintable,

    #[error("The GitHub write token is not a GitHub token (github_pat_..., ghp_...)")]
    WriteTokenInvalid,

    #[error(
        "{} in the guest agent's environment is the GitHub write token, so every task would have it: give the guest the read-only token in ~/.env.d/github.env",
        GH_TOKEN_VARIABLE
    )]
    WriteTokenInEnvironment,

    #[error("The first line on stdin must be the Claude subscription token")]
    ClaudeTokenLine,

    #[error("The second line on stdin must be the GitHub write token, or empty for none")]
    WriteTokenLine,

    #[error("The third line on stdin must be the agent's bus seed")]
    SeedLine,

    #[error(
        "{} must not be in the guest agent's environment, where every task could read it: the agent reads its secrets from stdin. Start it with dev/agent-bus/start-guest.sh",
        variables.join(" and ")
    )]
    SecretsInEnvironment { variables: Vec<&'static str> },

    #[error(
        "Another guest agent of this name is running ({} holds {}): stop it first",
        holder.map_or_else(|| "another process".to_owned(), |pid| format!("PID {pid}")),
        path.display()
    )]
    GuestRunning { holder: Option<u32>, path: PathBuf },

    #[cfg(feature = "record")]
    #[error(
        "The record is busy: {} has held {} for more than {}s. A command holds it only while it \
         catches up and reads; if that process is stuck, stop it",
        holder.map_or_else(|| "another process".to_owned(), |pid| format!("process {pid}")),
        path.display(),
        wait.as_secs_f32()
    )]
    RecordBusy {
        holder: Option<u32>,
        path: PathBuf,
        wait: Duration,
    },

    #[error(
        "{} has a {} but no {}/ folder: it is in the old lab layout. Run any sherpa 0.5.15 \
         command in that folder (such as `sherpa inspect`) to move its files into {}/, then try \
         again",
        dir.display(),
        LAB_FILE_NAME,
        LAB_STATE_DIR,
        LAB_STATE_DIR
    )]
    OldLabLayout { dir: PathBuf },

    #[error("Unable to read {}: is it a lab directory?", path.display())]
    LabInfoUnreadable { path: PathBuf },

    #[error("{} is not a lab info file with an ipv4_gateway", path.display())]
    NotLabInfo { path: PathBuf },

    #[error(
        "No bus to connect to: run `agent-bus bus start`, or set --server or {} (looked for {})",
        SERVER_ENV,
        record.display()
    )]
    NoServer { record: PathBuf },

    #[error("No bus recorded in {}: run `agent-bus bus start` first", path.display())]
    NoBusRecorded { path: PathBuf },

    #[error(
        "Unable to enroll '{name}'. The previous registry and config are back and the bus has \
         reloaded them, so its previous key, if any, still works. Fix the cause and enroll it \
         again"
    )]
    EnrollmentRolledBack {
        name: String,
        #[source]
        enrollment: anyhow::Error,
    },

    #[error(
        "Unable to enroll '{name}', and putting the previous registry and config back failed \
         too ({restore:#}). Fix the cause, then enroll it again, which replaces the key, or run \
         `agent-bus bus start` to render and load the config from the registry"
    )]
    EnrollmentNotRestored {
        name: String,
        #[source]
        enrollment: anyhow::Error,
        restore: anyhow::Error,
    },

    #[error(
        "{} exists but {} does not. Remove {} too to make a new bus CA, then start every guest \
         agent again so it gets the new CA certificate",
        there.display(),
        missing.display(),
        there.display()
    )]
    HalfCa { there: PathBuf, missing: PathBuf },

    #[error("Unable to start {what}")]
    CommandNotStarted { what: String },

    #[error("Unable to write to {what}")]
    InputNotWritten { what: String },

    #[error("Unable to write to {what}, which ended with {status}")]
    InputNotWrittenAfterExit { what: String, status: ExitStatus },

    #[error("{what} failed: {stderr}")]
    CommandFailed { what: String, stderr: String },

    #[error(
        "Unable to keep the message in the journal {}: not printing or acknowledging it",
        path.display()
    )]
    NotJournaled { path: PathBuf },

    #[error("Unable to print the message")]
    NotPrinted,

    #[cfg(feature = "record")]
    #[error(
        "The Generation data in the pull request body is format {version}, newer than {newest} \
         that this agent-bus writes: use a newer agent-bus"
    )]
    GenerationDataTooNew { version: u32, newest: u32 },

    #[cfg(feature = "record")]
    #[error(
        "The pull request body has its Generation markers out of order or more than once: \
         {} must be followed by {}, once each, each on a line of its own. Fix the body by hand",
        START,
        END
    )]
    GenerationMarkers,

    #[cfg(feature = "record")]
    #[error(
        "No work item has pull request {repo}#{number}: link a task to it with `link <task> \
         --pr {number}`"
    )]
    NoWorkItem { repo: String, number: u64 },

    #[cfg(feature = "record")]
    #[error(
        "The record has schema version {version}, newer than {current} that this agent-bus \
         knows: use a newer agent-bus"
    )]
    SchemaTooNew { version: usize, current: usize },

    #[cfg(feature = "record")]
    #[error("The record has no task '{task}'")]
    NoTask { task: String },

    #[cfg(feature = "record")]
    #[error("Unable to record a model of the run")]
    RunModelUnrecorded,

    #[error(
        "Give the repository of --issue and --pr with --repo or {}, such as bwks/sherpa",
        REPO_ENV
    )]
    RepoUnset,
}
