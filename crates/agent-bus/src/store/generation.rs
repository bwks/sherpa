//! What the Generation sync reads and keeps in the record: the pull requests that name an item,
//! which pull request counts each task and response (`placements`), the rows of a pull
//! request's section, and how each pull request's last write went (`generation_pr`).

use std::collections::{BTreeMap, BTreeSet, HashMap};

use anyhow::{Context, Result};
use tracing::instrument;
use turso::{Connection, Value};

use crate::generation::placement::{ItemPlan, Named, Placements, PullRequestId, place};
use crate::generation::section::{Counted, HostRow, Row, Rows};
use crate::generation::sync::is_frozen;
use crate::message::TaskUsage;
use crate::report::usage::{HOST, Time, add_host};
use crate::store::cells::{Cells, integer, optional_integer, optional_text};
use crate::store::host::{charged_pull_request, focus, host_model_times, host_rows, task_waits};
use crate::store::read;
use crate::store::sql::{
    GENERATION_PRS, HOST_META, ITEM_TASK_IDS, PULL_REQUEST_REFS, SET_HOST_META, TASK_EVENTS,
    TASK_MODELS, TASK_PULL_REQUESTS, UPSERT_GENERATION_PR,
};

/// The import fact holding when the first sync ran.
const SINCE: &str = "generation_since_ms";

/// A pull request that names an item.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PullRequest {
    pub repo: String,
    pub number: u64,
    pub work: i64,
    /// The latest message of the item's tasks or response of the host charged to it.
    pub active_ms: Option<u64>,
}

impl PullRequest {
    /// Its repository and number.
    pub fn id(&self) -> PullRequestId {
        PullRequestId {
            repo: self.repo.clone(),
            number: self.number,
        }
    }
}

/// How the last write of one pull request's section went: its `generation_pr` row.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LastWrite {
    pub repo: String,
    pub number: u64,
    pub work: i64,
    /// `OPEN`, `MERGED` or `CLOSED`, as `gh` gave it at the last write.
    pub state: Option<String>,
    /// The section last written, and when; `None` until a write succeeds.
    pub section: Option<String>,
    pub written_at_ms: Option<u64>,
    /// Why the last attempt failed; `None` after a success.
    pub last_error: Option<String>,
    /// When the last attempt was, success or failure.
    pub attempted_at_ms: Option<u64>,
}

/// Every pull request that names an item, by repository and in the order they were learned.
///
/// # Errors
///
/// Fails when the query fails or a row is not one.
#[instrument(level = "debug", skip(conn))]
pub async fn pull_requests(conn: &Connection) -> Result<Vec<PullRequest>> {
    let mut rows = conn
        .query(PULL_REQUEST_REFS, ())
        .await
        .context("Unable to read the pull requests of the items")?;
    let mut found = Vec::new();
    while let Some(row) = rows
        .next()
        .await
        .context("Unable to read the pull requests of the items")?
    {
        let mut cells = Cells::new(&row);
        let repo = cells.text("repo")?;
        let reference = cells.text("ref")?;
        let work = cells.id("work")?;
        let task_ms = cells.optional_count("task_at_ms")?;
        let host_ms = cells.optional_count("host_at_ms")?;
        // A ref of kind pull_request is always a number; another is not one to write.
        let Ok(number) = reference.parse() else {
            continue;
        };
        found.push(PullRequest {
            repo,
            number,
            work,
            active_ms: task_ms.max(host_ms),
        });
    }
    Ok(found)
}

/// How each pull request's last write went, by repository and number.
///
/// # Errors
///
/// Fails when the query fails or a row is not one.
#[instrument(level = "debug", skip(conn))]
pub async fn written(conn: &Connection) -> Result<HashMap<(String, u64), LastWrite>> {
    let mut rows = conn
        .query(GENERATION_PRS, ())
        .await
        .context("Unable to read the Generation sections written")?;
    let mut found = HashMap::new();
    while let Some(row) = rows
        .next()
        .await
        .context("Unable to read the Generation sections written")?
    {
        let mut cells = Cells::new(&row);
        let written = LastWrite {
            repo: cells.text("repo")?,
            number: cells.count("number")?,
            work: cells.id("work")?,
            state: cells.optional_text("state")?,
            section: cells.optional_text("section")?,
            written_at_ms: cells.optional_count("written_at_ms")?,
            last_error: cells.optional_text("last_error")?,
            attempted_at_ms: cells.optional_count("attempted_at_ms")?,
        };
        found.insert((written.repo.clone(), written.number), written);
    }
    Ok(found)
}

/// Keep how the write of one pull request's section went.
///
/// # Errors
///
/// Fails when the statement fails.
#[instrument(level = "debug", skip_all, fields(repo = %written.repo, number = written.number))]
pub async fn save(conn: &Connection, written: &LastWrite) -> Result<()> {
    conn.execute(
        UPSERT_GENERATION_PR,
        vec![
            Value::Text(written.repo.clone()),
            integer(written.number),
            Value::Integer(written.work),
            optional_text(written.state.as_deref()),
            optional_text(written.section.as_deref()),
            optional_integer(written.written_at_ms),
            optional_text(written.last_error.as_deref()),
            optional_integer(written.attempted_at_ms),
        ],
    )
    .await
    .with_context(|| {
        format!(
            "Unable to keep the Generation write of {}#{}",
            written.repo, written.number
        )
    })?;
    Ok(())
}

/// When the first sync ran, set to `now_ms` by the first.
///
/// # Errors
///
/// Fails when a statement fails.
#[instrument(level = "debug", skip(conn))]
pub async fn since(conn: &Connection, now_ms: u64) -> Result<u64> {
    conn.execute(
        SET_HOST_META,
        vec![Value::Text(SINCE.to_owned()), integer(now_ms)],
    )
    .await
    .context("Unable to record when the Generation sync started")?;
    let mut rows = conn
        .query(HOST_META, vec![Value::Text(SINCE.to_owned())])
        .await
        .context("Unable to read when the Generation sync started")?;
    let row = rows
        .next()
        .await
        .context("Unable to read when the Generation sync started")?
        .context("When the Generation sync started was not recorded")?;
    Cells::new(&row).count("value")
}

/// The rows of `pull_request`'s section, from the record, with `placements` saying which pull
/// request counts what: each task the host sent that has usage and is counted here (its own,
/// then those brought forward, each in the order they were first recorded), the host's usage
/// and time counted here on each model with how many of its responses have no price, a pointer
/// for each task of this pull request counted on another, and every task the record has in an
/// item.
///
/// # Errors
///
/// Fails when a query fails or a row is not one.
#[instrument(level = "debug", skip(conn, placements), fields(repo = %pull_request.repo, number = pull_request.number))]
pub async fn rows(
    conn: &Connection,
    pull_request: &PullRequest,
    placements: &Placements,
) -> Result<Rows> {
    let item = pull_request.work;
    let here = pull_request.id();
    let waits = task_waits(conn).await?;
    let models = task_models(conn, item).await?;
    let mut tasks = Vec::new();
    let mut counted = Vec::new();
    // The pull requests of the item that count work: a task's or a response of the host's.
    let mut counting: BTreeSet<PullRequestId> = BTreeSet::new();
    for row in read::usage(conn, HOST, None, None, false, Some(item)).await? {
        let Some((_, placement)) = placements.task(&row.task) else {
            continue;
        };
        counting.extend(placement.pull_request.iter().cloned());
        if placement.pull_request.as_ref() != Some(&here) {
            if let Some(home) = &placement.pull_request
                && placement.elsewhere.contains(&here)
            {
                counted.push(Counted {
                    task: row.task,
                    agent: row.agent,
                    repo: home.repo.clone(),
                    number: home.number,
                });
            }
            continue;
        }
        let used = match models.get(&row.task) {
            Some(used) => used.clone(),
            None => row.model.iter().cloned().collect(),
        };
        tasks.push(Row {
            models: used,
            usage: row.usage.unwrap_or_default(),
            time: Time {
                work: row.time.work,
                testing: row.time.testing,
                waiting: Some(waits.get(&row.task).copied().unwrap_or_default()),
            },
            task: row.task,
            agent: row.agent,
            brought_forward: placement.brought_forward,
        });
    }
    // Its own tasks first, then those brought forward to it.
    tasks.sort_by_key(|row| row.brought_forward);
    let focus = focus(conn).await?;
    let mut by_model: BTreeMap<String, (TaskUsage, u64)> = BTreeMap::new();
    for response in host_rows(conn, None, Some(item)).await? {
        let charged = charged_pull_request(
            placements,
            &focus,
            response.work,
            &response.session,
            response.charged_ms,
            response.pinned.as_ref(),
        );
        counting.extend(charged.iter().cloned());
        if charged.as_ref() != Some(&here) {
            continue;
        }
        let (usage, unpriced) = by_model.entry(response.model.clone()).or_default();
        add_host(usage, &response);
        if response.cost_usd.is_none() {
            *unpriced += 1;
        }
    }
    let times = host_model_times(conn, placements, &here).await?;
    let host = by_model
        .into_iter()
        .map(|(model, (usage, unpriced))| HostRow {
            time: times.get(&model).copied().unwrap_or_default().into(),
            model,
            usage,
            unpriced: (unpriced > 0).then_some(unpriced),
        })
        .collect::<Vec<HostRow>>();
    // A pull request no task named (added by `link`) points to those that count the item's work.
    let homes = if tasks.is_empty() && host.is_empty() && counted.is_empty() {
        counting.into_iter().collect()
    } else {
        Vec::new()
    };
    Ok(Rows {
        tasks,
        host,
        counted,
        homes,
        known: placements
            .items
            .values()
            .flat_map(|plan| plan.tasks.iter().cloned())
            .collect::<BTreeSet<String>>(),
    })
}

/// Which pull request counts each task of every item and each response of the host, from the
/// record, with `home_repo` the host's repository (`generation::placement`): the tasks of each
/// item in the order they were first recorded, the pull requests each named in its own item,
/// their events, and when each merged or closed pull request had its final write.
///
/// # Errors
///
/// Fails when a query fails or a row is not one.
#[instrument(level = "debug", skip(conn))]
pub async fn placements(conn: &Connection, home_repo: Option<&str>) -> Result<Placements> {
    let mut items: HashMap<i64, ItemPlan> = HashMap::new();
    let mut index: HashMap<String, (i64, usize)> = HashMap::new();
    let mut rows = conn
        .query(ITEM_TASK_IDS, ())
        .await
        .context("Unable to read the tasks of the items")?;
    while let Some(row) = rows
        .next()
        .await
        .context("Unable to read the tasks of the items")?
    {
        let mut cells = Cells::new(&row);
        let item = cells.id("work")?;
        let task = cells.text("task")?;
        let plan = items.entry(item).or_default();
        index.insert(task.clone(), (item, plan.tasks.len()));
        plan.tasks.push(task);
    }
    drop(rows);
    let mut rows = conn
        .query(TASK_EVENTS, ())
        .await
        .context("Unable to read the events of the tasks")?;
    while let Some(row) = rows
        .next()
        .await
        .context("Unable to read the events of the tasks")?
    {
        let mut cells = Cells::new(&row);
        let task = cells.text("task")?;
        let at_ms = cells.count("at_ms")?;
        if let Some((item, at)) = index.get(&task)
            && let Some(plan) = items.get_mut(item)
        {
            plan.events.push((at_ms, *at));
        }
    }
    drop(rows);
    let mut named = task_pull_requests(conn).await?;
    for plan in items.values_mut() {
        plan.events.sort_by_key(|(at_ms, _)| *at_ms);
        let of_tasks: Vec<Vec<Named>> = plan
            .tasks
            .iter()
            .map(|task| named.remove(task).unwrap_or_default())
            .collect();
        plan.placements = place(&of_tasks, home_repo);
    }
    Ok(Placements {
        items,
        frozen: final_writes(conn).await?,
    })
}

/// The pull requests each task named in its own item, by task.
async fn task_pull_requests(conn: &Connection) -> Result<HashMap<String, Vec<Named>>> {
    let mut rows = conn
        .query(TASK_PULL_REQUESTS, ())
        .await
        .context("Unable to read the pull requests of the tasks")?;
    let mut found: HashMap<String, Vec<Named>> = HashMap::new();
    while let Some(row) = rows
        .next()
        .await
        .context("Unable to read the pull requests of the tasks")?
    {
        let mut cells = Cells::new(&row);
        let task = cells.text("task")?;
        let repo = cells.text("repo")?;
        let reference = cells.text("ref")?;
        let source = cells.text("source")?;
        let seq = cells.count("seq")?;
        // A ref of kind pull_request is always a number; another is not one to count in.
        let Ok(number) = reference.parse() else {
            continue;
        };
        found.entry(task).or_default().push(Named {
            pull_request: PullRequestId { repo, number },
            source,
            seq,
        });
    }
    Ok(found)
}

/// When each merged or closed pull request had its final write.
async fn final_writes(conn: &Connection) -> Result<HashMap<PullRequestId, u64>> {
    Ok(written(conn)
        .await?
        .into_values()
        .filter(|last| is_frozen(last.state.as_deref()))
        .filter_map(|last| {
            last.written_at_ms.map(|final_ms| {
                (
                    PullRequestId {
                        repo: last.repo,
                        number: last.number,
                    },
                    final_ms,
                )
            })
        })
        .collect())
}

/// Every model each task of `item` used in its runs, sorted.
async fn task_models(conn: &Connection, item: i64) -> Result<HashMap<String, Vec<String>>> {
    let mut rows = conn
        .query(TASK_MODELS, vec![Value::Integer(item)])
        .await
        .context("Unable to read the models of the tasks")?;
    let mut found: HashMap<String, Vec<String>> = HashMap::new();
    while let Some(row) = rows
        .next()
        .await
        .context("Unable to read the models of the tasks")?
    {
        let mut cells = Cells::new(&row);
        let task = cells.text("task")?;
        let model = cells.text("model")?;
        found.entry(task).or_default().push(model);
    }
    Ok(found)
}
