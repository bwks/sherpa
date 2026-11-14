//! The commands that read the host's record: `usage`, `show`, `history`, `open`, `link` and
//! `work`. Each opens the record, caught up from the bus first, answers from it (`link` and
//! `work backfill` write to it) and closes it.

use std::time::Duration;

use anyhow::{Context, Result, bail, ensure};
use clap::{Args, Subcommand};

use crate::files::{millis, now_ms, standard_state_dir};
use crate::keys::seed_from_environment;
use crate::listen;
use crate::message::{RefKind, check_agent_name, check_repo};
use crate::record::{BusAccess, Paths, Record};
use crate::report::grouped::{self, By};
use crate::report::{history, usage, work};
use crate::store::items::{self, Item};
use crate::store::{Ref, backfill, link, read, ref_item};
use crate::{REPO_ENV, find_server, repo_or_environment};

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
        /// Only the tasks of the work item this issue, pull request or branch names
        #[command(flatten)]
        item: ItemArgs,
        /// The repository of --issue, --pr and --branch, `owner/name`. Without it:
        /// `AGENT_BUS_REPO`
        #[arg(long)]
        repo: Option<String>,
        /// Add the tasks up by agent, by model (each run's models; the task's own model for
        /// its priming runs) or by ISO week in UTC
        #[arg(long, value_enum, conflicts_with_all = ["issue", "pr", "branch"])]
        by: Option<By>,
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
    /// List the tasks still open (sent, running or waiting on a question) with their work item,
    /// their age and the question, from the host's record, first caught up from the bus
    Open {
        /// The bus to catch the record up from, for example `nats://192.0.2.1:4222`. Without
        /// it: `AGENT_BUS_URL`, then where `bus start` recorded the bus
        #[arg(long)]
        server: Option<String>,
        /// Print the tasks as JSON instead of a table
        #[arg(long)]
        json: bool,
    },
    /// Put a task in the work item an issue, pull request or branch names. When the ref names
    /// no item it names the task's (or a new one); when it names another item than the task's,
    /// the task's item is merged into it
    Link {
        /// The bus to catch the record up from, for example `nats://192.0.2.1:4222`. Without
        /// it: `AGENT_BUS_URL`, then where `bus start` recorded the bus
        #[arg(long)]
        server: Option<String>,
        /// The task
        task: String,
        #[command(flatten)]
        item: ItemArgs,
        /// The repository of --issue, --pr and --branch, `owner/name`. Without it:
        /// `AGENT_BUS_REPO`
        #[arg(long)]
        repo: Option<String>,
    },
    /// Work items: the issues, pull requests and branches that name a piece of work, and the
    /// tasks that worked on it
    Work {
        #[command(subcommand)]
        action: WorkAction,
    },
}

/// What `work` does.
#[derive(Subcommand)]
pub enum WorkAction {
    /// Print one item (its refs, its tasks with their cost, its conflicts and what was merged
    /// into it), or every item without --issue, --pr or --branch
    Show {
        /// The bus to catch the record up from, for example `nats://192.0.2.1:4222`. Without
        /// it: `AGENT_BUS_URL`, then where `bus start` recorded the bus
        #[arg(long)]
        server: Option<String>,
        #[command(flatten)]
        item: ItemArgs,
        /// The repository of --issue, --pr and --branch, `owner/name`. Without it:
        /// `AGENT_BUS_REPO`
        #[arg(long)]
        repo: Option<String>,
        /// Print it as JSON instead of tables
        #[arg(long)]
        json: bool,
    },
    /// Parse the Generation line of every recorded outcome that has no guest report (older
    /// guests, older messages) into refs of its task's item. Running it again changes nothing
    Backfill {
        /// The bus to catch the record up from, for example `nats://192.0.2.1:4222`. Without
        /// it: `AGENT_BUS_URL`, then where `bus start` recorded the bus
        #[arg(long)]
        server: Option<String>,
        /// The host's repository, `owner/name`, which a parsed line's branch is in when it
        /// names no pull request. Without it: `AGENT_BUS_REPO`
        #[arg(long)]
        repo: Option<String>,
    },
}

/// An issue, a pull request or a branch: one of them, naming a work item.
#[derive(Args, Debug, Clone, Default, PartialEq, Eq)]
#[group(multiple = false)]
pub struct ItemArgs {
    /// The item this issue number names
    #[arg(long)]
    pub issue: Option<u64>,
    /// The item this pull request number names
    #[arg(long)]
    pub pr: Option<u64>,
    /// The item this branch names
    #[arg(long)]
    pub branch: Option<String>,
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
            item,
            repo,
            by,
        } => {
            let filter = UsageFilter {
                host,
                agent,
                since,
                all,
            };
            print_usage(server, filter, &item, repo, by).await
        }
        Command::Show { server, host, task } => print_task(server, host, task).await,
        Command::History {
            server,
            agent,
            since,
            limit,
            json,
        } => print_history(server, agent, since, limit, json).await,
        Command::Open { server, json } => print_open(server, json).await,
        Command::Link {
            server,
            task,
            item,
            repo,
        } => link_task(server, &task, &item, repo).await,
        Command::Work { action } => match action {
            WorkAction::Show {
                server,
                item,
                repo,
                json,
            } => print_work(server, &item, repo, json).await,
            WorkAction::Backfill { server, repo } => run_backfill(server, repo).await,
        },
    }
}

/// Which tasks `usage` reports.
struct UsageFilter {
    host: String,
    agent: Option<String>,
    since: Option<Duration>,
    all: bool,
}

impl ItemArgs {
    /// The ref given, in `repo` (`--repo`, else `AGENT_BUS_REPO`); `None` when none is.
    ///
    /// # Errors
    ///
    /// Fails when a ref is given without a repository, or either is invalid.
    pub fn reference(&self, repo: Option<&str>) -> Result<Option<Ref>> {
        let (kind, name) = match (self.issue, self.pr, &self.branch) {
            (Some(issue), _, _) => (RefKind::Issue, issue.to_string()),
            (_, Some(pr), _) => (RefKind::PullRequest, pr.to_string()),
            (_, _, Some(branch)) => {
                ensure!(
                    !branch.is_empty() && !branch.chars().any(char::is_whitespace),
                    "Branch '{branch}' is not a branch name"
                );
                (RefKind::Branch, branch.clone())
            }
            (None, None, None) => return Ok(None),
        };
        let repo = repo.with_context(|| {
            format!("Give the repository with --repo or {REPO_ENV}, such as bwks/sherpa")
        })?;
        check_repo(repo)?;
        Ok(Some(Ref::new(repo, kind, name)))
    }
}

/// Print what each task `filter` selects cost, from the record: of the item `item` names only
/// when given, and added up `by` when given.
async fn print_usage(
    server: Option<String>,
    filter: UsageFilter,
    item: &ItemArgs,
    repo: Option<String>,
    by: Option<By>,
) -> Result<()> {
    check_agent_name(&filter.host)?;
    if let Some(agent) = &filter.agent {
        check_agent_name(agent)?;
    }
    let repo = repo_or_environment(repo);
    let target = item.reference(repo.as_deref())?;
    let record = open_record(server, repo.as_deref()).await?;
    let found = match &target {
        Some(target) => Some(find_item(&record, target).await?),
        None => None,
    };
    let rows = read::usage(
        record.conn(),
        &filter.host,
        filter.agent.as_deref(),
        filter.since.map(since_ms),
        filter.all,
        found.as_ref().map(|found| found.id),
    )
    .await?;
    let runs = match by {
        Some(By::Model) => items::run_parts(record.conn()).await?,
        _ => Vec::new(),
    };
    drop(record);
    if let Some(found) = &found {
        let refs: Vec<String> = found
            .refs
            .iter()
            .map(|named| named.reference.qualified())
            .collect();
        println!("Item {}: {} ({})", found.id, found.name, refs.join(", "));
    }
    match by {
        Some(by) => println!("{}", grouped::table(&grouped::group(&rows, &runs, by), by)),
        None => println!("{}", usage::table(&rows)),
    }
    Ok(())
}

/// Print the open tasks, as a table or as JSON.
async fn print_open(server: Option<String>, json: bool) -> Result<()> {
    let record = open_record(server, repo_or_environment(None).as_deref()).await?;
    let open = items::open_tasks(record.conn()).await?;
    drop(record);
    if json {
        println!("{}", serde_json::to_string_pretty(&open)?);
    } else {
        println!("{}", work::open(&open, now_ms()));
    }
    Ok(())
}

/// Put `task` in the item the ref in `item` names, and say what changed.
async fn link_task(
    server: Option<String>,
    task: &str,
    item: &ItemArgs,
    repo: Option<String>,
) -> Result<()> {
    let repo = repo_or_environment(repo);
    let Some(target) = item.reference(repo.as_deref())? else {
        bail!("Give the item to link the task to: --issue, --pr or --branch");
    };
    let mut record = open_record(server, repo.as_deref()).await?;
    let linked = link(record.conn_mut(), task, &target, now_ms()).await?;
    let found = find_item(&record, &target).await?;
    drop(record);
    println!("{}", work::linked(task, &linked, &found.name));
    Ok(())
}

/// Print the item the ref in `item` names, or every item without one.
async fn print_work(
    server: Option<String>,
    item: &ItemArgs,
    repo: Option<String>,
    json: bool,
) -> Result<()> {
    let repo = repo_or_environment(repo);
    let target = item.reference(repo.as_deref())?;
    let record = open_record(server, repo.as_deref()).await?;
    if let Some(target) = target {
        let found = find_item(&record, &target).await?;
        drop(record);
        if json {
            println!("{}", serde_json::to_string_pretty(&found)?);
        } else {
            println!("{}", work::item(&found));
        }
    } else {
        let all = items::items(record.conn()).await?;
        drop(record);
        if json {
            println!("{}", serde_json::to_string_pretty(&all)?);
        } else {
            println!("{}", work::items(&all));
        }
    }
    Ok(())
}

/// Parse the recorded outcomes without a guest report into refs, and say what changed.
async fn run_backfill(server: Option<String>, repo: Option<String>) -> Result<()> {
    let repo = repo_or_environment(repo);
    if let Some(repo) = &repo {
        check_repo(repo)?;
    }
    let mut record = open_record(server, repo.as_deref()).await?;
    let done = backfill(record.conn_mut(), repo.as_deref()).await?;
    drop(record);
    println!("{}", work::backfilled(&done));
    Ok(())
}

/// The item `target` names, read in full.
///
/// # Errors
///
/// Fails when no item has that ref, or the record cannot be read.
async fn find_item(record: &Record, target: &Ref) -> Result<Item> {
    let Some(id) = ref_item(record.conn(), target).await? else {
        bail!("No work item is named {}", target.qualified());
    };
    items::item(record.conn(), id).await
}

/// Print every recorded message of `task` that `host` sent or received, oldest first, as one
/// JSON line each.
async fn print_task(server: Option<String>, host: String, task: String) -> Result<()> {
    check_agent_name(&host)?;
    let record = open_record(server, repo_or_environment(None).as_deref()).await?;
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
    let record = open_record(server, repo_or_environment(None).as_deref()).await?;
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
/// then `AGENT_BUS_URL`, then where `bus start` recorded it) with the seed in `AGENT_BUS_SEED`,
/// and `home_repo` the host's repository for a parsed Generation line's branch. When it cannot
/// be caught up, it is opened as it is, and a warning says so.
async fn open_record(server: Option<String>, home_repo: Option<&str>) -> Result<Record> {
    let access = find_server(server).and_then(|server| {
        Ok(BusAccess {
            server,
            seed: seed_from_environment()?,
        })
    });
    let record = Record::open(&Paths::in_dir(&standard_state_dir()?), access, home_repo).await?;
    if let Some(warning) = record.behind() {
        eprintln!("{warning}");
    }
    Ok(record)
}

/// The time `since` ago, in milliseconds since the Unix epoch.
fn since_ms(since: Duration) -> u64 {
    now_ms().saturating_sub(millis(since))
}
