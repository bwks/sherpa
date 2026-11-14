//! `agent-bus`: a development tool that gives the host agent and agents in lab guests a message
//! channel, over NATS with `JetStream`.
//!
//! The host runs the bus in a container (`bus start`). The host agent sends tasks (`send`) and
//! follows the replies (`listen`, one JSON line per message, kept in a journal first). A guest agent (`guest`) takes each
//! task to Claude Code and sends back its progress, questions and result.
//!
//! Every agent authenticates with its own `NKey`. The host's public key goes in the agents file
//! (`key new` makes the pair); guests are enrolled with `enroll`, which makes a fresh pair, keeps
//! the public key in the host's registry, reloads the bus and prints the seed for a pipe. The
//! server's permissions are rendered from both. The host's commands read its seed from the
//! environment variable `AGENT_BUS_SEED`; a guest agent reads its seed, and its Claude and GitHub
//! tokens, from stdin and holds them in memory only, never in its environment. A seed is never a
//! command-line value.
#![deny(clippy::unwrap_used, clippy::expect_used)]

mod agents;
mod bus;
mod channel;
mod config;
mod files;
mod guest;
mod keys;
mod listen;
mod message;
mod model;
mod pull;
#[cfg(feature = "record")]
mod record;
mod registry;
#[cfg(feature = "record")]
mod report;
mod state;
mod status;
#[cfg(feature = "record")]
mod store;
mod table;
mod tls;

use std::env;
use std::io::{self, IsTerminal};
use std::net::Ipv4Addr;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Result, bail};
use async_nats::rustls::crypto::aws_lc_rs;
use clap::{ArgGroup, Parser, Subcommand};
use tracing_subscriber::EnvFilter;
use tracing_subscriber::fmt::format::FmtSpan;

use agents::Role;
use bus::Bus;
use channel::{Channel, Endpoint, RECORDER};
use files::now_ms;
use keys::seed_from_environment;
use message::{Kind, Message, RefKind, Work, WorkRef, check_agent_name, check_repo, new_task_id};
use model::{DEFAULT_MODEL, model_id};
use status::{Observed, STALE};

/// The variable holding the host's repository, `owner/name`, that `send --issue/--pr` tags.
const REPO_ENV: &str = "AGENT_BUS_REPO";
/// The variable Claude Code sets in the commands it runs: its session's ID.
const SESSION_ENV: &str = "CLAUDE_CODE_SESSION_ID";

#[derive(Parser)]
#[command(
    version,
    about = "A NATS message channel between the host agent and guest agents",
    after_help = "The host's commands read its NKey seed from the environment variable \
                  AGENT_BUS_SEED; `agent-bus guest` reads its own from stdin. \
                  `agent-bus key new` makes the host's key \
                  pair; `agent-bus enroll` makes a guest's. Every connection is TLS, trusting \
                  only the bus's own CA: the certificate in AGENT_BUS_CA, else the one \
                  `bus start` made in the host's state directory."
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Make a user key pair: print its public key and its seed
    Key {
        #[command(subcommand)]
        action: KeyAction,
    },
    /// Run the bus on the host
    Bus {
        #[command(subcommand)]
        action: BusAction,
    },
    /// Give a guest agent a fresh key on the running bus, replacing its previous key, and print
    /// only the new seed on stdout, for a pipe; stdout must not be a terminal. Run as the host:
    /// `AGENT_BUS_SEED` is the host's seed
    Enroll {
        /// The guest agent; the agents file must not list it
        name: String,
    },
    /// Remove an enrolled guest agent's key; the bus refuses it from then on. With the host's
    /// seed in `AGENT_BUS_SEED` and the bus running, its inbox is deleted too; its messages stay
    /// until the stream drops them
    Unenroll {
        /// The guest agent
        name: String,
    },
    /// Send a task, or an answer to a guest agent's question
    Send {
        /// The bus, for example `nats://192.0.2.1:4222`. Without it: `AGENT_BUS_URL`, then
        /// where `bus start` recorded the bus
        #[arg(long)]
        server: Option<String>,
        /// The sending agent
        #[arg(long, default_value = "host")]
        from: String,
        /// The receiving agent
        #[arg(long)]
        to: String,
        /// The task this answers; a new task gets a new ID
        #[arg(long)]
        answer: Option<String>,
        /// Let the task use GitHub (commit, push, pull requests). An answer has the flag of its
        /// task.
        #[arg(long, conflicts_with = "answer")]
        github: bool,
        /// The model the task runs on: opus, sonnet, haiku or a full ID such as
        /// claude-sonnet-5-5. Without it, the guest's own. An answer runs on its task's model.
        #[arg(long, conflicts_with = "answer", value_parser = model_id)]
        model: Option<String>,
        /// Tag the task with this issue number: the task is part of the work item it names.
        /// Repeat for more
        #[arg(long, conflicts_with = "answer")]
        issue: Vec<u64>,
        /// Tag the task with this pull request number. Repeat for more
        #[arg(long, conflicts_with = "answer")]
        pr: Vec<u64>,
        /// The repository of --issue and --pr, `owner/name`. Without it: `AGENT_BUS_REPO`
        #[arg(long, conflicts_with = "answer")]
        repo: Option<String>,
        /// The task, or the answer
        text: String,
    },
    /// Cancel a task on a guest agent, whether it is running or queued
    Cancel {
        /// The bus, for example `nats://192.0.2.1:4222`. Without it: `AGENT_BUS_URL`, then
        /// where `bus start` recorded the bus
        #[arg(long)]
        server: Option<String>,
        /// The sending agent
        #[arg(long, default_value = "host")]
        from: String,
        /// The agent running the task
        #[arg(long)]
        to: String,
        /// The task to cancel
        task: String,
    },
    /// Print each message sent to an agent, as one JSON line. Each is first kept in the journal
    /// `$XDG_STATE_HOME/agent-bus/listen-<agent>.jsonl`, then printed, then acknowledged
    Listen {
        /// The bus, for example `nats://192.0.2.1:4222`. Without it: `AGENT_BUS_URL`, then
        /// where `bus start` recorded the bus
        #[arg(long)]
        server: Option<String>,
        /// First print the journal's messages received this long ago or later, such as `30m`
        /// (units s, m, h, d), then listen
        #[arg(long, value_parser = listen::parse_since)]
        since: Option<Duration>,
        /// The agent whose messages to print
        agent: String,
    },
    #[cfg(feature = "record")]
    #[command(flatten)]
    Record(report::Command),
    /// Show every agent of the recorded bus, the host and each guest: whether it is online, the
    /// task it runs and for how long, how many tasks wait for it, and when it was last active.
    /// Run as the host: `AGENT_BUS_SEED` is the host's seed
    Agents {
        /// The bus, for example `nats://192.0.2.1:4222`. Without it: `AGENT_BUS_URL`, then
        /// where `bus start` recorded the bus
        #[arg(long)]
        server: Option<String>,
        /// Print the agents as JSON instead of a table
        #[arg(long)]
        json: bool,
    },
    /// Run a guest agent: take each task to Claude Code and reply with its progress and result.
    /// Its secrets come on stdin, which must not be a terminal, one per line: the Claude
    /// subscription token (from `claude setup-token`), the GitHub write token (an empty line for
    /// none) and the agent's bus seed. They are held in memory only. The Claude token is set in
    /// each Claude Code run's environment and the write token in a --github task's alone; without
    /// a write token, --github tasks fail. It refuses to start with `AGENT_BUS_SEED` or
    /// `CLAUDE_CODE_OAUTH_TOKEN` in its environment
    Guest {
        /// The bus, for example `nats://192.0.2.1:4222`
        #[arg(long, env = "AGENT_BUS_URL")]
        server: String,
        /// This agent's name
        #[arg(long)]
        name: String,
        /// The agent that receives the replies
        #[arg(long, default_value = "host")]
        reply_to: String,
        /// The directory Claude Code works in
        #[arg(long)]
        workdir: PathBuf,
        /// The model Claude Code runs on when a task names none: opus, sonnet, haiku or a full ID
        #[arg(long, default_value = DEFAULT_MODEL, value_parser = model_id)]
        model: String,
    },
}

#[derive(Subcommand)]
enum KeyAction {
    /// Generate a user key pair, for the host or a guest with a fixed key in the agents file.
    /// The seed is printed once, here: store it in your secret store, and give it to its agent
    /// through `AGENT_BUS_SEED`. Guests normally get theirs from `enroll` instead
    New,
}

#[derive(Subcommand)]
enum BusAction {
    /// Start the bus container, listening on one host address
    #[command(group(ArgGroup::new("address").required(true).args(["lab", "listen"])))]
    Start {
        /// The lab to serve: a lab directory holding `.sherpa/lab-info.toml`. The bus listens on the
        /// lab's management gateway, which is where its guests find it
        #[arg(long)]
        lab: Option<PathBuf>,
        /// The address to listen on instead of a lab's
        #[arg(long)]
        listen: Option<Ipv4Addr>,
        /// The agents file (TOML): the host's name and public key, and any guests with fixed
        /// keys. The server's permissions are rendered from it and the enrolled guests. Run as
        /// the host: `AGENT_BUS_SEED` is the host's seed
        #[arg(long)]
        agents: PathBuf,
    },
    /// Stop and remove the bus container; its stored messages are kept
    Stop,
    /// Show whether the bus container is running
    Status,
}

/// What a new task may do, runs on and is for. An answer carries none: it has its task's.
struct Options {
    github: bool,
    model: Option<String>,
    work: Option<Work>,
}

#[tokio::main]
async fn main() -> Result<()> {
    // The TLS connection to the bus takes rustls's process default, aws-lc-rs as in the rest
    // of the workspace. Already installed is fine.
    let _ = aws_lc_rs::default_provider().install_default();
    init_tracing();
    execute(Cli::parse()).await
}

/// Run the command `cli` names.
async fn execute(cli: Cli) -> Result<()> {
    match cli.command {
        Command::Key { action } => match action {
            KeyAction::New => new_key(),
        },
        Command::Bus { action } => bus_command(action).await,
        Command::Enroll { name } => enroll(&name).await,
        Command::Unenroll { name } => {
            // The seed is optional: with it, the inbox is deleted too. One that is set must be
            // valid.
            let seed = match env::var_os(keys::SEED_VARIABLE) {
                Some(_) => Some(seed_from_environment()?),
                None => None,
            };
            bus::unenroll_standard(&name, seed.as_deref()).await
        }
        Command::Send {
            server,
            from,
            to,
            answer,
            github,
            model,
            issue,
            pr,
            repo,
            text,
        } => {
            let repo = repo_or_environment(repo);
            let work = work_tags(
                &issue,
                &pr,
                repo.as_deref(),
                env::var(SESSION_ENV)
                    .ok()
                    .filter(|session| !session.is_empty()),
            )?;
            let server = find_server(server)?;
            send(
                &server,
                &seed_from_environment()?,
                from,
                to,
                answer,
                Options {
                    github,
                    model,
                    work,
                },
                text,
            )
            .await
        }
        Command::Cancel {
            server,
            from,
            to,
            task,
        } => {
            let server = find_server(server)?;
            cancel(&server, &seed_from_environment()?, from, to, task).await
        }
        Command::Listen {
            server,
            since,
            agent,
        } => {
            let server = find_server(server)?;
            listen::listen(&server, &seed_from_environment()?, &agent, since).await
        }
        #[cfg(feature = "record")]
        Command::Record(command) => report::run(command).await,
        Command::Agents { server, json } => {
            let server = find_server(server)?;
            agents(&server, &seed_from_environment()?, json).await
        }
        Command::Guest {
            server,
            name,
            reply_to,
            workdir,
            model,
        } => Box::pin(run_guest(server, &name, &reply_to, &workdir, &model)).await,
    }
}

/// Run a guest agent, with its secrets from stdin.
async fn run_guest(
    server: String,
    name: &str,
    reply_to: &str,
    workdir: &Path,
    model: &str,
) -> Result<()> {
    let stdin = io::stdin();
    if stdin.is_terminal() {
        bail!(
            "agent-bus guest reads its secrets from stdin, which is a terminal: pipe them to it instead (see dev/agent-bus/start-guest.sh)"
        );
    }
    let secrets = guest::read_secrets(stdin.lock())?;
    let server = Endpoint {
        url: server,
        ca: tls::client_ca()?,
    };
    guest::run(&server, &secrets, name, reply_to, workdir, model).await
}

/// Print spans to stderr when `RUST_LOG` is set, such as `RUST_LOG=agent_bus=debug`: one line
/// as each span closes, with its fields and how long it took. Without `RUST_LOG` nothing is
/// installed and the output is as it always was. stdout is left alone: `listen`, `send` and
/// `enroll` print what scripts read there.
fn init_tracing() {
    let Some(directives) = env::var_os(EnvFilter::DEFAULT_ENV).filter(|value| !value.is_empty())
    else {
        return;
    };
    let filter = match EnvFilter::try_new(directives.to_string_lossy()) {
        Ok(filter) => filter,
        Err(error) => {
            eprintln!("Ignoring {}: {error}", EnvFilter::DEFAULT_ENV);
            return;
        }
    };
    let stderr = io::stderr();
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(io::stderr)
        .with_ansi(stderr.is_terminal())
        .with_span_events(FmtSpan::CLOSE)
        .try_init();
}

/// Run a `bus` subcommand.
async fn bus_command(action: BusAction) -> Result<()> {
    match action {
        BusAction::Start {
            lab,
            listen,
            agents,
        } => {
            let (listen, lab) = match (lab, listen) {
                (Some(dir), _) => {
                    let (gateway, record) = state::lab_gateway(&dir)?;
                    (gateway, Some(record))
                }
                (None, Some(listen)) => (listen, None),
                (None, None) => bail!("Give --lab or --listen"),
            };
            bus::start(
                &Bus::standard(listen, lab)?,
                &agents,
                &seed_from_environment()?,
            )
            .await
        }
        BusAction::Stop => bus::stop().await,
        BusAction::Status => bus::status().await,
    }
}

/// Enroll a guest on the recorded bus and print its new seed, alone, on stdout. Refused when
/// stdout is a terminal, so the seed does not land on the screen or in a scrollback.
async fn enroll(name: &str) -> Result<()> {
    if io::stdout().is_terminal() {
        bail!(
            "enroll prints the new seed on stdout, which is a terminal: pipe it to its agent instead (see dev/agent-bus/start-agent.sh)"
        );
    }
    let seed = seed_from_environment()?;
    let (bus, agents) = Bus::recorded()?;
    let new_seed = bus::enroll(&bus, &agents, name, &seed).await?;
    println!("{new_seed}");
    Ok(())
}

/// The bus to connect to: `--server`, then `AGENT_BUS_URL`, then where `bus start` recorded it;
/// trusting the CA certificate in `AGENT_BUS_CA`, else the host's.
fn find_server(flag: Option<String>) -> Result<Endpoint> {
    Ok(Endpoint {
        url: state::find_server(
            flag,
            std::env::var(state::SERVER_ENV).ok(),
            &Bus::standard_record_path()?,
        )?,
        ca: tls::client_ca()?,
    })
}

/// Print a new key pair. This is the only place a seed is printed.
fn new_key() -> Result<()> {
    let key = keys::generate()?;
    println!("Public key (goes in the agents file, not secret):");
    println!("{}", key.public_key);
    println!(
        "Seed (SECRET: store it in a secret store, and give it only to this agent as AGENT_BUS_SEED; it is not shown again):"
    );
    println!("{}", key.seed);
    Ok(())
}

/// `repo`, else `AGENT_BUS_REPO` when it is set and not empty.
fn repo_or_environment(repo: Option<String>) -> Option<String> {
    repo.or_else(|| env::var(REPO_ENV).ok().filter(|repo| !repo.is_empty()))
}

/// The `work` of a task tagged with `issues` and `pull_requests` in `repo`, sent from the host's
/// Claude Code `session` when known; `None` without tags.
///
/// # Errors
///
/// Fails when there are tags and no repository, or the repository is invalid.
fn work_tags(
    issues: &[u64],
    pull_requests: &[u64],
    repo: Option<&str>,
    session: Option<String>,
) -> Result<Option<Work>> {
    if issues.is_empty() && pull_requests.is_empty() {
        return Ok(None);
    }
    let Some(repo) = repo else {
        bail!(
            "Give the repository of --issue and --pr with --repo or {REPO_ENV}, such as bwks/sherpa"
        );
    };
    check_repo(repo)?;
    let tag = |kind: RefKind, number: &u64| WorkRef {
        repo: repo.to_owned(),
        kind,
        number: *number,
    };
    let refs = issues
        .iter()
        .map(|number| tag(RefKind::Issue, number))
        .chain(
            pull_requests
                .iter()
                .map(|number| tag(RefKind::PullRequest, number)),
        )
        .collect();
    Ok(Some(Work { refs, session }))
}

/// Send a new task, or an answer to a task, and print the task's ID.
async fn send(
    server: &Endpoint,
    seed: &str,
    from: String,
    to: String,
    answer: Option<String>,
    options: Options,
    text: String,
) -> Result<()> {
    check_agent_name(&from)?;
    check_agent_name(&to)?;
    let (task, kind) = match answer {
        Some(task) => (task, Kind::Answer),
        None => (new_task_id(), Kind::Task),
    };
    let channel = Channel::connect(server, seed).await?;
    channel
        .send(&Message {
            task: task.clone(),
            from,
            to,
            kind,
            text,
            usage: None,
            task_usage: None,
            github: options.github,
            model: options.model,
            work: options.work,
            ended: None,
        })
        .await?;
    println!("{task}");
    Ok(())
}

/// Ask a guest agent to cancel a task. The outcome comes back as a `canceled` or `failed` message.
async fn cancel(
    server: &Endpoint,
    seed: &str,
    from: String,
    to: String,
    task: String,
) -> Result<()> {
    check_agent_name(&from)?;
    check_agent_name(&to)?;
    let channel = Channel::connect(server, seed).await?;
    channel
        .send(&Message {
            task,
            from,
            to,
            kind: Kind::Cancel,
            text: String::new(),
            usage: None,
            task_usage: None,
            github: false,
            model: None,
            work: None,
            ended: None,
        })
        .await
}

/// Print every agent of the recorded bus, as a table or as JSON. Nothing is consumed: the host
/// reads each inbox consumer's info and each guest's status.
async fn agents(server: &Endpoint, seed: &str, json: bool) -> Result<()> {
    let (bus, agents_path) = Bus::recorded()?;
    let agents = bus::agents(&bus, &agents_path, seed)?;
    let channel = Channel::connect(server, seed).await?;
    let names = || agents.all().map(|agent| agent.name.as_str());
    let statuses = channel.statuses(names()).await?;
    if statuses.is_none() {
        eprintln!(
            "The bus has no status bucket: it was prepared by an older agent-bus. Run \
             `agent-bus bus start` again to create it"
        );
    }
    let mut statuses = statuses.unwrap_or_default();
    let record = channel.consumer_state(RECORDER).await?;
    let mut observed = Vec::new();
    for agent in agents.all() {
        let heartbeat = statuses
            .iter()
            .position(|(name, _)| *name == agent.name)
            .map(|index| statuses.swap_remove(index).1);
        observed.push(Observed {
            name: agent.name.clone(),
            role: agent.role,
            inbox: channel.consumer_state(&agent.name).await?,
            heartbeat,
            record: record.filter(|_| agent.role == Role::Host),
        });
    }
    let now = now_ms();
    let rows: Vec<status::Row> = observed
        .iter()
        .map(|agent| status::row(agent, now, STALE))
        .collect();
    if json {
        println!("{}", serde_json::to_string_pretty(&rows)?);
    } else {
        println!("{}", table::agents(&rows));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::fmt::{self, Write};
    use std::fs;
    use std::io::Write as _;
    use std::net::TcpListener;
    use std::os::unix::fs::PermissionsExt;
    use std::path::Path;
    use std::process::{self, Stdio};
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::{Arc, Mutex};

    use anyhow::{Context, ensure};
    use bollard::query_parameters::RemoveVolumeOptions;
    use clap::error::ErrorKind;
    use container::{docker_connection, remove_container};
    use nix::sys::signal::{Signal, kill};
    use nix::unistd::Pid;
    use tokio::time::{Instant, sleep, timeout};
    use tracing::field::{Field, Visit};
    use tracing::span::{Attributes, Id, Record};
    use tracing::{Event, Metadata, Subscriber};

    #[cfg(feature = "record")]
    use crate::record::{BusAccess, Paths, Record as HostRecord};
    #[cfg(feature = "record")]
    use crate::store::{Ref, items, ref_item};

    /// A subscriber that keeps every span, with its fields, and every event as text.
    #[derive(Clone, Default)]
    struct Capture {
        seen: Arc<Mutex<Vec<String>>>,
        next_id: Arc<AtomicU64>,
    }

    /// Appends ` name=value` for each field.
    struct Fields<'a>(&'a mut String);

    impl Visit for Fields<'_> {
        fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
            write!(self.0, " {}={value:?}", field.name()).unwrap();
        }
    }

    impl Capture {
        fn keep(&self, text: String) {
            self.seen.lock().unwrap().push(text);
        }

        fn seen(&self) -> Vec<String> {
            self.seen.lock().unwrap().clone()
        }
    }

    impl Subscriber for Capture {
        fn enabled(&self, _: &Metadata<'_>) -> bool {
            true
        }

        fn new_span(&self, span: &Attributes<'_>) -> Id {
            let mut text = span.metadata().name().to_owned();
            span.record(&mut Fields(&mut text));
            self.keep(text);
            Id::from_u64(self.next_id.fetch_add(1, Ordering::Relaxed) + 1)
        }

        fn record(&self, _: &Id, values: &Record<'_>) {
            let mut text = "record".to_owned();
            values.record(&mut Fields(&mut text));
            self.keep(text);
        }

        fn record_follows_from(&self, _: &Id, _: &Id) {}

        fn event(&self, event: &Event<'_>) {
            let mut text = "event".to_owned();
            event.record(&mut Fields(&mut text));
            self.keep(text);
        }

        fn enter(&self, _: &Id) {}

        fn exit(&self, _: &Id) {}
    }

    fn send(extra: &[&str]) -> Result<Cli, clap::Error> {
        let mut args = vec!["agent-bus", "send", "--to", "dev"];
        args.extend_from_slice(extra);
        args.push("text");
        Cli::try_parse_from(args)
    }

    #[test]
    fn an_answer_refuses_a_model() {
        let error = send(&["--answer", "t1", "--model", "opus"]).err().unwrap();
        assert_eq!(error.kind(), ErrorKind::ArgumentConflict);
        let text = error.to_string();
        assert!(text.contains("--answer"), "{text}");
        assert!(text.contains("--model"), "{text}");
    }

    #[test]
    fn an_answer_refuses_github() {
        let error = send(&["--answer", "t1", "--github"]).err().unwrap();
        assert_eq!(error.kind(), ErrorKind::ArgumentConflict);
        let text = error.to_string();
        assert!(text.contains("--answer"), "{text}");
        assert!(text.contains("--github"), "{text}");
    }

    #[test]
    fn a_task_takes_a_model_and_github() {
        let cli = send(&["--model", "haiku", "--github"]).unwrap();
        let Command::Send {
            github,
            model,
            answer,
            ..
        } = cli.command
        else {
            panic!("not a send");
        };
        assert!(github);
        assert_eq!(model.as_deref(), Some("claude-haiku-5-5"));
        assert_eq!(answer, None);
    }

    #[test]
    fn an_answer_alone_parses() {
        let cli = send(&["--answer", "t1"]).unwrap();
        let Command::Send {
            github,
            model,
            answer,
            ..
        } = cli.command
        else {
            panic!("not a send");
        };
        assert!(!github);
        assert_eq!(model, None);
        assert_eq!(answer.as_deref(), Some("t1"));
    }

    #[test]
    fn a_task_takes_issue_and_pull_request_tags_and_an_answer_refuses_them() {
        let cli = send(&[
            "--issue", "580", "--pr", "612", "--pr", "613", "--repo", "o/r",
        ])
        .unwrap();
        let Command::Send {
            issue, pr, repo, ..
        } = cli.command
        else {
            panic!("not a send");
        };
        assert_eq!((issue, pr), (vec![580], vec![612, 613]));
        assert_eq!(repo.as_deref(), Some("o/r"));
        for tag in ["--issue", "--pr", "--repo"] {
            let error = send(&["--answer", "t1", tag, "1"]).err().unwrap();
            assert_eq!(error.kind(), ErrorKind::ArgumentConflict, "{tag}");
        }
        assert!(send(&["--issue", "x"]).is_err());
    }

    #[test]
    fn work_tags_need_a_repository_and_carry_the_session() {
        assert_eq!(
            work_tags(&[], &[], None, Some("s".to_owned())).unwrap(),
            None
        );
        let error = work_tags(&[580], &[], None, None).unwrap_err();
        assert!(error.to_string().contains(REPO_ENV), "{error}");
        assert!(work_tags(&[580], &[], Some("not a repo"), None).is_err());
        let work = work_tags(&[580], &[612], Some("o/r"), Some("sess".to_owned()))
            .unwrap()
            .unwrap();
        assert_eq!(
            work,
            Work {
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
                session: Some("sess".to_owned()),
            }
        );
    }

    #[cfg(feature = "record")]
    #[test]
    fn the_work_commands_parse_their_item_and_refuse_two() {
        let parse = |args: &[&str]| {
            let mut all = vec!["agent-bus"];
            all.extend_from_slice(args);
            Cli::try_parse_from(all)
        };
        for args in [
            &["link", "t1", "--issue", "580"][..],
            &["link", "t1", "--branch", "feat/x", "--repo", "o/r"],
            &["work", "show"],
            &["work", "show", "--pr", "612", "--json"],
            &["work", "backfill", "--repo", "o/r"],
            &["usage", "--issue", "580"],
            &["usage", "--by", "week"],
            &["usage", "--by", "model", "--agent", "dev01"],
            &["open", "--json"],
        ] {
            assert!(parse(args).is_ok(), "{args:?}");
        }
        for args in [
            &["link", "t1", "--issue", "580", "--pr", "612"][..],
            &["work", "show", "--issue", "1", "--branch", "x"],
            &["usage", "--by", "week", "--issue", "580"],
            &["usage", "--by", "month"],
        ] {
            assert!(parse(args).is_err(), "{args:?}");
        }
    }

    #[test]
    fn the_record_commands_are_there_only_with_the_record_feature() {
        for command in ["usage", "show", "history", "open", "link", "work"] {
            let kind = Cli::try_parse_from(["agent-bus", command, "--help"])
                .err()
                .map(|error| error.kind());
            let expected = if cfg!(feature = "record") {
                ErrorKind::DisplayHelp
            } else {
                ErrorKind::InvalidSubcommand
            };
            assert_eq!(kind, Some(expected), "{command}");
        }
        // The guest's commands are there either way.
        for command in ["guest", "agents", "listen", "send"] {
            let kind = Cli::try_parse_from(["agent-bus", command, "--help"])
                .err()
                .map(|error| error.kind());
            assert_eq!(kind, Some(ErrorKind::DisplayHelp), "{command}");
        }
    }

    /// Every instrumented function that takes a seed, or the guest's secrets, records its span
    /// without them. Each call
    /// fails early (no CA, no agents file, a bad name), after its span is made.
    #[tokio::test]
    async fn a_seed_never_appears_in_a_span() {
        let capture = Capture::default();
        let _guard = tracing::subscriber::set_default(capture.clone());
        let seed = keys::generate().unwrap().seed;
        let missing = Path::new("/nonexistent/agent-bus-test");
        let server = Endpoint {
            url: "nats://127.0.0.1:1".to_owned(),
            ca: missing.join("ca.pem"),
        };
        let bus = Bus {
            container: "agent-bus-test".to_owned(),
            volume: "agent-bus-test".to_owned(),
            listen: Ipv4Addr::LOCALHOST,
            port: 1,
            state_dir: missing.to_owned(),
            lab: None,
        };
        let agents_file = missing.join("agents.toml");

        assert!(Channel::connect(&server, &seed).await.is_err());
        assert!(bus::start(&bus, &agents_file, &seed).await.is_err());
        assert!(
            bus::enroll(&bus, &agents_file, "guest1", &seed)
                .await
                .is_err()
        );
        assert!(bus::agents(&bus, &agents_file, &seed).is_err());
        assert!(
            listen::listen(&server, &seed, "not a name", None)
                .await
                .is_err()
        );
        let claude = "sk-ant-oat01-spantest0123456789";
        let write = "github_pat_spantest0123456789";
        let secrets =
            guest::read_secrets(format!("{claude}\n{write}\n{seed}\n").as_bytes()).unwrap();
        assert!(
            guest::run(
                &server,
                &secrets,
                "not a name",
                "host",
                missing,
                DEFAULT_MODEL
            )
            .await
            .is_err()
        );

        let spans = capture.seen();
        for name in ["connect", "start", "enroll", "agents", "listen", "run"] {
            assert!(
                spans
                    .iter()
                    .any(|span| span.starts_with(&format!("{name} "))),
                "no {name} span in {spans:#?}"
            );
        }
        assert!(
            spans
                .iter()
                .any(|span| span.contains("url=nats://127.0.0.1:1")),
            "{spans:#?}"
        );
        for span in &spans {
            for secret in [seed.as_str(), claude, write] {
                assert!(!span.contains(secret), "a span holds a secret: {span}");
            }
        }
    }

    /// The variable that makes `guest_agent_process` run a guest agent: the command line, one
    /// argument per line. It holds no secret.
    const CHILD_ARGS: &str = "AGENT_BUS_TEST_GUEST_ARGS";
    /// What the agent writes once it is connected and reading its inbox.
    const WAITING: &str = "is waiting for tasks";

    /// A stand-in for `claude` that collects, in its working directory, everything a task can
    /// read of its agent: the agent's environment and command line, its stdin, its state files
    /// and its log; its own environment, kept apart; and whether it may open the agent's memory.
    const REACHING_CLAUDE: &str = r#"#!/bin/bash
read -r prompt
agent=$PPID
{
    echo "== agent environ"; tr '\0' '\n' < "/proc/$agent/environ"
    echo "== agent cmdline"; tr '\0' ' ' < "/proc/$agent/cmdline"; echo
    echo "== agent stdin"; timeout 2 cat "/proc/$agent/fd/0"
    echo "== agent state"; find "$XDG_STATE_HOME" -type f -exec cat {} +
    echo "== agent log"; cat "$AGENT_LOG"
} > "reach.$$" 2>&1
tr '\0' '\n' < "/proc/$$/environ" > "self-env.$$"
if (exec 3< "/proc/$agent/mem") 2> /dev/null; then echo open; else echo denied; fi > "mem.$$"
echo '{"type":"system","subtype":"init","session_id":"sess-1"}'
echo '{"type":"result","is_error":false,"result":"done","session_id":"sess-1"}'
"#;

    /// Not a test on its own: the guest agent process that
    /// `a_guest_agents_secrets_are_out_of_reach_of_its_tasks` starts, running `agent-bus guest`
    /// as `main` does, with its secrets on stdin. Without `CHILD_ARGS` it does nothing.
    #[tokio::test]
    #[ignore = "the guest agent process of another test"]
    async fn guest_agent_process() {
        let Some(args) = env::var_os(CHILD_ARGS) else {
            return;
        };
        let _ = aws_lc_rs::default_provider().install_default();
        let cli = Cli::try_parse_from(args.to_string_lossy().lines()).unwrap();
        execute(cli).await.unwrap();
    }

    /// The `== name` section of a `reach` file.
    fn section<'t>(text: &'t str, name: &str) -> &'t str {
        let start = text.find(&format!("== {name}\n")).unwrap();
        let rest = &text[start..];
        rest[1..].find("\n== ").map_or(rest, |end| &rest[..=end])
    }

    /// A directory for `a_guest_agents_secrets_are_out_of_reach_of_its_tasks`: the agents file
    /// with `host` and the guest `dev`, `bin/claude` (`REACHING_CLAUDE`) and the checkout `work`.
    fn secrets_test_dir(host: &keys::NewKey, guest: &keys::NewKey) -> PathBuf {
        let dir = env::temp_dir().join(format!("agent-bus-secrets-{}", process::id()));
        let _ = fs::remove_dir_all(&dir);
        for path in [dir.join("bin"), dir.join("work")] {
            fs::create_dir_all(path).unwrap();
        }
        fs::write(
            dir.join("agents.toml"),
            format!(
                "[[agent]]\nname = \"host\"\nkey = \"{}\"\nrole = \"host\"\n\
                 [[agent]]\nname = \"dev\"\nkey = \"{}\"\nrole = \"guest\"\n",
                host.public_key, guest.public_key
            ),
        )
        .unwrap();
        let claude = dir.join("bin").join("claude");
        fs::write(&claude, REACHING_CLAUDE).unwrap();
        fs::set_permissions(&claude, fs::Permissions::from_mode(0o755)).unwrap();
        dir
    }

    /// Start the guest agent `dev` in its own process, as `start-guest.sh` does: its secrets on
    /// stdin, never in its environment, and its output in a log a task can read.
    fn spawn_guest_agent(
        dir: &Path,
        server: &Endpoint,
        claude_token: &str,
        seed: &str,
    ) -> Result<process::Child> {
        let work = dir.join("work").to_string_lossy().into_owned();
        let args = [
            "agent-bus",
            "guest",
            "--server",
            &server.url,
            "--name",
            "dev",
            "--workdir",
            &work,
        ]
        .join("\n");
        let path = format!(
            "{}:{}",
            dir.join("bin").display(),
            env::var("PATH").unwrap_or_default()
        );
        let log = dir.join("agent-bus-dev.log");
        let output = fs::File::create(&log)?;
        let mut child = process::Command::new(env::current_exe()?)
            .args([
                "--exact",
                "tests::guest_agent_process",
                "--ignored",
                "--nocapture",
                "--test-threads=1",
            ])
            .env(CHILD_ARGS, &args)
            .env(tls::CA_ENV, &server.ca)
            .env("XDG_STATE_HOME", dir.join("guest-state"))
            .env("AGENT_LOG", &log)
            .env("PATH", path)
            .env_remove(keys::SEED_VARIABLE)
            .env_remove("CLAUDE_CODE_OAUTH_TOKEN")
            .stdin(Stdio::piped())
            .stdout(output.try_clone()?)
            .stderr(output)
            .spawn()?;
        // Written, then closed: the agent's stdin is empty from here on.
        child
            .stdin
            .take()
            .context("no stdin")?
            .write_all(format!("{claude_token}\n\n{seed}\n").as_bytes())?;
        Ok(child)
    }

    /// Wait until the agent in `child` is reading its inbox, as its `log` says.
    async fn wait_until_waiting(child: &mut process::Child, log: &Path) -> Result<()> {
        let deadline = Instant::now() + Duration::from_secs(60);
        while !fs::read_to_string(log)?.contains(WAITING) {
            if let Some(status) = child.try_wait()? {
                bail!("the agent exited ({status}): {}", fs::read_to_string(log)?);
            }
            if Instant::now() > deadline {
                bail!("the agent did not start: {}", fs::read_to_string(log)?);
            }
            sleep(Duration::from_millis(100)).await;
        }
        Ok(())
    }

    /// Send `dev` a task without --github as the host, and return its outcome. No reply may
    /// hold any of `secrets`.
    async fn run_task(server: &Endpoint, host_seed: &str, secrets: &[&str]) -> Result<Message> {
        let channel = Channel::connect(server, host_seed).await?;
        let mut inbox = channel.inbox("host", io::stderr()).await?;
        let task = new_task_id();
        channel
            .send(&Message {
                task: task.clone(),
                from: "host".to_owned(),
                to: "dev".to_owned(),
                kind: Kind::Task,
                text: "look around".to_owned(),
                usage: None,
                task_usage: None,
                github: false,
                model: None,
                work: None,
                ended: None,
            })
            .await?;
        loop {
            let delivery = timeout(Duration::from_secs(60), inbox.next())
                .await
                .context("no reply in time")??
                .context("the inbox closed")?;
            let message = delivery.message.clone();
            delivery.ack().await?;
            for secret in secrets {
                ensure!(!message.text.contains(secret), "a reply has a secret");
            }
            if message.task == task && message.kind != Kind::Progress {
                return Ok(message);
            }
        }
    }

    /// What each Claude Code run collected in `work`: nothing of its agent's holds the seed or
    /// the Claude token, its own environment has the Claude token and not the seed, and with
    /// `ptrace_scope` 1 or more it could not open the agent's memory.
    fn check_what_tasks_read(work: &Path, seed: &str, claude_token: &str) -> Result<()> {
        let ptrace_scope = fs::read_to_string("/proc/sys/kernel/yama/ptrace_scope")
            .ok()
            .and_then(|scope| scope.trim().parse::<u8>().ok())
            .unwrap_or(0);
        let mut runs = 0;
        for entry in fs::read_dir(work)? {
            let path = entry?.path();
            let name = path
                .file_name()
                .map_or_default(|name| name.to_string_lossy().into_owned());
            let text = fs::read_to_string(&path)?;
            if let Some(pid) = name.strip_prefix("reach.") {
                runs += 1;
                ensure!(!text.contains(seed), "run {pid} read the seed: {text}");
                ensure!(
                    !text.contains(claude_token),
                    "run {pid} read the Claude token from its agent: {text}"
                );
                // It did read each: they are just empty of secrets.
                ensure!(section(&text, "agent environ").contains("PATH="), "{text}");
                ensure!(
                    section(&text, "agent cmdline").contains("guest_agent_process"),
                    "{text}"
                );
                ensure!(section(&text, "agent log").contains(WAITING), "{text}");
            } else if name.starts_with("self-env.") {
                // The one place a task has the Claude token: its own environment, where Claude
                // Code takes it from. Never the seed.
                let expected = format!("CLAUDE_CODE_OAUTH_TOKEN={claude_token}");
                ensure!(
                    text.lines().any(|line| line == expected),
                    "a run had no Claude token"
                );
                ensure!(!text.contains(seed), "a run had the seed: {text}");
            } else if name.starts_with("mem.") && ptrace_scope >= 1 {
                ensure!(text.trim() == "denied", "a run opened its agent's memory");
            }
        }
        // Two priming runs and the task's own.
        ensure!(runs >= 3, "only {runs} runs looked");
        Ok(())
    }

    /// Stop the agent in `child` with SIGTERM, and kill it if it has not exited in time.
    async fn stop_agent(mut child: process::Child) {
        if let Ok(pid) = i32::try_from(child.id()) {
            let _ = kill(Pid::from_raw(pid), Signal::SIGTERM);
        }
        let deadline = Instant::now() + Duration::from_secs(20);
        while matches!(child.try_wait(), Ok(None)) && Instant::now() < deadline {
            sleep(Duration::from_millis(100)).await;
        }
        let _ = child.kill();
        let _ = child.wait();
    }

    /// Needs Docker. Starts a throwaway bus and a guest agent in its own process, with its
    /// secrets on stdin as `start-guest.sh` gives them, then shows that neither the seed nor
    /// the Claude token is in the agent's environment or command line, and that a task (a
    /// Claude Code run without --github) reads neither from anything of the agent's it can
    /// read. The task has the Claude token in its own environment only.
    #[tokio::test]
    #[ignore = "needs Docker"]
    async fn a_guest_agents_secrets_are_out_of_reach_of_its_tasks() {
        let _ = aws_lc_rs::default_provider().install_default();
        let (host, guest) = (keys::generate().unwrap(), keys::generate().unwrap());
        let claude_token = format!("sk-ant-oat01-{}", uuid::Uuid::new_v4().simple());
        let dir = secrets_test_dir(&host, &guest);
        let port = TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let id = process::id();
        let bus = Bus {
            container: format!("sherpa-agent-bus-secrets-test-{id}"),
            volume: format!("sherpa-agent-bus-secrets-test-{id}-data"),
            listen: Ipv4Addr::LOCALHOST,
            port,
            state_dir: dir.join("bus-state"),
            lab: None,
        };
        let server = Endpoint {
            url: format!("nats://127.0.0.1:{port}"),
            ca: bus.state_dir.join(tls::TLS_DIR).join(tls::CA_CERT_FILE),
        };
        let log = dir.join("agent-bus-dev.log");
        let mut agent = None;

        let result = async {
            bus::start(&bus, &dir.join("agents.toml"), &host.seed).await?;
            let child = agent.insert(spawn_guest_agent(
                &dir,
                &server,
                &claude_token,
                &guest.seed,
            )?);
            wait_until_waiting(child, &log).await?;

            // The agent's environment and command line, as any process of its user reads them.
            for file in ["environ", "cmdline"] {
                let text = fs::read(format!("/proc/{}/{file}", child.id()))?;
                let text = String::from_utf8_lossy(&text);
                ensure!(!text.contains(&guest.seed), "the seed is in its {file}");
                ensure!(
                    !text.contains(&claude_token),
                    "the Claude token is in its {file}"
                );
            }

            // A task without --github: the priming runs and the task's own run each look.
            let outcome = run_task(&server, &host.seed, &[&guest.seed, &claude_token]).await?;
            ensure!(outcome.kind == Kind::Completed, "{outcome:?}");
            check_what_tasks_read(&dir.join("work"), &guest.seed, &claude_token)?;

            let logged = fs::read_to_string(&log)?;
            ensure!(!logged.contains(&guest.seed), "the log has the seed");
            ensure!(
                !logged.contains(&claude_token),
                "the log has the Claude token"
            );
            Ok(())
        }
        .await;

        // Stop the agent, then clean up whatever happened.
        if let Some(child) = agent {
            stop_agent(child).await;
        }
        let docker = docker_connection().unwrap();
        let _ = remove_container(&docker, &bus.container).await;
        let _ = docker
            .remove_volume(&bus.volume, None::<RemoveVolumeOptions>)
            .await;
        if result.is_ok() {
            let _ = fs::remove_dir_all(&dir);
        }
        result.unwrap();
    }

    /// A stand-in for `claude` that answers every prompt, priming ones included, with `done`.
    #[cfg(feature = "record")]
    const DONE_CLAUDE: &str = r#"#!/bin/bash
read -r prompt
echo '{"type":"system","subtype":"init","session_id":"sess-1"}'
echo '{"type":"result","is_error":false,"result":"done","session_id":"sess-1"}'
"#;

    /// A stand-in for `gh`: the checkout is `o/r`, default branch `main`, and every branch has
    /// the open pull request 630.
    #[cfg(feature = "record")]
    const PULL_REQUEST_GH: &str = r#"#!/bin/bash
case "$1 $2" in
"repo view") echo '{"nameWithOwner":"o/r","defaultBranchRef":{"name":"main"}}' ;;
"pr list") echo '[{"number":630,"url":"https://github.com/o/r/pull/630","body":"","isCrossRepository":false}]' ;;
*) exit 2 ;;
esac
"#;

    /// A directory for `a_task_tagged_with_an_issue_becomes_an_item_with_the_guest_s_branch`:
    /// the agents file, `bin/claude` (`DONE_CLAUDE`), `bin/gh` (`PULL_REQUEST_GH`) and the
    /// checkout `work` on `feat/580-x`.
    #[cfg(feature = "record")]
    fn work_item_test_dir(host: &keys::NewKey, guest: &keys::NewKey) -> PathBuf {
        let dir = env::temp_dir().join(format!("agent-bus-work-item-{}", process::id()));
        let _ = fs::remove_dir_all(&dir);
        for path in [dir.join("bin"), dir.join("work")] {
            fs::create_dir_all(path).unwrap();
        }
        fs::write(
            dir.join("agents.toml"),
            format!(
                "[[agent]]\nname = \"host\"\nkey = \"{}\"\nrole = \"host\"\n\
                 [[agent]]\nname = \"dev\"\nkey = \"{}\"\nrole = \"guest\"\n",
                host.public_key, guest.public_key
            ),
        )
        .unwrap();
        for (name, script) in [("claude", DONE_CLAUDE), ("gh", PULL_REQUEST_GH)] {
            let path = dir.join("bin").join(name);
            fs::write(&path, script).unwrap();
            fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
        }
        let status = process::Command::new("git")
            .args(["init", "-q", "-b", "feat/580-x"])
            .current_dir(dir.join("work"))
            .status()
            .unwrap();
        assert!(status.success());
        dir
    }

    /// Send `dev` a task tagged with issue 580 of `o/r` as the host, wait for its outcome, then
    /// send, as `dev`, the outcome an older guest would have sent for another task: no `ended`,
    /// a Generation line. Returns the tagged task's ID and its outcome.
    #[cfg(feature = "record")]
    async fn tagged_task_and_an_old_outcome(
        server: &Endpoint,
        host_seed: &str,
        guest_seed: &str,
    ) -> Result<(String, Message)> {
        let channel = Channel::connect(server, host_seed).await?;
        let mut inbox = channel.inbox("host", io::stderr()).await?;
        let task = new_task_id();
        channel
            .send(&Message {
                task: task.clone(),
                from: "host".to_owned(),
                to: "dev".to_owned(),
                kind: Kind::Task,
                text: "work on it".to_owned(),
                usage: None,
                task_usage: None,
                github: false,
                model: None,
                work: work_tags(&[580], &[], Some("o/r"), Some("host-session".to_owned()))?,
                ended: None,
            })
            .await?;
        let outcome = loop {
            let delivery = timeout(Duration::from_secs(60), inbox.next())
                .await
                .context("no reply in time")??
                .context("the inbox closed")?;
            let message = delivery.message.clone();
            delivery.ack().await?;
            if message.task == task && message.kind != Kind::Progress {
                break message;
            }
        };
        let dev = Channel::connect(server, guest_seed).await?;
        dev.send(&Message {
            task: "0ld0ld00".to_owned(),
            from: "dev".to_owned(),
            to: "host".to_owned(),
            kind: Kind::Completed,
            text: "Generation section: no open pull request for branch fix/old\n\ndone".to_owned(),
            usage: None,
            task_usage: None,
            github: false,
            model: None,
            work: None,
            ended: None,
        })
        .await?;
        Ok((task, outcome))
    }

    /// The record after `tagged_task_and_an_old_outcome`: `task` alone in the item of issue 580,
    /// named also by the guest's branch and pull request, and the old outcome's branch naming
    /// another item.
    #[cfg(feature = "record")]
    async fn check_work_items(record: &HostRecord, task: &str) -> Result<()> {
        let issue = Ref::new("o/r", RefKind::Issue, "580");
        let item_id = ref_item(record.conn(), &issue)
            .await?
            .context("the issue names no item")?;
        let item = items::item(record.conn(), item_id).await?;
        let refs: Vec<(String, &str)> = item
            .refs
            .iter()
            .map(|named| (named.reference.qualified(), named.source.as_str()))
            .collect();
        ensure!(
            refs == [
                ("o/r branch feat/580-x".to_owned(), "guest"),
                ("o/r #580".to_owned(), "brief"),
                ("o/r PR #630".to_owned(), "guest"),
            ],
            "{refs:?}"
        );
        let tasks: Vec<&str> = item.tasks.iter().map(|task| task.task.as_str()).collect();
        ensure!(tasks == [task], "{tasks:?}");
        ensure!(item.name == "#580", "{}", item.name);
        let old = Ref::new("o/r", RefKind::Branch, "fix/old");
        let old_item = ref_item(record.conn(), &old)
            .await?
            .context("the old outcome's branch names no item")?;
        ensure!(old_item != item_id, "one item for both");
        Ok(())
    }

    /// Needs Docker. A task sent with `--issue` to a guest agent in its own process, whose
    /// checkout is on a branch with an open pull request, ends up in the record as one item:
    /// the issue (brief), and the guest's branch and pull request (guest). An older guest's
    /// outcome on the same bus is parsed into its own item. With `AGENT_BUS_KEEP_RECORD` set
    /// to a directory, the record is copied there for a `work backfill` by hand.
    #[cfg(feature = "record")]
    #[tokio::test]
    #[ignore = "needs Docker"]
    async fn a_task_tagged_with_an_issue_becomes_an_item_with_the_guest_s_branch() {
        let _ = aws_lc_rs::default_provider().install_default();
        let (host, guest) = (keys::generate().unwrap(), keys::generate().unwrap());
        let dir = work_item_test_dir(&host, &guest);
        let port = TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let id = process::id();
        let bus = Bus {
            container: format!("sherpa-agent-bus-work-test-{id}"),
            volume: format!("sherpa-agent-bus-work-test-{id}-data"),
            listen: Ipv4Addr::LOCALHOST,
            port,
            state_dir: dir.join("bus-state"),
            lab: None,
        };
        let server = Endpoint {
            url: format!("nats://127.0.0.1:{port}"),
            ca: bus.state_dir.join(tls::TLS_DIR).join(tls::CA_CERT_FILE),
        };
        let log = dir.join("agent-bus-dev.log");
        let paths = Paths::in_dir(&dir.join("record-state"));
        let mut agent = None;

        let result = async {
            bus::start(&bus, &dir.join("agents.toml"), &host.seed).await?;
            let child = agent.insert(spawn_guest_agent(
                &dir,
                &server,
                "sk-ant-oat01-worktest",
                &guest.seed,
            )?);
            wait_until_waiting(child, &log).await?;
            let (task, outcome) =
                tagged_task_and_an_old_outcome(&server, &host.seed, &guest.seed).await?;
            ensure!(outcome.kind == Kind::Completed, "{outcome:?}");
            ensure!(
                outcome
                    .ended
                    .as_ref()
                    .and_then(|ended| ended.branch.as_deref())
                    == Some("feat/580-x"),
                "{outcome:?}"
            );

            let access = BusAccess {
                server: server.clone(),
                seed: host.seed.clone(),
            };
            let record = HostRecord::open(&paths, Ok(access), Some("o/r")).await?;
            ensure!(record.behind().is_none(), "{:?}", record.behind());
            check_work_items(&record, &task).await?;
            drop(record);
            if let Some(keep) = env::var_os("AGENT_BUS_KEEP_RECORD") {
                fs::create_dir_all(&keep)?;
                for suffix in ["", "-wal"] {
                    let mut from = paths.record.as_os_str().to_owned();
                    from.push(suffix);
                    if Path::new(&from).exists() {
                        fs::copy(
                            &from,
                            Path::new(&keep).join(format!("{}{suffix}", store::FILE)),
                        )?;
                    }
                }
            }
            Ok(())
        }
        .await;

        if let Some(child) = agent {
            stop_agent(child).await;
        }
        let docker = docker_connection().unwrap();
        let _ = remove_container(&docker, &bus.container).await;
        let _ = docker
            .remove_volume(&bus.volume, None::<RemoveVolumeOptions>)
            .await;
        if result.is_ok() {
            let _ = fs::remove_dir_all(&dir);
        }
        result.unwrap();
    }
}
