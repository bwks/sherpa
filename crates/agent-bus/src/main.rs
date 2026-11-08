//! `agent-bus`: a development tool that gives the host agent and agents in lab guests a message
//! channel, over NATS with `JetStream`.
//!
//! The host runs the bus in a container (`bus start`). The host agent sends tasks (`send`) and
//! follows the replies (`listen`, one JSON line per message). A guest agent (`guest`) takes each
//! task to Claude Code and sends back its progress, questions and result.
#![deny(clippy::unwrap_used, clippy::expect_used)]

mod bus;
mod channel;
mod guest;
mod message;

use std::net::Ipv4Addr;
use std::path::PathBuf;

use anyhow::Result;
use clap::{Parser, Subcommand};

use channel::Channel;
use message::{Kind, Message, check_agent_name, new_task_id};

#[derive(Parser)]
#[command(
    version,
    about = "A NATS message channel between the host agent and guest agents"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
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
enum BusAction {
    /// Start the bus container, listening on one host address
    Start {
        /// The address to listen on, such as a lab's management bridge address
        #[arg(long)]
        listen: Ipv4Addr,
    },
    /// Stop and remove the bus container; its stored messages are kept
    Stop,
    /// Show whether the bus container is running
    Status,
}

#[tokio::main]
async fn main() -> Result<()> {
    match Cli::parse().command {
        Command::Bus { action } => match action {
            BusAction::Start { listen } => bus::start(listen).await,
            BusAction::Stop => bus::stop().await,
            BusAction::Status => bus::status().await,
        },
        Command::Send {
            server,
            from,
            to,
            answer,
            text,
        } => send(&server, from, to, answer, text).await,
        Command::Listen { server, agent } => listen(&server, &agent).await,
        Command::Guest {
            server,
            name,
            reply_to,
            workdir,
        } => {
            let channel = Channel::connect(&server).await?;
            guest::run(&channel, &name, &reply_to, &workdir).await
        }
    }
}

/// Send a new task, or an answer to a task, and print the task's ID.
async fn send(
    server: &str,
    from: String,
    to: String,
    answer: Option<String>,
    text: String,
) -> Result<()> {
    check_agent_name(&from)?;
    check_agent_name(&to)?;
    let (task, kind) = match answer {
        Some(task) => (task, Kind::Answer),
        None => (new_task_id(), Kind::Task),
    };
    let channel = Channel::connect(server).await?;
    channel
        .send(&Message {
            task: task.clone(),
            from,
            to,
            kind,
            text,
            usage: None,
        })
        .await?;
    println!("{task}");
    Ok(())
}

/// Print each message sent to `agent` as one JSON line, until the bus closes.
async fn listen(server: &str, agent: &str) -> Result<()> {
    let channel = Channel::connect(server).await?;
    let mut inbox = channel.inbox(agent).await?;
    while let Some(delivery) = inbox.next().await? {
        println!("{}", serde_json::to_string(&delivery.message)?);
        delivery.ack().await?;
    }
    Ok(())
}
