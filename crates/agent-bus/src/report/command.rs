//! The commands that read the host's record: `usage`, `show` and `history`. Each opens the
//! record, caught up from the bus first, answers from it and closes it.

use std::time::Duration;

use anyhow::Result;
use clap::Subcommand;

use crate::files::{millis, now_ms, standard_state_dir};
use crate::find_server;
use crate::keys::seed_from_environment;
use crate::listen;
use crate::message::check_agent_name;
use crate::record::{BusAccess, Paths, Record};
use crate::report::{history, usage};
use crate::store::read;

/// The commands that read the host's record, flattened into the CLI's commands.
#[derive(Subcommand)]
pub enum Command {
    /// Print what each task cost, from the host's record, first caught up from the bus. Run as
    /// the host: `AGENT_BUS_SEED` is the host's seed
    Usage {
        /// The bus to catch the record up from, for example `nats://192.0.2.1:4222`. Without
        /// it: `AGENT_BUS_URL`, then where `bus start` recorded the bus
        #[arg(long)]
        server: Option<String>,
        /// The agent that sent the tasks
        #[arg(long, default_value = "host")]
        host: String,
        /// Only the tasks of this guest agent
        #[arg(long)]
        agent: Option<String>,
        /// Only tasks with a message this long ago or later, such as `7d` (units s, m, h, d)
        #[arg(long, value_parser = listen::parse_since)]
        since: Option<Duration>,
        /// Also list tasks from before usage was tracked, with n/a for their figures
        #[arg(long)]
        all: bool,
    },
    /// Print every message of a task that the host sent or received, oldest first, as one JSON
    /// line each, from the host's record, first caught up from the bus
    Show {
        /// The bus to catch the record up from, for example `nats://192.0.2.1:4222`. Without
        /// it: `AGENT_BUS_URL`, then where `bus start` recorded the bus
        #[arg(long)]
        server: Option<String>,
        /// The agent whose messages to print
        #[arg(long, default_value = "host")]
        host: String,
        /// The task to print
        task: String,
    },
    /// List the newest tasks, from the host's record: their agent, state, model, last activity,
    /// cost and the start of their prompt. The record is first caught up from the bus
    History {
        /// The bus to catch the record up from, for example `nats://192.0.2.1:4222`. Without
        /// it: `AGENT_BUS_URL`, then where `bus start` recorded the bus
        #[arg(long)]
        server: Option<String>,
        /// Only the tasks of this guest agent
        #[arg(long)]
        agent: Option<String>,
        /// Only tasks with a message this long ago or later, such as `2d` (units s, m, h, d)
        #[arg(long, value_parser = listen::parse_since)]
        since: Option<Duration>,
        /// How many tasks to list
        #[arg(long, default_value_t = 20)]
        limit: u64,
        /// Print the tasks as JSON instead of a table
        #[arg(long)]
        json: bool,
    },
}

/// Run the record command `command` names.
///
/// # Errors
///
/// Fails when a name is invalid, or the record cannot be opened or read.
pub async fn run(command: Command) -> Result<()> {
    match command {
        Command::Usage {
            server,
            host,
            agent,
            since,
            all,
        } => print_usage(server, host, agent, since, all).await,
        Command::Show { server, host, task } => print_task(server, host, task).await,
        Command::History {
            server,
            agent,
            since,
            limit,
            json,
        } => print_history(server, agent, since, limit, json).await,
    }
}

/// Print what each task `host` sent cost, from the record.
async fn print_usage(
    server: Option<String>,
    host: String,
    agent: Option<String>,
    since: Option<Duration>,
    all: bool,
) -> Result<()> {
    check_agent_name(&host)?;
    if let Some(agent) = &agent {
        check_agent_name(agent)?;
    }
    let record = open_record(server).await?;
    let rows = read::usage(
        record.conn(),
        &host,
        agent.as_deref(),
        since.map(since_ms),
        all,
    )
    .await?;
    drop(record);
    println!("{}", usage::table(&rows));
    Ok(())
}

/// Print every recorded message of `task` that `host` sent or received, oldest first, as one
/// JSON line each.
async fn print_task(server: Option<String>, host: String, task: String) -> Result<()> {
    check_agent_name(&host)?;
    let record = open_record(server).await?;
    let messages = read::task_messages(record.conn(), &task, &host).await?;
    drop(record);
    for message in messages {
        println!("{}", serde_json::to_string(&message)?);
    }
    Ok(())
}

/// Print the newest `limit` tasks from the record, as a table or as JSON.
async fn print_history(
    server: Option<String>,
    agent: Option<String>,
    since: Option<Duration>,
    limit: u64,
    json: bool,
) -> Result<()> {
    if let Some(agent) = &agent {
        check_agent_name(agent)?;
    }
    let record = open_record(server).await?;
    let rows = read::history(record.conn(), agent.as_deref(), since.map(since_ms), limit).await?;
    drop(record);
    if json {
        println!("{}", serde_json::to_string_pretty(&rows)?);
    } else {
        println!("{}", history::table(&rows));
    }
    Ok(())
}

/// The host's record in the state directory, caught up from the bus at `server` (`--server`,
/// then `AGENT_BUS_URL`, then where `bus start` recorded it) with the seed in `AGENT_BUS_SEED`.
/// When it cannot be caught up, it is opened as it is, and a warning says so.
async fn open_record(server: Option<String>) -> Result<Record> {
    let access = find_server(server).and_then(|server| {
        Ok(BusAccess {
            server,
            seed: seed_from_environment()?,
        })
    });
    let record = Record::open(&Paths::in_dir(&standard_state_dir()?), access).await?;
    if let Some(warning) = record.behind() {
        eprintln!("{warning}");
    }
    Ok(record)
}

/// The time `since` ago, in milliseconds since the Unix epoch.
fn since_ms(since: Duration) -> u64 {
    now_ms().saturating_sub(millis(since))
}
