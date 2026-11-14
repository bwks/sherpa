//! The commands that read the host's record: `usage`, `show`, `history`, `open`, `link`, `work`
//! and `host`. Each opens the record, caught up from the bus first and with the host's
//! transcripts imported when `AGENT_BUS_HOST_PROJECT` is set, answers from it (`link`, `work`
//! and `host import` write to it) and closes it.

use std::env;
use std::time::Duration;

use anyhow::{Context, Result, bail, ensure};
use clap::{Args, Subcommand};
use jiff::Timestamp;

use crate::bus::{self, Bus};
use crate::files::{millis, now_ms, standard_state_dir};
use crate::host::import::Options;
use crate::keys::seed_from_environment;
use crate::listen;
use crate::message::{RefKind, check_agent_name, check_repo};
use crate::record::{BusAccess, Busy, HOOK_LOCK_WAIT, LOCK_WAIT, Paths, Record};
use crate::report::grouped::{self, By};
use crate::report::host::{self, HostAction};
use crate::report::{history, usage, work};
use crate::store::host::{
    Reassign, add_focus, attribute, current_focus, focus, host_rows, host_times, reassign,
    task_waits,
};
use crate::store::items::{self, Item};
use crate::store::{Ref, backfill, item_for, link, read, ref_item};
use crate::{REPO_ENV, SESSION_ENV, find_server, repo_or_environment};

/// How `work reassign` names the host's usage that no item has.
const UNATTRIBUTED: &str = "unattributed";

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
        /// Also list the tasks of agents no longer enrolled, which never finish
        #[arg(long)]
        all: bool,
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
    /// The host's own usage and time, imported from its Claude Code transcripts
    Host {
        #[command(subcommand)]
        action: HostAction,
    },
}

/// What `work` does.
#[derive(Subcommand)]
pub enum WorkAction {
    /// Print one item (its refs, its tasks with their cost, its conflicts and what was merged
    /// into it), or every item and the host's active item of each session without --issue,
    /// --pr or --branch
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
    /// Make the item an issue, pull request or branch names the host's active item: the host's
    /// usage from now on is charged to it. Run inside Claude Code (`CLAUDE_CODE_SESSION_ID`
    /// set), it is for that session; from a terminal, for every session
    Use {
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
    },
    /// No active item from now on: the host's usage is unattributed. For this Claude Code
    /// session, or every session from a terminal
    Clear {
        /// The bus to catch the record up from, for example `nats://192.0.2.1:4222`. Without
        /// it: `AGENT_BUS_URL`, then where `bus start` recorded the bus
        #[arg(long)]
        server: Option<String>,
    },
    /// Move the host's usage from one item, or from unattributed, to another, and pin it so it
    /// is never attributed again. Items are `issue:N`, `pr:N`, `branch:NAME` or `item:ID`
    Reassign {
        /// The bus to catch the record up from, for example `nats://192.0.2.1:4222`. Without
        /// it: `AGENT_BUS_URL`, then where `bus start` recorded the bus
        #[arg(long)]
        server: Option<String>,
        /// The item to move from, or `unattributed`
        #[arg(long)]
        from: String,
        /// The item to move to
        #[arg(long)]
        to: String,
        /// Only usage at or after this time, such as 2026-11-14T10:40:00Z
        #[arg(long)]
        since: Option<Timestamp>,
        /// Only usage before this time
        #[arg(long)]
        until: Option<Timestamp>,
        /// Only this Claude Code session's usage
        #[arg(long)]
        session: Option<String>,
        /// The repository of issue:, pr: and branch:, `owner/name`. Without it:
        /// `AGENT_BUS_REPO`
        #[arg(long)]
        repo: Option<String>,
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
        Command::Open { server, json, all } => print_open(server, json, all).await,
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
            WorkAction::Use { server, item, repo } => {
                let repo = repo_or_environment(repo);
                let Some(target) = item.reference(repo.as_deref())? else {
                    bail!("Give the item to work on: --issue, --pr or --branch");
                };
                set_focus(server, Some(&target), repo.as_deref()).await
            }
            WorkAction::Clear { server } => {
                set_focus(server, None, repo_or_environment(None).as_deref()).await
            }
            WorkAction::Reassign {
                server,
                from,
                to,
                since,
                until,
                session,
                repo,
            } => {
                let repo = repo_or_environment(repo);
                let from = ItemSpec::parse(&from, repo.as_deref())?;
                let to = ItemSpec::parse(&to, repo.as_deref())?;
                let window = Window {
                    since_ms: since.map(timestamp_ms).transpose()?,
                    until_ms: until.map(timestamp_ms).transpose()?,
                    session,
                };
                run_reassign(server, &from, &to, window, repo.as_deref()).await
            }
        },
        Command::Host { action } => {
            let server = match &action {
                HostAction::Import { server, .. } => server.clone(),
                HostAction::Status { .. } => None,
            };
            host::run(
                action,
                bus_access(server),
                repo_or_environment(None).as_deref(),
            )
            .await
        }
    }
}

/// An item named on the command line: `issue:N`, `pr:N`, `branch:NAME`, `item:ID`, or
/// `unattributed`.
#[derive(Debug, Clone, PartialEq, Eq)]
enum ItemSpec {
    Unattributed,
    Named(Ref),
    Id(i64),
}

/// Which of the host's usage `work reassign` moves, beside its item.
struct Window {
    since_ms: Option<u64>,
    until_ms: Option<u64>,
    session: Option<String>,
}

impl ItemSpec {
    /// The item `text` names, with `repo` the repository of a ref.
    ///
    /// # Errors
    ///
    /// Fails when `text` is none of the forms, or names a ref without a repository.
    fn parse(text: &str, repo: Option<&str>) -> Result<Self> {
        if text == UNATTRIBUTED {
            return Ok(Self::Unattributed);
        }
        let Some((kind, name)) = text.split_once(':') else {
            bail!("Item '{text}' must be issue:N, pr:N, branch:NAME, item:ID or {UNATTRIBUTED}");
        };
        let number = || {
            name.parse::<u64>()
                .with_context(|| format!("'{name}' in '{text}' is not a number"))
        };
        let args = match kind {
            "issue" => ItemArgs {
                issue: Some(number()?),
                ..ItemArgs::default()
            },
            "pr" => ItemArgs {
                pr: Some(number()?),
                ..ItemArgs::default()
            },
            "branch" => ItemArgs {
                branch: Some(name.to_owned()),
                ..ItemArgs::default()
            },
            "item" => {
                return Ok(Self::Id(name.parse().with_context(|| {
                    format!("'{name}' in '{text}' is not an item ID")
                })?));
            }
            _ => {
                bail!("Item '{text}' must be issue:N, pr:N, branch:NAME, item:ID or {UNATTRIBUTED}")
            }
        };
        let reference = args.reference(repo)?.context("The item names no ref")?;
        Ok(Self::Named(reference))
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
    let record = open_record(server, repo.as_deref(), LOCK_WAIT).await?;
    let found = match &target {
        Some(target) => Some(find_item(&record, target, repo.as_deref()).await?),
        None => None,
    };
    let item_id = found.as_ref().map(|found| found.id);
    let since = filter.since.map(since_ms);
    let mut rows = read::usage(
        record.conn(),
        &filter.host,
        filter.agent.as_deref(),
        since,
        filter.all,
        item_id,
    )
    .await?;
    let waits = task_waits(record.conn()).await?;
    for row in &mut rows {
        row.time.waiting = Some(waits.get(&row.task).copied().unwrap_or_default());
    }
    // The host's own usage, unless only another agent's is asked for.
    let host_usage = if filter
        .agent
        .as_deref()
        .is_none_or(|agent| agent == usage::HOST)
    {
        host_rows(record.conn(), since, item_id).await?
    } else {
        Vec::new()
    };
    let runs = match by {
        Some(By::Model) => items::run_parts(record.conn()).await?,
        _ => Vec::new(),
    };
    if by.is_some() {
        rows.extend(usage::host_group_rows(&host_usage));
    } else if filter
        .agent
        .as_deref()
        .is_none_or(|agent| agent == usage::HOST)
    {
        let mut times = host_times(record.conn(), since).await?;
        if let Some(id) = item_id {
            times.retain(|item, _| *item == Some(id));
        }
        rows.extend(usage::host_rows(&host_usage, &times));
    }
    let names = items::names(record.conn(), repo.as_deref()).await?;
    drop(record);
    for row in &mut rows {
        row.item = row.work.map(|work| {
            names
                .get(&work)
                .cloned()
                .unwrap_or_else(|| format!("item {work}"))
        });
    }
    if let Some(found) = &found {
        let refs: Vec<String> = found
            .refs
            .iter()
            .map(|named| named.reference.qualified())
            .collect();
        println!("Item {}: {} ({})", found.id, found.name, refs.join(", "));
    }
    if let Some(warning) = host::read_status(&Paths::in_dir(&standard_state_dir()?).host_status)
        .ok()
        .as_ref()
        .and_then(host::failing)
    {
        eprintln!("{warning}");
    }
    match by {
        Some(by) => println!("{}", grouped::table(&grouped::group(&rows, &runs, by), by)),
        None => println!("{}", usage::table(&rows)),
    }
    Ok(())
}

/// Print the open tasks, as a table or as JSON: those of agents no longer enrolled only with
/// `all`.
async fn print_open(server: Option<String>, json: bool, all: bool) -> Result<()> {
    let home_repo = repo_or_environment(None);
    let record = open_record(server, home_repo.as_deref(), LOCK_WAIT).await?;
    let open = items::open_tasks(record.conn(), home_repo.as_deref()).await?;
    drop(record);
    let open = match enrolled() {
        Ok(names) => {
            let (shown, hidden) = work::of_enrolled(open, &names, all);
            if hidden > 0 {
                eprintln!(
                    "Hiding {hidden} open tasks of agents no longer enrolled; --all shows them"
                );
            }
            shown
        }
        Err(error) => {
            eprintln!("Showing every open task: unable to read the enrolled agents: {error:#}");
            open
        }
    };
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
    let mut record = open_record(server, repo.as_deref(), LOCK_WAIT).await?;
    let linked = link(record.conn_mut(), task, &target, now_ms()).await?;
    let found = find_item(&record, &target, repo.as_deref()).await?;
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
    let record = open_record(server, repo.as_deref(), LOCK_WAIT).await?;
    if let Some(target) = target {
        let found = find_item(&record, &target, repo.as_deref()).await?;
        let responses = host_rows(record.conn(), None, Some(found.id)).await?;
        let mut times = host_times(record.conn(), None).await?;
        times.retain(|item, _| *item == Some(found.id));
        drop(record);
        if json {
            println!("{}", serde_json::to_string_pretty(&found)?);
        } else {
            println!("{}", work::item(&found));
            let mut host = usage::host_rows(&responses, &times);
            if !host.is_empty() {
                for row in &mut host {
                    row.item = Some(found.name.clone());
                }
                println!("{}", usage::table(&host));
            }
        }
    } else {
        let all = items::items(record.conn(), repo.as_deref()).await?;
        let current = current_focus(&focus(record.conn()).await?);
        let names = items::names(record.conn(), repo.as_deref()).await?;
        drop(record);
        if json {
            println!("{}", serde_json::to_string_pretty(&all)?);
        } else {
            println!("{}", work::items(&all));
            if let Some(active) = work::active(&current, &names) {
                println!("{active}");
            }
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
    let mut record = open_record(server, repo.as_deref(), LOCK_WAIT).await?;
    let done = backfill(record.conn_mut(), repo.as_deref()).await?;
    drop(record);
    println!("{}", work::backfilled(&done));
    Ok(())
}

/// The item `target` names, read in full, with `home_repo` the host's repository for its name.
///
/// # Errors
///
/// Fails when no item has that ref, or the record cannot be read.
async fn find_item(record: &Record, target: &Ref, home_repo: Option<&str>) -> Result<Item> {
    let Some(id) = ref_item(record.conn(), target).await? else {
        bail!("No work item is named {}", target.qualified());
    };
    items::item(record.conn(), id, home_repo).await
}

/// Print every recorded message of `task` that `host` sent or received, oldest first, as one
/// JSON line each.
async fn print_task(server: Option<String>, host: String, task: String) -> Result<()> {
    check_agent_name(&host)?;
    let record = open_record(server, repo_or_environment(None).as_deref(), LOCK_WAIT).await?;
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
    let record = open_record(server, repo_or_environment(None).as_deref(), LOCK_WAIT).await?;
    let rows = read::history(record.conn(), agent.as_deref(), since.map(since_ms), limit).await?;
    drop(record);
    if json {
        println!("{}", serde_json::to_string_pretty(&rows)?);
    } else {
        println!("{}", history::table(&rows));
    }
    Ok(())
}

/// What catching up needs: the bus at `server` (`--server`, then `AGENT_BUS_URL`, then where
/// `bus start` recorded it) and the seed in `AGENT_BUS_SEED`.
fn bus_access(server: Option<String>) -> Result<BusAccess> {
    let server = find_server(server)?;
    Ok(BusAccess {
        server,
        seed: seed_from_environment()?,
    })
}

/// The host's record in the state directory, caught up from the bus at `server`, with
/// `home_repo` the host's repository for a parsed Generation line's branch, and the host's
/// transcripts imported when `AGENT_BUS_HOST_PROJECT` names its project. When it cannot be
/// caught up or the import fails, it is opened as it is, and a warning says so. A record held
/// by another command is waited for up to `wait`.
async fn open_record(
    server: Option<String>,
    home_repo: Option<&str>,
    wait: Duration,
) -> Result<Record> {
    let paths = Paths::in_dir(&standard_state_dir()?);
    let mut record = Record::open(&paths, bus_access(server), home_repo, wait).await?;
    if let Some(warning) = record.behind() {
        eprintln!("{warning}");
    }
    if let Some(project) = host::configured(None) {
        let options = Options {
            project,
            rescan: false,
            reprice: false,
        };
        if let Err(error) =
            host::import_noted(&mut record, &paths.host_status, &options, now_ms()).await
        {
            eprintln!("Not imported the host's transcripts: {error:#}");
        }
    }
    Ok(record)
}

/// After `send`: catch the record up and import the host's transcripts, so the task's tag sets
/// the host's active item at once. Waits `HOOK_LOCK_WAIT` for a busy record, and only warns:
/// the task is sent whatever happens here.
pub async fn refresh_after_send(home_repo: Option<&str>) {
    match open_record(None, home_repo, HOOK_LOCK_WAIT).await {
        Ok(record) => drop(record),
        Err(error) if error.downcast_ref::<Busy>().is_some() => {}
        Err(error) => eprintln!("Sent, but the record was not updated: {error:#}"),
    }
}

/// Make the item `target` names (none without one) the host's active item from now on, for
/// this Claude Code session when run inside one, else for every session.
async fn set_focus(
    server: Option<String>,
    target: Option<&Ref>,
    home_repo: Option<&str>,
) -> Result<()> {
    let session = env::var(SESSION_ENV)
        .ok()
        .filter(|session| !session.is_empty());
    let record = open_record(server, home_repo, LOCK_WAIT).await?;
    let now = now_ms();
    let item = match target {
        Some(target) => Some(item_for(record.conn(), target, now).await?),
        None => None,
    };
    let source = if item.is_some() { "use" } else { "clear" };
    add_focus(record.conn(), now, session.as_deref(), item, source, None).await?;
    attribute(record.conn()).await?;
    let name = match item {
        Some(id) => Some(items::item(record.conn(), id, home_repo).await?.name),
        None => None,
    };
    drop(record);
    let scope = session.map_or_else(
        || "every session".to_owned(),
        |session| format!("session {session}"),
    );
    match name {
        Some(name) => println!("Active item for {scope}: {name}"),
        None => println!("No active item for {scope}"),
    }
    Ok(())
}

/// Move the host's usage `window` selects from `from` to `to`, pin it, and say how much moved.
async fn run_reassign(
    server: Option<String>,
    from: &ItemSpec,
    to: &ItemSpec,
    window: Window,
    home_repo: Option<&str>,
) -> Result<()> {
    let record = open_record(server, home_repo, LOCK_WAIT).await?;
    let from = match from {
        ItemSpec::Unattributed => None,
        ItemSpec::Id(id) => Some(*id),
        ItemSpec::Named(named) => Some(find_item(&record, named, home_repo).await?.id),
    };
    let to = match to {
        ItemSpec::Unattributed => bail!("Usage moves to an item, not to {UNATTRIBUTED}"),
        ItemSpec::Id(id) => *id,
        ItemSpec::Named(named) => item_for(record.conn(), named, now_ms()).await?,
    };
    let moved = reassign(
        record.conn(),
        &Reassign {
            from,
            to,
            since_ms: window.since_ms,
            until_ms: window.until_ms,
            session: window.session,
        },
    )
    .await?;
    let name = items::item(record.conn(), to, home_repo).await?.name;
    drop(record);
    println!("Moved {moved} host responses to {name}, pinned");
    Ok(())
}

/// The names of every agent of the recorded bus: the agents file's and the enrolled guests.
///
/// # Errors
///
/// Fails when the bus was never started here, the files are not valid, or `AGENT_BUS_SEED` is
/// not the host's.
fn enrolled() -> Result<Vec<String>> {
    let (bus, agents_path) = Bus::recorded()?;
    let agents = bus::agents(&bus, &agents_path, &seed_from_environment()?)?;
    Ok(agents.all().map(|agent| agent.name.clone()).collect())
}

/// `at` in milliseconds since the Unix epoch.
///
/// # Errors
///
/// Fails for a time before 1970.
fn timestamp_ms(at: Timestamp) -> Result<u64> {
    u64::try_from(at.as_millisecond()).context("The time is before 1970")
}

/// The time `since` ago, in milliseconds since the Unix epoch.
fn since_ms(since: Duration) -> u64 {
    now_ms().saturating_sub(millis(since))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reassign_takes_refs_ids_and_unattributed() {
        let repo = Some("o/r");
        assert_eq!(
            ItemSpec::parse("unattributed", repo).unwrap(),
            ItemSpec::Unattributed
        );
        assert_eq!(
            ItemSpec::parse("issue:590", repo).unwrap(),
            ItemSpec::Named(Ref::new("o/r", RefKind::Issue, "590"))
        );
        assert_eq!(
            ItemSpec::parse("pr:612", repo).unwrap(),
            ItemSpec::Named(Ref::new("o/r", RefKind::PullRequest, "612"))
        );
        assert_eq!(
            ItemSpec::parse("branch:feat/x", repo).unwrap(),
            ItemSpec::Named(Ref::new("o/r", RefKind::Branch, "feat/x"))
        );
        assert_eq!(ItemSpec::parse("item:7", None).unwrap(), ItemSpec::Id(7));
        for text in ["590", "issue:x", "pr:", "item:x", "label:bug", "branch:a b"] {
            assert!(ItemSpec::parse(text, repo).is_err(), "{text}");
        }
        // A ref needs a repository.
        assert!(ItemSpec::parse("issue:590", None).is_err());
    }
}
