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
//! server's permissions are rendered from both. Each agent gets its seed in the environment
//! variable `AGENT_BUS_SEED`. A seed is never a command-line value.
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
mod registry;
mod report;
mod state;
mod status;
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

use bus::Bus;
use channel::{Channel, Endpoint};
use files::now_ms;
use keys::seed_from_environment;
use message::{Kind, Message, check_agent_name, new_task_id};
use model::{DEFAULT_MODEL, model_id};
use status::{Observed, STALE};

#[derive(Parser)]
#[command(
    version,
    about = "A NATS message channel between the host agent and guest agents",
    after_help = "Commands that connect to the bus read the agent's NKey seed from the \
                  environment variable AGENT_BUS_SEED. `agent-bus key new` makes the host's key \
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
    /// Remove an enrolled guest agent's key; the bus refuses it from then on. Its inbox and
    /// messages are kept
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
    /// Print what each task cost, from the outcome messages the bus still keeps for the host. The
    /// host's inbox is not consumed
    Usage {
        /// The bus, for example `nats://192.0.2.1:4222`. Without it: `AGENT_BUS_URL`, then
        /// where `bus start` recorded the bus
        #[arg(long)]
        server: Option<String>,
        /// The agent whose inbox to read
        #[arg(long, default_value = "host")]
        host: String,
        /// Only the tasks of this guest agent
        #[arg(long)]
        agent: Option<String>,
        /// Also list tasks from before usage was tracked, with n/a for their figures
        #[arg(long)]
        all: bool,
    },
    /// Print every message of a task kept in the host's inbox stream, oldest first, as one JSON
    /// line each. The host's inbox is not consumed
    Show {
        /// The bus, for example `nats://192.0.2.1:4222`. Without it: `AGENT_BUS_URL`, then
        /// where `bus start` recorded the bus
        #[arg(long)]
        server: Option<String>,
        /// The agent whose inbox to read
        #[arg(long, default_value = "host")]
        host: String,
        /// The task to print
        task: String,
    },
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
    /// Run a guest agent: take each task to Claude Code and reply with its progress and result
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
        /// Read the GitHub write token for --github tasks from the first line of stdin; an
        /// empty line is none. It is held in memory only, and set as `GH_TOKEN` for --github tasks
        /// alone. Without it, --github tasks fail
        #[arg(long)]
        github_token_stdin: bool,
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

/// What a new task may do and runs on. An answer carries neither: it has its task's.
struct Options {
    github: bool,
    model: Option<String>,
}

#[tokio::main]
async fn main() -> Result<()> {
    // The TLS connection to the bus takes rustls's process default, aws-lc-rs as in the rest
    // of the workspace. Already installed is fine.
    let _ = aws_lc_rs::default_provider().install_default();
    init_tracing();
    let cli = Cli::parse();
    match cli.command {
        Command::Key { action } => match action {
            KeyAction::New => new_key(),
        },
        Command::Bus { action } => bus_command(action).await,
        Command::Enroll { name } => enroll(&name).await,
        Command::Unenroll { name } => bus::unenroll_standard(&name).await,
        Command::Send {
            server,
            from,
            to,
            answer,
            github,
            model,
            text,
        } => {
            let server = find_server(server)?;
            send(
                &server,
                &seed_from_environment()?,
                from,
                to,
                answer,
                Options { github, model },
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
        Command::Usage {
            server,
            host,
            agent,
            all,
        } => {
            let server = find_server(server)?;
            usage(
                &server,
                &seed_from_environment()?,
                &host,
                agent.as_deref(),
                all,
            )
            .await
        }
        Command::Show { server, host, task } => {
            let server = find_server(server)?;
            show(&server, &seed_from_environment()?, &host, &task).await
        }
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
            github_token_stdin,
        } => {
            Box::pin(run_guest(
                server,
                &name,
                &reply_to,
                &workdir,
                &model,
                github_token_stdin,
            ))
            .await
        }
    }
}

/// Run a guest agent, with the GitHub write token from stdin when `github_token_stdin` is set.
async fn run_guest(
    server: String,
    name: &str,
    reply_to: &str,
    workdir: &Path,
    model: &str,
    github_token_stdin: bool,
) -> Result<()> {
    let seed = seed_from_environment()?;
    let write_token = if github_token_stdin {
        guest::read_write_token(io::stdin().lock())?
    } else {
        None
    };
    let server = Endpoint {
        url: server,
        ca: tls::client_ca()?,
    };
    guest::run(
        &server,
        &seed,
        name,
        reply_to,
        workdir,
        model,
        write_token.as_ref(),
    )
    .await
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
        })
        .await
}

/// Print what each task cost, from the messages kept in `host`'s inbox stream. Nothing is
/// acknowledged and the inbox is not consumed.
async fn usage(
    server: &Endpoint,
    seed: &str,
    host: &str,
    agent: Option<&str>,
    all: bool,
) -> Result<()> {
    if let Some(agent) = agent {
        check_agent_name(agent)?;
    }
    let channel = Channel::connect(server, seed).await?;
    let messages = channel.history(host).await?;
    println!("{}", report::table(&report::rows(&messages, agent, all)));
    Ok(())
}

/// Print every message of `task` kept in `host`'s inbox stream, oldest first, as one JSON line
/// each. Nothing is acknowledged and the inbox is not consumed.
async fn show(server: &Endpoint, seed: &str, host: &str, task: &str) -> Result<()> {
    let channel = Channel::connect(server, seed).await?;
    let messages = channel.history(host).await?;
    for message in report::task_messages(&messages, task) {
        println!("{}", serde_json::to_string(message)?);
    }
    Ok(())
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
    let mut observed = Vec::new();
    for agent in agents.all() {
        let heartbeat = statuses
            .iter()
            .position(|(name, _)| *name == agent.name)
            .map(|index| statuses.swap_remove(index).1);
        observed.push(Observed {
            name: agent.name.clone(),
            role: agent.role,
            inbox: channel.inbox_state(&agent.name).await?,
            heartbeat,
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
    use std::path::Path;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::{Arc, Mutex};

    use clap::error::ErrorKind;
    use tracing::field::{Field, Visit};
    use tracing::span::{Attributes, Id, Record};
    use tracing::{Event, Metadata, Subscriber};

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

    /// Every instrumented function that takes a seed records its span without it. Each call
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
        assert!(
            guest::run(
                &server,
                &seed,
                "not a name",
                "host",
                missing,
                DEFAULT_MODEL,
                None
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
            assert!(!span.contains(&seed), "a span holds the seed: {span}");
        }
    }
}
