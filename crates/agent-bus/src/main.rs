//! `agent-bus`: a development tool that gives the host agent and agents in lab guests a message
//! channel, over NATS with `JetStream`.
//!
//! The host runs the bus in a container (`bus start`). The host agent sends tasks (`send`) and
//! follows the replies (`listen`, one JSON line per message, kept in a journal first). A guest agent (`guest`) takes each
//! task to Claude Code and sends back its progress, questions and result.
//!
//! Every agent authenticates with an `NKey`. `key new` makes a key pair; the public keys go in the
//! agents file that `bus start` renders the server's permissions from, and each agent gets its
//! seed in the environment variable `AGENT_BUS_SEED`. A seed is never a command-line value.
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
mod report;
mod state;

use std::net::Ipv4Addr;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Result, bail};
use clap::{ArgGroup, Parser, Subcommand};

use bus::Bus;
use channel::Channel;
use keys::seed_from_environment;
use message::{Kind, Message, check_agent_name, new_task_id};

#[derive(Parser)]
#[command(
    version,
    about = "A NATS message channel between the host agent and guest agents",
    after_help = "Commands that connect to the bus read the agent's NKey seed from the \
                  environment variable AGENT_BUS_SEED. `agent-bus key new` makes a key pair."
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
        /// The model Claude Code runs on
        #[arg(long, default_value = guest::DEFAULT_MODEL)]
        model: String,
    },
}

#[derive(Subcommand)]
enum KeyAction {
    /// Generate a user key pair. The seed is printed once, here: store it in your secret
    /// store, and give it to its agent through `AGENT_BUS_SEED`
    New,
}

#[derive(Subcommand)]
enum BusAction {
    /// Start the bus container, listening on one host address
    #[command(group(ArgGroup::new("address").required(true).args(["lab", "listen"])))]
    Start {
        /// The lab to serve: a lab directory holding `lab-info.toml`. The bus listens on the
        /// lab's management gateway, which is where its guests find it
        #[arg(long)]
        lab: Option<PathBuf>,
        /// The address to listen on instead of a lab's
        #[arg(long)]
        listen: Option<Ipv4Addr>,
        /// The agents file (TOML): each agent's name, public key and role. The server's
        /// permissions are rendered from it. Run as the host: `AGENT_BUS_SEED` is the host's seed
        #[arg(long)]
        agents: PathBuf,
    },
    /// Stop and remove the bus container; its stored messages are kept
    Stop,
    /// Show whether the bus container is running
    Status,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Key { action } => match action {
            KeyAction::New => new_key(),
        },
        Command::Bus { action } => match action {
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
        },
        Command::Send {
            server,
            from,
            to,
            answer,
            github,
            text,
        } => {
            let server = find_server(server)?;
            send(
                &server,
                &seed_from_environment()?,
                from,
                to,
                answer,
                github,
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
        Command::Guest {
            server,
            name,
            reply_to,
            workdir,
            model,
        } => {
            let seed = seed_from_environment()?;
            guest::run(&server, &seed, &name, &reply_to, &workdir, &model).await
        }
    }
}

/// The bus to connect to: `--server`, then `AGENT_BUS_URL`, then where `bus start` recorded it.
fn find_server(flag: Option<String>) -> Result<String> {
    state::find_server(
        flag,
        std::env::var(state::SERVER_ENV).ok(),
        &Bus::record_path()?,
    )
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
    server: &str,
    seed: &str,
    from: String,
    to: String,
    answer: Option<String>,
    github: bool,
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
            github,
        })
        .await?;
    println!("{task}");
    Ok(())
}

/// Ask a guest agent to cancel a task. The outcome comes back as a `canceled` or `failed` message.
async fn cancel(server: &str, seed: &str, from: String, to: String, task: String) -> Result<()> {
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
        })
        .await
}

/// Print what each task cost, from the messages kept in `host`'s inbox stream. Nothing is
/// acknowledged and the inbox is not consumed.
async fn usage(server: &str, seed: &str, host: &str, agent: Option<&str>, all: bool) -> Result<()> {
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
async fn show(server: &str, seed: &str, host: &str, task: &str) -> Result<()> {
    let channel = Channel::connect(server, seed).await?;
    let messages = channel.history(host).await?;
    for message in report::task_messages(&messages, task) {
        println!("{}", serde_json::to_string(message)?);
    }
    Ok(())
}
