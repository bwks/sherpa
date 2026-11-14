//! What the work commands read from the record: an item with its refs, tasks and conflicts, the
//! list of items, the open tasks, and each run's models for `usage --by model`.

use std::collections::HashMap;

use anyhow::{Context, Result};
use serde::Serialize;
use tracing::instrument;
use turso::{Connection, Value};

use crate::store::cells::Cells;
use crate::store::facts::{Ref, display_name};
use crate::store::sql::{
    ITEM_CONFLICTS, ITEM_REFS, ITEM_TASKS, MERGED_ITEMS, OPEN_TASKS, RUNS, WORK_ITEMS,
};
use crate::store::task::TaskState;

/// One item, as `work show` prints it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Item {
    pub id: i64,
    /// Its lowest issue, else its lowest pull request, else its branch.
    pub name: String,
    pub refs: Vec<ItemRef>,
    pub tasks: Vec<ItemTask>,
    /// The refs its tasks named that name another item, and its refs that other items' tasks
    /// named.
    pub conflicts: Vec<Conflict>,
    /// The items `link` merged into it.
    pub merged: Vec<i64>,
}

/// A name of an item.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ItemRef {
    #[serde(flatten)]
    pub reference: Ref,
    /// How it was learned: `brief`, `guest`, `parsed` or `manual`.
    pub source: String,
    pub at_ms: u64,
}

/// A task of an item.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ItemTask {
    pub task: String,
    pub agent: String,
    pub state: TaskState,
    /// The model it ran on, else the one it asked for.
    pub model: Option<String>,
    pub prompt: Option<String>,
    pub last_at_ms: u64,
    pub runs: Option<u64>,
    pub cost_usd: Option<f64>,
    /// How it was put in the item.
    pub source: String,
}

/// A ref a task named that names another item than the task's.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Conflict {
    pub task: String,
    #[serde(flatten)]
    pub reference: Ref,
    /// How the task named it.
    pub source: String,
    /// The task's item, and its name.
    pub task_item: i64,
    pub task_item_name: String,
    /// The item the ref names, and its name.
    pub ref_item: i64,
    pub ref_item_name: String,
}

/// One line of `work show` without an item.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ItemSummary {
    pub id: i64,
    pub name: String,
    pub refs: Vec<Ref>,
    pub tasks: u64,
    /// What its tasks cost, those with usage.
    pub cost_usd: f64,
    /// Its tasks' latest message; its creation without tasks.
    pub last_at_ms: u64,
}

/// A task still open: sent, running or waiting on a question.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct OpenTask {
    pub task: String,
    pub agent: String,
    pub state: TaskState,
    /// When it was sent, else when it was first seen.
    pub since_ms: u64,
    pub item: Option<i64>,
    pub item_name: Option<String>,
    /// The question it waits on.
    pub question: Option<String>,
}

/// What one model of one run used, or a whole run that named no model.
#[derive(Debug, Clone, PartialEq)]
pub struct RunPart {
    /// The run's stream sequence.
    pub seq: u64,
    pub task: String,
    /// The model; `None` for a run that named none.
    pub model: Option<String>,
    pub input: u64,
    pub output: u64,
    pub cache_creation: u64,
    pub cache_read: u64,
    pub cost_usd: f64,
}

/// Every item's refs, by item.
type RefsByItem = HashMap<i64, Vec<ItemRef>>;

/// The item `id`, with its refs, tasks, conflicts and the items merged into it.
///
/// # Errors
///
/// Fails when a query fails or a row is not one.
#[instrument(level = "debug", skip(conn))]
pub async fn item(conn: &Connection, id: i64) -> Result<Item> {
    let refs = refs_by_item(conn).await?;
    let tasks = item_tasks(conn, Some(id)).await?;
    let mut rows = conn
        .query(ITEM_CONFLICTS, vec![Value::Integer(id)])
        .await
        .context("Unable to read the conflicts")?;
    let mut conflicts = Vec::new();
    while let Some(row) = rows.next().await.context("Unable to read the conflicts")? {
        let mut cells = Cells::new(&row);
        let task = cells.text("task")?;
        let reference = cells.reference()?;
        let source = cells.text("source")?;
        let task_item = cells.id("task_item")?;
        let ref_item = cells.id("ref_item")?;
        conflicts.push(Conflict {
            task,
            reference,
            source,
            task_item,
            task_item_name: name_of(&refs, task_item),
            ref_item,
            ref_item_name: name_of(&refs, ref_item),
        });
    }
    drop(rows);
    let mut rows = conn
        .query(MERGED_ITEMS, vec![Value::Integer(id)])
        .await
        .context("Unable to read the merged items")?;
    let mut merged = Vec::new();
    while let Some(row) = rows
        .next()
        .await
        .context("Unable to read the merged items")?
    {
        merged.push(Cells::new(&row).id("id")?);
    }
    Ok(Item {
        id,
        name: name_of(&refs, id),
        refs: refs.get(&id).cloned().unwrap_or_default(),
        tasks: tasks.into_iter().map(|(_, task)| task).collect(),
        conflicts,
        merged,
    })
}

/// Every item not merged into another, oldest first.
///
/// # Errors
///
/// Fails when a query fails or a row is not one.
#[instrument(level = "debug", skip(conn))]
pub async fn items(conn: &Connection) -> Result<Vec<ItemSummary>> {
    let refs = refs_by_item(conn).await?;
    let tasks = item_tasks(conn, None).await?;
    let mut rows = conn
        .query(WORK_ITEMS, ())
        .await
        .context("Unable to read the items")?;
    let mut items = Vec::new();
    while let Some(row) = rows.next().await.context("Unable to read the items")? {
        let mut cells = Cells::new(&row);
        let id = cells.id("id")?;
        let created_at_ms = cells.count("created_at_ms")?;
        let of_item = tasks.iter().filter(|(item, _)| *item == id);
        items.push(ItemSummary {
            id,
            name: name_of(&refs, id),
            refs: refs.get(&id).map_or_else(Vec::new, |named| {
                named.iter().map(|named| named.reference.clone()).collect()
            }),
            tasks: u64::try_from(of_item.clone().count()).unwrap_or(u64::MAX),
            // Folded from 0.0: an empty `sum` of floats is -0.0, shown as `$-0.0000`.
            cost_usd: of_item
                .clone()
                .filter_map(|(_, task)| task.cost_usd)
                .fold(0.0, |sum, cost| sum + cost),
            last_at_ms: of_item
                .map(|(_, task)| task.last_at_ms)
                .max()
                .unwrap_or(created_at_ms),
        });
    }
    Ok(items)
}

/// The open tasks, oldest first, with their item.
///
/// # Errors
///
/// Fails when a query fails or a row is not one.
#[instrument(level = "debug", skip(conn))]
pub async fn open_tasks(conn: &Connection) -> Result<Vec<OpenTask>> {
    let refs = refs_by_item(conn).await?;
    let mut rows = conn
        .query(OPEN_TASKS, ())
        .await
        .context("Unable to read the open tasks")?;
    let mut open = Vec::new();
    while let Some(row) = rows.next().await.context("Unable to read the open tasks")? {
        let mut cells = Cells::new(&row);
        let task = cells.text("id")?;
        let agent = cells.text("agent")?;
        let state = TaskState::parse(&cells.text("state")?)?;
        let sent_at_ms = cells.optional_count("sent_at_ms")?;
        let last_at_ms = cells.count("last_at_ms")?;
        let result = cells.optional_text("result")?;
        let item = cells.optional_id("work")?;
        open.push(OpenTask {
            task,
            agent,
            state,
            since_ms: sent_at_ms.unwrap_or(last_at_ms),
            item,
            item_name: item.map(|item| name_of(&refs, item)),
            question: result.filter(|_| state == TaskState::Question),
        });
    }
    Ok(open)
}

/// Every run's parts: one per model it named, or the whole run when it named none.
///
/// # Errors
///
/// Fails when the query fails or a row is not one.
#[instrument(level = "debug", skip(conn))]
pub async fn run_parts(conn: &Connection) -> Result<Vec<RunPart>> {
    let mut rows = conn
        .query(RUNS, ())
        .await
        .context("Unable to read the runs")?;
    let mut parts = Vec::new();
    while let Some(row) = rows.next().await.context("Unable to read the runs")? {
        let mut cells = Cells::new(&row);
        let seq = cells.count("seq")?;
        let task = cells.text("task")?;
        let run = RunPart {
            seq,
            task: task.clone(),
            model: cells.optional_text("model")?,
            input: cells.count("input")?,
            output: cells.count("output")?,
            cache_creation: cells.count("cache_creation")?,
            cache_read: cells.count("cache_read")?,
            cost_usd: cells.optional_real("cost_usd")?.unwrap_or_default(),
        };
        let part = match cells.optional_text("run_model.model")? {
            Some(model) => RunPart {
                seq,
                task,
                model: Some(model),
                input: cells.count("run_model.input")?,
                output: cells.count("run_model.output")?,
                cache_creation: cells.count("run_model.cache_creation")?,
                cache_read: cells.count("run_model.cache_read")?,
                cost_usd: cells
                    .optional_real("run_model.cost_usd")?
                    .unwrap_or_default(),
            },
            None => run,
        };
        parts.push(part);
    }
    Ok(parts)
}

/// Every item's refs.
async fn refs_by_item(conn: &Connection) -> Result<RefsByItem> {
    let mut rows = conn
        .query(ITEM_REFS, vec![Value::Null])
        .await
        .context("Unable to read the refs")?;
    let mut refs = RefsByItem::new();
    while let Some(row) = rows.next().await.context("Unable to read the refs")? {
        let mut cells = Cells::new(&row);
        let item = cells.id("work")?;
        let reference = cells.reference()?;
        let source = cells.text("source")?;
        let at_ms = cells.count("at_ms")?;
        refs.entry(item).or_default().push(ItemRef {
            reference,
            source,
            at_ms,
        });
    }
    Ok(refs)
}

/// The tasks of `item`, or of every item, with their item.
async fn item_tasks(conn: &Connection, item: Option<i64>) -> Result<Vec<(i64, ItemTask)>> {
    let mut rows = conn
        .query(ITEM_TASKS, vec![item.map_or(Value::Null, Value::Integer)])
        .await
        .context("Unable to read the tasks of the items")?;
    let mut tasks = Vec::new();
    while let Some(row) = rows
        .next()
        .await
        .context("Unable to read the tasks of the items")?
    {
        let mut cells = Cells::new(&row);
        let item = cells.id("work")?;
        let source = cells.text("source")?;
        let task = cells.text("id")?;
        let agent = cells.text("agent")?;
        let state = TaskState::parse(&cells.text("state")?)?;
        let model = cells.optional_text("model")?;
        let requested_model = cells.optional_text("requested_model")?;
        let prompt = cells.optional_text("prompt")?;
        let last_at_ms = cells.count("last_at_ms")?;
        let runs = cells.optional_count("runs")?;
        let cost_usd = cells.optional_real("cost_usd")?;
        tasks.push((
            item,
            ItemTask {
                task,
                agent,
                state,
                model: model.or(requested_model),
                prompt,
                last_at_ms,
                runs,
                cost_usd,
                source,
            },
        ));
    }
    Ok(tasks)
}

/// The name of `item`: from its refs, else `item <id>`.
fn name_of(refs: &RefsByItem, item: i64) -> String {
    refs.get(&item)
        .and_then(|named| {
            let named: Vec<Ref> = named.iter().map(|named| named.reference.clone()).collect();
            display_name(&named)
        })
        .unwrap_or_else(|| format!("item {item}"))
}
