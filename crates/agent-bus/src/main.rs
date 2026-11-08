//! `agent-bus`: a development tool that gives the host agent and agents in lab guests a message
//! channel, over NATS with `JetStream`.
//!
//! The host runs the bus in a container (`bus start`). The host agent sends tasks (`send`) and
//! follows the replies (`listen`, one JSON line per message). A guest agent (`guest`) takes each
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
mod guest;
mod keys;
mod message;

use std::net::Ipv4Addr;
use std::path::PathBuf;

use anyhow::Result;
use clap::{Parser, Subcommand};

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
        /// The bus, for example `nats://172.31.3.1:4222`
        #[arg(long, env = "AGENT_BUS_URL")]
        server: String,
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
    /// Print each message sent to an agent, as one JSON line
    Listen {
        /// The bus, for example `nats://172.31.3.1:4222`
        #[arg(long, env = "AGENT_BUS_URL")]
        server: String,
        /// The agent whose messages to print
        agent: String,
    },
    /// Run a guest agent: take each task to Claude Code and reply with its progress and result
    Guest {
        /// The bus, for example `nats://172.31.3.1:4222`
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
    Start {
        /// The address to listen on, such as a lab's management bridge address
        #[arg(long)]
        listen: Ipv4Addr,
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
            BusAction::Start { listen, agents } => {
                bus::start(&Bus::standard(listen)?, &agents, &seed_from_environment()?).await
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
        Command::Listen { server, agent } => {
            listen(&server, &seed_from_environment()?, &agent).await
        }
        Command::Guest {
            server,
            name,
            reply_to,
            workdir,
        } => {
            let channel = Channel::connect(&server, &seed_from_environment()?).await?;
            guest::run(&channel, &name, &reply_to, &workdir).await
        }
    }
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
            github,
        })
        .await?;
    println!("{task}");
    Ok(())
}

/// Print each message sent to `agent` as one JSON line, until the bus closes.
async fn listen(server: &str, seed: &str, agent: &str) -> Result<()> {
    let channel = Channel::connect(server, seed).await?;
    let mut inbox = channel.inbox(agent).await?;
    while let Some(delivery) = inbox.next().await? {
        println!("{}", serde_json::to_string(&delivery.message)?);
        delivery.ack().await?;
    }
    Ok(())
}
