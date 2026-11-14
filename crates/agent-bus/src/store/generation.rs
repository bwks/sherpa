//! What the Generation sync reads and keeps in the record: the pull requests that name an item,
//! the rows of an item's section, and how each pull request's last write went
//! (`generation_pr`).

use std::collections::{BTreeMap, BTreeSet, HashMap};

use anyhow::{Context, Result};
use tracing::instrument;
use turso::{Connection, Value};

use crate::generation::section::{HostRow, Row, Rows};
use crate::message::TaskUsage;
use crate::report::usage::{HOST, Time, add_host};
use crate::store::cells::{Cells, integer, optional_integer, optional_text};
use crate::store::host::{host_model_times, host_rows, task_waits};
use crate::store::read;
use crate::store::sql::{
    GENERATION_PRS, HOST_META, ITEM_TASK_IDS, PULL_REQUEST_REFS, SET_HOST_META, TASK_MODELS,
    UPSERT_GENERATION_PR,
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

/// The rows of `item`'s section, from the record: each task the host sent that has usage, in
/// the order they were first recorded, the host's usage and time on each model with how many
/// of its responses have no price, and every task the record has in an item.
///
/// # Errors
///
/// Fails when a query fails or a row is not one.
#[instrument(level = "debug", skip(conn))]
pub async fn rows(conn: &Connection, item: i64) -> Result<Rows> {
    let waits = task_waits(conn).await?;
    let models = task_models(conn, item).await?;
    let mut tasks = Vec::new();
    for row in read::usage(conn, HOST, None, None, false, Some(item)).await? {
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
        });
    }
    let mut by_model: BTreeMap<String, (TaskUsage, u64)> = BTreeMap::new();
    for response in host_rows(conn, None, Some(item)).await? {
        let (usage, unpriced) = by_model.entry(response.model.clone()).or_default();
        add_host(usage, &response);
        if response.cost_usd.is_none() {
            *unpriced += 1;
        }
    }
    let times = host_model_times(conn, item).await?;
    let host = by_model
        .into_iter()
        .map(|(model, (usage, unpriced))| HostRow {
            time: times.get(&model).copied().unwrap_or_default().into(),
            model,
            usage,
            unpriced: (unpriced > 0).then_some(unpriced),
        })
        .collect();
    Ok(Rows {
        tasks,
        host,
        known: known_tasks(conn).await?,
    })
}

/// Every task the record has in a work item.
async fn known_tasks(conn: &Connection) -> Result<BTreeSet<String>> {
    let mut rows = conn
        .query(ITEM_TASK_IDS, ())
        .await
        .context("Unable to read the tasks of the items")?;
    let mut found = BTreeSet::new();
    while let Some(row) = rows
        .next()
        .await
        .context("Unable to read the tasks of the items")?
    {
        found.insert(Cells::new(&row).text("task")?);
    }
    Ok(found)
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
