//! The host's own usage and time in the record: what `host import` writes (responses, prompts
//! and tool calls, the cursors and the install time), the host's active item (`host_focus`),
//! attribution, `work reassign`, and what the reports read.
//!
//! Attribution is a pure function of the focus rows and each response's time (`active`), so it
//! is worked out again over every unpinned response after each import and each focus change: a
//! focus row recorded late moves the responses in its interval.

use std::collections::{BTreeMap, HashMap};

use anyhow::{Context, Result};
use serde::Serialize;
use tracing::instrument;
use turso::{Connection, Value};

use crate::host::transcript::Response;
use crate::price::{self, Tokens};
use crate::store::cells::{Cells, boolean, integer, optional_integer, optional_text};
use crate::store::sql::{
    ATTRIBUTE_HOST_USAGE, END_HOST_SPAN, FIND_HOST_USAGE, FIND_TRANSCRIPT, HOST_FOCUS, HOST_META,
    HOST_RESPONSES, HOST_SPANS, HOST_TOTALS, HOST_TRANSCRIPTS, HOST_USAGE_ROWS, INSERT_HOST_FOCUS,
    INSERT_HOST_SPAN, PRICE_HOST_USAGE, REASSIGN_HOST_USAGE, RESCAN_TRANSCRIPTS, SET_HOST_META,
    SUBAGENT_STARTS, TASK_WAITS, UNPINNED_HOST_USAGE, UNPRICED_HOST_USAGE, UPSERT_HOST_USAGE,
    UPSERT_TRANSCRIPT,
};
use crate::timing::{
    Call, Class, Prompt, Response as TimedResponse, Timeline, Times, segments, waiting_on_answers,
};

/// The import fact holding the time of the first import.
const INSTALLED_AT: &str = "installed_at_ms";
/// The kind of a span that is the user's prompt, and of one Claude Code wrote.
const USER_PROMPT: &str = "prompt";
const NOTICE: &str = "notice";

/// Where reading a transcript stopped.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Cursor {
    pub inode: u64,
    pub offset: u64,
    pub started_at_ms: Option<u64>,
    pub skipped: u64,
}

/// A transcript file, and where reading it stopped now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Transcript<'a> {
    pub path: &'a str,
    pub project: &'a str,
    pub session: &'a str,
    pub agent_id: Option<&'a str>,
    pub cursor: Cursor,
    pub imported_at_ms: u64,
}

/// A change of the host's active item.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Focus {
    pub from_ms: u64,
    /// The Claude Code session it is for; `None` is every session.
    pub session: Option<String>,
    /// The item; `None` is none.
    pub work: Option<i64>,
    pub source: String,
}

/// Which responses `work reassign` moves.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reassign {
    /// The item they are charged to; `None` is unattributed.
    pub from: Option<i64>,
    pub to: i64,
    pub since_ms: Option<u64>,
    pub until_ms: Option<u64>,
    pub session: Option<String>,
}

/// One response of the host, for the reports.
#[derive(Debug, Clone, PartialEq)]
pub struct HostRow {
    pub work: Option<i64>,
    pub model: String,
    pub at_ms: u64,
    pub tokens: Tokens,
    /// `None` when it is not priced.
    pub cost_usd: Option<f64>,
}

/// A transcript read, for `host status`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TranscriptRow {
    pub path: String,
    pub session: String,
    pub agent_id: Option<String>,
    pub offset: u64,
    pub imported_at_ms: u64,
    pub skipped: u64,
}

/// The host's totals, for `host status`.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Totals {
    pub responses: u64,
    pub cost_usd: f64,
    pub unpriced: u64,
    pub first_at_ms: Option<u64>,
    pub last_at_ms: Option<u64>,
    pub installed_at_ms: Option<u64>,
}

/// The time of the first import, set to `now_ms` by the first.
///
/// # Errors
///
/// Fails when a statement fails.
#[instrument(level = "debug", skip(conn))]
pub async fn installed_at(conn: &Connection, now_ms: u64) -> Result<u64> {
    conn.execute(
        SET_HOST_META,
        vec![Value::Text(INSTALLED_AT.to_owned()), integer(now_ms)],
    )
    .await
    .context("Unable to record the install time")?;
    installed(conn)
        .await?
        .context("The install time was not recorded")
}

/// The time of the first import, if there was one.
async fn installed(conn: &Connection) -> Result<Option<u64>> {
    let mut rows = conn
        .query(HOST_META, vec![Value::Text(INSTALLED_AT.to_owned())])
        .await
        .context("Unable to read the install time")?;
    match rows
        .next()
        .await
        .context("Unable to read the install time")?
    {
        Some(row) => Ok(Some(Cells::new(&row).count("value")?)),
        None => Ok(None),
    }
}

/// Where reading the transcript at `path` stopped, if it was read.
///
/// # Errors
///
/// Fails when the query fails.
pub async fn cursor(conn: &Connection, path: &str) -> Result<Option<Cursor>> {
    let mut rows = conn
        .query(FIND_TRANSCRIPT, vec![Value::Text(path.to_owned())])
        .await
        .context("Unable to read a transcript's cursor")?;
    let Some(row) = rows
        .next()
        .await
        .context("Unable to read a transcript's cursor")?
    else {
        return Ok(None);
    };
    let mut cells = Cells::new(&row);
    Ok(Some(Cursor {
        inode: cells.count("inode")?,
        offset: cells.count("offset")?,
        started_at_ms: cells.optional_count("started_at_ms")?,
        skipped: cells.count("skipped")?,
    }))
}

/// Keep where reading `transcript` stopped.
///
/// # Errors
///
/// Fails when the statement fails.
pub async fn save_cursor(conn: &Connection, transcript: &Transcript<'_>) -> Result<()> {
    let cursor = &transcript.cursor;
    conn.execute(
        UPSERT_TRANSCRIPT,
        vec![
            Value::Text(transcript.path.to_owned()),
            Value::Text(transcript.project.to_owned()),
            Value::Text(transcript.session.to_owned()),
            optional_text(transcript.agent_id),
            integer(cursor.inode),
            integer(cursor.offset),
            optional_integer(cursor.started_at_ms),
            integer(transcript.imported_at_ms),
            integer(cursor.skipped),
        ],
    )
    .await
    .context("Unable to keep a transcript's cursor")?;
    Ok(())
}

/// Read every transcript of `project` from the start again.
///
/// # Errors
///
/// Fails when the statement fails.
#[instrument(level = "debug", skip(conn))]
pub async fn rescan(conn: &Connection, project: &str) -> Result<()> {
    conn.execute(RESCAN_TRANSCRIPTS, vec![Value::Text(project.to_owned())])
        .await
        .context("Unable to reset the cursors")?;
    Ok(())
}

/// Record `response`, or merge it into the row of its `message.id`: the earliest time and the
/// first session stay, the latest time and the largest of each count are taken, so a partial
/// line never lowers a figure. Returns whether the response is new to the record.
///
/// # Errors
///
/// Fails when a statement fails.
pub async fn keep_response(conn: &Connection, response: &Response) -> Result<bool> {
    let mut rows = conn
        .query(
            FIND_HOST_USAGE,
            vec![Value::Text(response.message_id.clone())],
        )
        .await
        .context("Unable to read a response")?;
    let found = match rows.next().await.context("Unable to read a response")? {
        Some(row) => Some(stored_response(&mut Cells::new(&row))?),
        None => None,
    };
    drop(rows);
    let incoming = Stored {
        session: response.session.clone(),
        agent_id: response.agent_id.clone(),
        at_ms: response.at_ms,
        last_at_ms: response.at_ms,
        model: response.model.clone(),
        speed: response.speed.clone(),
        tokens: response.tokens,
        cost_usd: price::cost(&response.model, &response.speed, &response.tokens),
    };
    let new = found.is_none();
    let merged = match found {
        Some(stored) => {
            let merged = merge(&stored, &incoming);
            if merged == stored {
                return Ok(false);
            }
            merged
        }
        None => incoming,
    };
    let tokens = merged.tokens;
    conn.execute(
        UPSERT_HOST_USAGE,
        vec![
            Value::Text(response.message_id.clone()),
            Value::Text(merged.session),
            optional_text(merged.agent_id.as_deref()),
            integer(merged.at_ms),
            integer(merged.last_at_ms),
            Value::Text(merged.model),
            Value::Text(merged.speed),
            integer(tokens.input),
            integer(tokens.output),
            integer(tokens.cache_read),
            integer(tokens.cache_write_5m),
            integer(tokens.cache_write_1h),
            merged.cost_usd.map_or(Value::Null, Value::Real),
        ],
    )
    .await
    .context("Unable to record a response")?;
    Ok(new)
}

/// A response as the record holds it.
#[derive(Debug, Clone, PartialEq)]
struct Stored {
    session: String,
    agent_id: Option<String>,
    at_ms: u64,
    last_at_ms: u64,
    model: String,
    speed: String,
    tokens: Tokens,
    cost_usd: Option<f64>,
}

/// A stored response from the columns `find_host_usage.sql` selects.
fn stored_response(cells: &mut Cells<'_>) -> Result<Stored> {
    Ok(Stored {
        session: cells.text("session")?,
        agent_id: cells.optional_text("agent_id")?,
        at_ms: cells.count("at_ms")?,
        last_at_ms: cells.count("last_at_ms")?,
        model: cells.text("model")?,
        speed: cells.text("speed")?,
        tokens: tokens(cells)?,
        cost_usd: cells.optional_real("cost_usd")?,
    })
}

/// Five token counts, input to the one-hour cache write.
fn tokens(cells: &mut Cells<'_>) -> Result<Tokens> {
    Ok(Tokens {
        input: cells.count("input")?,
        output: cells.count("output")?,
        cache_read: cells.count("cache_read")?,
        cache_write_5m: cells.count("cache_write_5m")?,
        cache_write_1h: cells.count("cache_write_1h")?,
    })
}

/// `stored` with what `incoming` adds: its session and earliest time stay, the latest time and
/// the largest counts are taken. The cost is worked out again only when a count went up; a
/// priced row is otherwise never priced again.
fn merge(stored: &Stored, incoming: &Stored) -> Stored {
    let tokens = Tokens {
        input: stored.tokens.input.max(incoming.tokens.input),
        output: stored.tokens.output.max(incoming.tokens.output),
        cache_read: stored.tokens.cache_read.max(incoming.tokens.cache_read),
        cache_write_5m: stored
            .tokens
            .cache_write_5m
            .max(incoming.tokens.cache_write_5m),
        cache_write_1h: stored
            .tokens
            .cache_write_1h
            .max(incoming.tokens.cache_write_1h),
    };
    let cost_usd = if tokens == stored.tokens {
        stored.cost_usd
    } else {
        price::cost(&stored.model, &stored.speed, &tokens)
    };
    let (session, agent_id) = if incoming.at_ms < stored.at_ms {
        (incoming.session.clone(), incoming.agent_id.clone())
    } else {
        (stored.session.clone(), stored.agent_id.clone())
    };
    Stored {
        session,
        agent_id,
        at_ms: stored.at_ms.min(incoming.at_ms),
        last_at_ms: stored.last_at_ms.max(incoming.last_at_ms),
        model: stored.model.clone(),
        speed: stored.speed.clone(),
        tokens,
        cost_usd,
    }
}

/// Record a prompt, the user's when `from_user`, unless its line is recorded.
///
/// # Errors
///
/// Fails when the statement fails.
pub async fn keep_prompt(
    conn: &Connection,
    id: &str,
    session: &str,
    agent_id: Option<&str>,
    at_ms: u64,
    from_user: bool,
) -> Result<()> {
    let kind = if from_user { USER_PROMPT } else { NOTICE };
    keep_span(conn, id, session, agent_id, kind, at_ms, Some(at_ms)).await
}

/// Record the start of a tool call, of `class`, unless it is recorded.
///
/// # Errors
///
/// Fails when the statement fails.
pub async fn keep_call(
    conn: &Connection,
    id: &str,
    session: &str,
    agent_id: Option<&str>,
    at_ms: u64,
    class: Class,
) -> Result<()> {
    keep_span(conn, id, session, agent_id, class_name(class), at_ms, None).await
}

/// The kind a tool call of `class` is stored as.
fn class_name(class: Class) -> &'static str {
    match class {
        Class::Work => "work",
        Class::Testing => "testing",
        Class::Excluded => "excluded",
    }
}

/// Insert one span unless its ID is recorded.
async fn keep_span(
    conn: &Connection,
    id: &str,
    session: &str,
    agent_id: Option<&str>,
    kind: &str,
    start_ms: u64,
    end_ms: Option<u64>,
) -> Result<()> {
    conn.execute(
        INSERT_HOST_SPAN,
        vec![
            Value::Text(id.to_owned()),
            Value::Text(session.to_owned()),
            optional_text(agent_id),
            Value::Text(kind.to_owned()),
            integer(start_ms),
            optional_integer(end_ms),
        ],
    )
    .await
    .context("Unable to record a prompt or a tool call")?;
    Ok(())
}

/// The tool call `id` returned at `at_ms`.
///
/// # Errors
///
/// Fails when the statement fails.
pub async fn end_call(conn: &Connection, id: &str, at_ms: u64) -> Result<()> {
    conn.execute(
        END_HOST_SPAN,
        vec![Value::Text(id.to_owned()), integer(at_ms)],
    )
    .await
    .context("Unable to record a tool result")?;
    Ok(())
}

/// Price every response without a cost that has a rate now. Returns how many were priced.
///
/// # Errors
///
/// Fails when a statement fails.
#[instrument(level = "debug", skip(conn))]
pub async fn reprice(conn: &Connection) -> Result<u64> {
    let mut rows = conn
        .query(UNPRICED_HOST_USAGE, ())
        .await
        .context("Unable to read the responses without a cost")?;
    let mut priced = Vec::new();
    while let Some(row) = rows
        .next()
        .await
        .context("Unable to read the responses without a cost")?
    {
        let mut cells = Cells::new(&row);
        let id = cells.text("message_id")?;
        let model = cells.text("model")?;
        let speed = cells.text("speed")?;
        if let Some(cost) = price::cost(&model, &speed, &tokens(&mut cells)?) {
            priced.push((id, cost));
        }
    }
    drop(rows);
    let mut count = 0;
    for (id, cost) in priced {
        count += conn
            .execute(PRICE_HOST_USAGE, vec![Value::Text(id), Value::Real(cost)])
            .await
            .context("Unable to price a response")?;
    }
    Ok(count)
}

/// Make `work` (`None`: none) the host's active item from `from_ms` on, for `session` (`None`:
/// every session), because of `source` (`use`, `clear` or `send`, with the `task` it tagged).
///
/// # Errors
///
/// Fails when the statement fails.
#[instrument(level = "debug", skip(conn))]
pub async fn add_focus(
    conn: &Connection,
    from_ms: u64,
    session: Option<&str>,
    work: Option<i64>,
    source: &str,
    task: Option<&str>,
) -> Result<()> {
    conn.execute(
        INSERT_HOST_FOCUS,
        vec![
            integer(from_ms),
            optional_text(session),
            work.map_or(Value::Null, Value::Integer),
            Value::Text(source.to_owned()),
            optional_text(task),
        ],
    )
    .await
    .context("Unable to record the active item")?;
    Ok(())
}

/// Every change of the host's active item, oldest first.
///
/// # Errors
///
/// Fails when the query fails.
pub async fn focus(conn: &Connection) -> Result<Vec<Focus>> {
    let mut rows = conn
        .query(HOST_FOCUS, ())
        .await
        .context("Unable to read the active items")?;
    let mut found = Vec::new();
    while let Some(row) = rows
        .next()
        .await
        .context("Unable to read the active items")?
    {
        let mut cells = Cells::new(&row);
        found.push(Focus {
            from_ms: cells.count("from_ms")?,
            session: cells.optional_text("session")?,
            work: cells.optional_id("work")?,
            source: cells.text("source")?,
        });
    }
    Ok(found)
}

/// The item active at `at_ms` in `session`: that of the newest focus row at or before it, for
/// the session or for every session. `focus` is oldest first. `None` before any, or after a
/// clear.
pub fn active(focus: &[Focus], session: &str, at_ms: u64) -> Option<i64> {
    focus
        .iter()
        .rev()
        .find(|row| {
            row.from_ms <= at_ms
                && row
                    .session
                    .as_deref()
                    .is_none_or(|scoped| scoped == session)
        })
        .and_then(|row| row.work)
}

/// The latest focus row of each session that has one, and of every session (`None`), for `work
/// show`.
pub fn current_focus(focus: &[Focus]) -> BTreeMap<Option<String>, Focus> {
    let mut current = BTreeMap::new();
    for row in focus {
        current.insert(row.session.clone(), row.clone());
    }
    current
}

/// Charge every unpinned response to the item active when it was made: a subagent's to the
/// item active when the subagent started. Returns how many moved.
///
/// # Errors
///
/// Fails when a statement fails.
#[instrument(level = "debug", skip(conn))]
pub async fn attribute(conn: &Connection) -> Result<u64> {
    let focus = focus(conn).await?;
    let mut rows = conn
        .query(SUBAGENT_STARTS, ())
        .await
        .context("Unable to read when the subagents started")?;
    let mut starts = HashMap::new();
    while let Some(row) = rows
        .next()
        .await
        .context("Unable to read when the subagents started")?
    {
        let mut cells = Cells::new(&row);
        starts.insert(cells.text("agent_id")?, cells.count("started_at_ms")?);
    }
    drop(rows);
    let mut rows = conn
        .query(UNPINNED_HOST_USAGE, ())
        .await
        .context("Unable to read the responses")?;
    let mut changes = Vec::new();
    while let Some(row) = rows.next().await.context("Unable to read the responses")? {
        let mut cells = Cells::new(&row);
        let id = cells.text("message_id")?;
        let session = cells.text("session")?;
        let agent_id = cells.optional_text("agent_id")?;
        let at_ms = cells.count("at_ms")?;
        let work = cells.optional_id("work")?;
        let at_ms = agent_id
            .and_then(|agent| starts.get(&agent).copied())
            .unwrap_or(at_ms);
        let charged = active(&focus, &session, at_ms);
        if charged != work {
            changes.push((id, charged));
        }
    }
    drop(rows);
    let mut moved = 0;
    for (id, work) in changes {
        moved += conn
            .execute(
                ATTRIBUTE_HOST_USAGE,
                vec![Value::Text(id), work.map_or(Value::Null, Value::Integer)],
            )
            .await
            .context("Unable to charge a response to its item")?;
    }
    Ok(moved)
}

/// Move the responses `reassign` selects to its item, and pin them. Returns how many moved.
///
/// # Errors
///
/// Fails when the statement fails.
#[instrument(level = "debug", skip(conn))]
pub async fn reassign(conn: &Connection, reassign: &Reassign) -> Result<u64> {
    conn.execute(
        REASSIGN_HOST_USAGE,
        vec![
            Value::Integer(reassign.to),
            reassign.from.map_or(Value::Null, Value::Integer),
            boolean(reassign.from.is_none()),
            optional_integer(reassign.since_ms),
            optional_integer(reassign.until_ms),
            optional_text(reassign.session.as_deref()),
        ],
    )
    .await
    .context("Unable to reassign the responses")
}

/// The host's responses at or after `since_ms` when given, of `item` when given, oldest first.
///
/// # Errors
///
/// Fails when the query fails or a row is not one.
#[instrument(level = "debug", skip(conn))]
pub async fn host_rows(
    conn: &Connection,
    since_ms: Option<u64>,
    item: Option<i64>,
) -> Result<Vec<HostRow>> {
    let mut rows = conn
        .query(
            HOST_USAGE_ROWS,
            vec![
                optional_integer(since_ms),
                item.map_or(Value::Null, Value::Integer),
            ],
        )
        .await
        .context("Unable to read the host's usage")?;
    let mut found = Vec::new();
    while let Some(row) = rows
        .next()
        .await
        .context("Unable to read the host's usage")?
    {
        let mut cells = Cells::new(&row);
        found.push(HostRow {
            work: cells.optional_id("work")?,
            model: cells.text("model")?,
            at_ms: cells.count("at_ms")?,
            tokens: tokens(&mut cells)?,
            cost_usd: cells.optional_real("cost_usd")?,
        });
    }
    Ok(found)
}

/// The host's time by item (`None`: unattributed), from its main conversations: each session's
/// prompts, tool calls and responses cut into buckets (`timing::segments`), the segments
/// starting at or after `since_ms` when given.
///
/// # Errors
///
/// Fails when a query fails or a row is not one.
#[instrument(level = "debug", skip(conn))]
pub async fn host_times(
    conn: &Connection,
    since_ms: Option<u64>,
) -> Result<HashMap<Option<i64>, Times>> {
    let mut timelines: BTreeMap<String, Timeline> = BTreeMap::new();
    let mut rows = conn
        .query(HOST_SPANS, ())
        .await
        .context("Unable to read the host's prompts and tool calls")?;
    while let Some(row) = rows
        .next()
        .await
        .context("Unable to read the host's prompts and tool calls")?
    {
        let mut cells = Cells::new(&row);
        let session = cells.text("session")?;
        let kind = cells.text("kind")?;
        let start_ms = cells.count("start_ms")?;
        let end_ms = cells.optional_count("end_ms")?;
        let timeline = timelines.entry(session).or_default();
        let class = match kind.as_str() {
            USER_PROMPT | NOTICE => {
                timeline.prompts.push(Prompt {
                    at_ms: start_ms,
                    from_user: kind == USER_PROMPT,
                });
                continue;
            }
            "testing" => Class::Testing,
            "excluded" => Class::Excluded,
            _ => Class::Work,
        };
        timeline.calls.push(Call {
            start_ms,
            end_ms,
            class,
        });
    }
    drop(rows);
    let mut rows = conn
        .query(HOST_RESPONSES, ())
        .await
        .context("Unable to read the host's responses")?;
    while let Some(row) = rows
        .next()
        .await
        .context("Unable to read the host's responses")?
    {
        let mut cells = Cells::new(&row);
        let session = cells.text("session")?;
        let first_ms = cells.count("at_ms")?;
        let last_ms = cells.count("last_at_ms")?;
        let item = cells.optional_id("work")?;
        timelines
            .entry(session)
            .or_default()
            .responses
            .push(TimedResponse {
                first_ms,
                last_ms,
                item,
            });
    }
    drop(rows);
    let mut times: HashMap<Option<i64>, Times> = HashMap::new();
    for timeline in timelines.values() {
        for segment in segments(timeline) {
            if since_ms.is_some_and(|since| segment.start_ms < since) {
                continue;
            }
            times.entry(segment.item).or_default().add(&segment);
        }
    }
    Ok(times)
}

/// How long each task waited on the user: each question to its answer, summed.
///
/// # Errors
///
/// Fails when the query fails or a row is not one.
#[instrument(level = "debug", skip(conn))]
pub async fn task_waits(conn: &Connection) -> Result<HashMap<String, u64>> {
    let mut rows = conn
        .query(TASK_WAITS, ())
        .await
        .context("Unable to read the questions and answers")?;
    let mut by_task: BTreeMap<String, Vec<(bool, u64)>> = BTreeMap::new();
    while let Some(row) = rows
        .next()
        .await
        .context("Unable to read the questions and answers")?
    {
        let mut cells = Cells::new(&row);
        let task = cells.text("task")?;
        let question = cells.text("kind")? == "question";
        let at_ms = cells.count("at_ms")?;
        by_task.entry(task).or_default().push((question, at_ms));
    }
    Ok(by_task
        .into_iter()
        .map(|(task, messages)| (task, waiting_on_answers(&messages)))
        .collect())
}

/// Every transcript read, the latest first.
///
/// # Errors
///
/// Fails when the query fails or a row is not one.
pub async fn transcripts(conn: &Connection) -> Result<Vec<TranscriptRow>> {
    let mut rows = conn
        .query(HOST_TRANSCRIPTS, ())
        .await
        .context("Unable to read the transcripts")?;
    let mut found = Vec::new();
    while let Some(row) = rows
        .next()
        .await
        .context("Unable to read the transcripts")?
    {
        let mut cells = Cells::new(&row);
        found.push(TranscriptRow {
            path: cells.text("path")?,
            session: cells.text("session")?,
            agent_id: cells.optional_text("agent_id")?,
            offset: cells.count("offset")?,
            imported_at_ms: cells.count("imported_at_ms")?,
            skipped: cells.count("skipped")?,
        });
    }
    Ok(found)
}

/// The host's totals and the install time.
///
/// # Errors
///
/// Fails when a query fails or a row is not one.
pub async fn totals(conn: &Connection) -> Result<Totals> {
    let mut rows = conn
        .query(HOST_TOTALS, ())
        .await
        .context("Unable to read the host's totals")?;
    let row = rows
        .next()
        .await
        .context("Unable to read the host's totals")?
        .context("No totals")?;
    let mut cells = Cells::new(&row);
    let responses = cells.count("count")?;
    let cost_usd = cells.optional_real("cost")?.unwrap_or_default();
    let unpriced = cells.optional_count("unpriced")?.unwrap_or_default();
    let first_at_ms = cells.optional_count("first")?;
    let last_at_ms = cells.optional_count("last")?;
    drop(rows);
    Ok(Totals {
        responses,
        cost_usd,
        unpriced,
        first_at_ms,
        last_at_ms,
        installed_at_ms: installed(conn).await?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::store::database::Store;

    fn focus_row(from_ms: u64, session: Option<&str>, work: Option<i64>) -> Focus {
        Focus {
            from_ms,
            session: session.map(str::to_owned),
            work,
            source: "use".to_owned(),
        }
    }

    fn response(id: &str, session: &str, at_ms: u64, output: u64) -> Response {
        Response {
            message_id: id.to_owned(),
            session: session.to_owned(),
            agent_id: None,
            at_ms,
            model: "claude-opus-5-5".to_owned(),
            speed: "standard".to_owned(),
            tokens: Tokens {
                input: 10,
                output,
                ..Tokens::default()
            },
            calls: Vec::new(),
        }
    }

    async fn conn() -> (Store, Connection) {
        let store = Store::open_in_memory().await.unwrap();
        let conn = store.connect().unwrap();
        (store, conn)
    }

    async fn works(conn: &Connection) -> Vec<(String, Option<i64>)> {
        let mut rows = conn
            .query(
                "SELECT message_id, work FROM host_usage ORDER BY message_id",
                (),
            )
            .await
            .unwrap();
        let mut found = Vec::new();
        while let Some(row) = rows.next().await.unwrap() {
            let mut cells = Cells::new(&row);
            found.push((
                cells.text("id").unwrap(),
                cells.optional_id("work").unwrap(),
            ));
        }
        found
    }

    async fn new_item(conn: &Connection) -> i64 {
        conn.execute("INSERT INTO work_item (created_at_ms) VALUES (0)", ())
            .await
            .unwrap();
        conn.last_insert_rowid()
    }

    #[test]
    fn nothing_is_active_before_any_focus_or_after_a_clear() {
        let focus = vec![focus_row(100, None, Some(1)), focus_row(200, None, None)];
        assert_eq!(active(&focus, "s1", 50), None);
        assert_eq!(active(&focus, "s1", 100), Some(1));
        assert_eq!(active(&focus, "s1", 199), Some(1));
        assert_eq!(active(&focus, "s1", 200), None);
    }

    #[test]
    fn a_session_focus_beats_an_older_global_one_and_leaves_other_sessions_alone() {
        let focus = vec![
            focus_row(100, None, Some(1)),
            focus_row(150, Some("s2"), Some(2)),
        ];
        assert_eq!(active(&focus, "s1", 160), Some(1));
        assert_eq!(active(&focus, "s2", 160), Some(2));
        assert_eq!(active(&focus, "s2", 120), Some(1));
        // A newer global focus covers every session again.
        let mut later = focus.clone();
        later.push(focus_row(300, None, Some(3)));
        assert_eq!(active(&later, "s2", 300), Some(3));
    }

    #[test]
    fn a_turn_spanning_a_change_is_split_by_response_time() {
        // `work use --issue 580` at 10:00, `send --issue 590` at 10:20:00; responses at
        // 10:19:55 and 10:20:10 of one turn go to each.
        let minute = 60_000;
        let focus = vec![
            focus_row(0, Some("s1"), Some(580)),
            focus_row(20 * minute, Some("s1"), Some(590)),
        ];
        assert_eq!(active(&focus, "s1", 20 * minute - 5_000), Some(580));
        assert_eq!(active(&focus, "s1", 20 * minute + 10_000), Some(590));
    }

    #[test]
    fn a_merge_keeps_the_first_session_and_never_lowers_a_count() {
        let stored = Stored {
            session: "s1".to_owned(),
            agent_id: None,
            at_ms: 100,
            last_at_ms: 100,
            model: "claude-opus-5-5".to_owned(),
            speed: "standard".to_owned(),
            tokens: Tokens {
                input: 10,
                output: 50,
                ..Tokens::default()
            },
            cost_usd: Some(0.001_04),
        };
        let mut partial = stored.clone();
        partial.session = "fork".to_owned();
        partial.at_ms = 200;
        partial.last_at_ms = 200;
        partial.tokens.output = 5;
        let merged = merge(&stored, &partial);
        assert_eq!(merged.session, "s1");
        assert_eq!(merged.at_ms, 100);
        assert_eq!(merged.last_at_ms, 200);
        assert_eq!(merged.tokens.output, 50);
        assert_eq!(merged.cost_usd, stored.cost_usd);
    }

    #[tokio::test]
    async fn the_same_response_twice_changes_nothing() {
        let (_store, conn) = conn().await;
        assert!(
            keep_response(&conn, &response("m1", "s1", 100, 50))
                .await
                .unwrap()
        );
        assert!(
            !keep_response(&conn, &response("m1", "s1", 100, 50))
                .await
                .unwrap()
        );
        // A forked session's copy, seen later: only its later time is taken.
        assert!(
            !keep_response(&conn, &response("m1", "fork", 300, 50))
                .await
                .unwrap()
        );
        let rows = host_rows(&conn, None, None).await.unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].tokens.output, 50);
        assert_eq!(rows[0].at_ms, 100);
    }

    #[tokio::test]
    async fn reprice_fills_only_the_rows_without_a_cost() {
        let (_store, conn) = conn().await;
        keep_response(&conn, &response("m1", "s1", 100, 50))
            .await
            .unwrap();
        // A row stored without a cost (its rate came later), and one priced at an older rate.
        conn.execute(
            "UPDATE host_usage SET cost_usd = NULL WHERE message_id = 'm1'",
            (),
        )
        .await
        .unwrap();
        let mut old = response("m2", "s1", 200, 50);
        old.speed = "fast".to_owned();
        keep_response(&conn, &old).await.unwrap();
        keep_response(&conn, &response("m3", "s1", 300, 50))
            .await
            .unwrap();
        conn.execute(
            "UPDATE host_usage SET cost_usd = 9.0 WHERE message_id = 'm3'",
            (),
        )
        .await
        .unwrap();
        assert_eq!(reprice(&conn).await.unwrap(), 1);
        let costs: Vec<Option<f64>> = host_rows(&conn, None, None)
            .await
            .unwrap()
            .into_iter()
            .map(|row| row.cost_usd)
            .collect();
        // m1 priced, m2 (fast mode) still not, m3 kept at its old figure.
        assert_eq!(costs, [Some(0.001_04), None, Some(9.0)]);
    }

    #[tokio::test]
    async fn a_late_focus_moves_unpinned_rows_and_leaves_pinned_ones() {
        let (_store, conn) = conn().await;
        let first = new_item(&conn).await;
        let second = new_item(&conn).await;
        for (id, at_ms) in [("m1", 100), ("m2", 200), ("m3", 300)] {
            keep_response(&conn, &response(id, "s1", at_ms, 5))
                .await
                .unwrap();
        }
        add_focus(&conn, 50, None, Some(first), "use", None)
            .await
            .unwrap();
        assert_eq!(attribute(&conn).await.unwrap(), 3);
        let moved = reassign(
            &conn,
            &Reassign {
                from: Some(first),
                to: second,
                since_ms: Some(300),
                until_ms: None,
                session: None,
            },
        )
        .await
        .unwrap();
        assert_eq!(moved, 1);
        // A focus recorded late (the `send` was caught up after the import) moves m2 and m3,
        // but m3 is pinned.
        add_focus(&conn, 150, Some("s1"), None, "clear", None)
            .await
            .unwrap();
        assert_eq!(attribute(&conn).await.unwrap(), 1);
        assert_eq!(
            works(&conn).await,
            [
                ("m1".to_owned(), Some(first)),
                ("m2".to_owned(), None),
                ("m3".to_owned(), Some(second)),
            ]
        );
        // Reassigning the unattributed ones moves m2.
        let moved = reassign(
            &conn,
            &Reassign {
                from: None,
                to: first,
                since_ms: None,
                until_ms: None,
                session: Some("s1".to_owned()),
            },
        )
        .await
        .unwrap();
        assert_eq!(moved, 1);
    }

    #[tokio::test]
    async fn a_subagent_is_charged_whole_to_the_item_active_when_it_started() {
        let (_store, conn) = conn().await;
        let first = new_item(&conn).await;
        let second = new_item(&conn).await;
        save_cursor(
            &conn,
            &Transcript {
                path: "/p/s1/subagents/agent-a1.jsonl",
                project: "/p",
                session: "s1",
                agent_id: Some("a1"),
                cursor: Cursor {
                    started_at_ms: Some(100),
                    ..Cursor::default()
                },
                imported_at_ms: 0,
            },
        )
        .await
        .unwrap();
        let mut late = response("m1", "s1", 500, 5);
        late.agent_id = Some("a1".to_owned());
        keep_response(&conn, &late).await.unwrap();
        keep_response(&conn, &response("m2", "s1", 500, 5))
            .await
            .unwrap();
        add_focus(&conn, 50, None, Some(first), "use", None)
            .await
            .unwrap();
        add_focus(&conn, 200, None, Some(second), "send", Some("t1"))
            .await
            .unwrap();
        attribute(&conn).await.unwrap();
        assert_eq!(
            works(&conn).await,
            [
                ("m1".to_owned(), Some(first)),
                ("m2".to_owned(), Some(second))
            ]
        );
    }

    #[tokio::test]
    async fn the_install_time_is_set_once() {
        let (_store, conn) = conn().await;
        assert_eq!(installed_at(&conn, 1_000).await.unwrap(), 1_000);
        assert_eq!(installed_at(&conn, 2_000).await.unwrap(), 1_000);
        assert_eq!(totals(&conn).await.unwrap().installed_at_ms, Some(1_000));
    }
}
