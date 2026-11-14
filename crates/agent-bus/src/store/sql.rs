//! The SQL of the record, one statement per file under `sql/`, as `crates/db` keeps its own.
//! `database::tests` applies every migration to an empty database and prepares every query
//! against the result, so a statement that does not fit the schema fails `cargo test`.

/// Every migration, in order: the first takes an empty database to schema version 1.
pub const MIGRATIONS: &[&str] = &[
    include_str!("../../sql/migrations/0001_record.sql"),
    include_str!("../../sql/migrations/0002_work.sql"),
    include_str!("../../sql/migrations/0003_host.sql"),
];

pub const INSERT_MESSAGE: &str = include_str!("../../sql/queries/insert_message.sql");
pub const FIND_TASK: &str = include_str!("../../sql/queries/find_task.sql");
pub const UPSERT_TASK: &str = include_str!("../../sql/queries/upsert_task.sql");
pub const INSERT_RUN: &str = include_str!("../../sql/queries/insert_run.sql");
pub const INSERT_RUN_MODEL: &str = include_str!("../../sql/queries/insert_run_model.sql");
pub const USAGE: &str = include_str!("../../sql/queries/usage.sql");
pub const TASK_MESSAGES: &str = include_str!("../../sql/queries/task_messages.sql");
pub const HISTORY: &str = include_str!("../../sql/queries/history.sql");
pub const FIND_REF_WORK: &str = include_str!("../../sql/queries/find_ref_work.sql");
pub const FIND_TASK_WORK: &str = include_str!("../../sql/queries/find_task_work.sql");
pub const INSERT_WORK_ITEM: &str = include_str!("../../sql/queries/insert_work_item.sql");
pub const INSERT_WORK_REF: &str = include_str!("../../sql/queries/insert_work_ref.sql");
pub const INSERT_TASK_REF: &str = include_str!("../../sql/queries/insert_task_ref.sql");
pub const INSERT_TASK_WORK: &str = include_str!("../../sql/queries/insert_task_work.sql");
pub const MERGE_WORK_ITEM: &str = include_str!("../../sql/queries/merge_work_item.sql");
pub const MOVE_WORK_REFS: &str = include_str!("../../sql/queries/move_work_refs.sql");
pub const MOVE_TASK_WORK: &str = include_str!("../../sql/queries/move_task_work.sql");
pub const MOVE_HOST_USAGE: &str = include_str!("../../sql/queries/move_host_usage.sql");
pub const MOVE_HOST_FOCUS: &str = include_str!("../../sql/queries/move_host_focus.sql");
pub const ITEM_REFS: &str = include_str!("../../sql/queries/item_refs.sql");
pub const ITEM_TASKS: &str = include_str!("../../sql/queries/item_tasks.sql");
pub const ITEM_CONFLICTS: &str = include_str!("../../sql/queries/item_conflicts.sql");
pub const MERGED_ITEMS: &str = include_str!("../../sql/queries/merged_items.sql");
pub const WORK_ITEMS: &str = include_str!("../../sql/queries/work_items.sql");
pub const UNREPORTED_OUTCOMES: &str = include_str!("../../sql/queries/unreported_outcomes.sql");
pub const OPEN_TASKS: &str = include_str!("../../sql/queries/open_tasks.sql");
pub const RUNS: &str = include_str!("../../sql/queries/runs.sql");
pub const HOST_META: &str = include_str!("../../sql/queries/host_meta.sql");
pub const SET_HOST_META: &str = include_str!("../../sql/queries/set_host_meta.sql");
pub const FIND_TRANSCRIPT: &str = include_str!("../../sql/queries/find_transcript.sql");
pub const UPSERT_TRANSCRIPT: &str = include_str!("../../sql/queries/upsert_transcript.sql");
pub const RESCAN_TRANSCRIPTS: &str = include_str!("../../sql/queries/rescan_transcripts.sql");
pub const FIND_HOST_USAGE: &str = include_str!("../../sql/queries/find_host_usage.sql");
pub const UPSERT_HOST_USAGE: &str = include_str!("../../sql/queries/upsert_host_usage.sql");
pub const INSERT_HOST_SPAN: &str = include_str!("../../sql/queries/insert_host_span.sql");
pub const END_HOST_SPAN: &str = include_str!("../../sql/queries/end_host_span.sql");
pub const UNPRICED_HOST_USAGE: &str = include_str!("../../sql/queries/unpriced_host_usage.sql");
pub const PRICE_HOST_USAGE: &str = include_str!("../../sql/queries/price_host_usage.sql");
pub const HOST_FOCUS: &str = include_str!("../../sql/queries/host_focus.sql");
pub const INSERT_HOST_FOCUS: &str = include_str!("../../sql/queries/insert_host_focus.sql");
pub const UNPINNED_HOST_USAGE: &str = include_str!("../../sql/queries/unpinned_host_usage.sql");
pub const SUBAGENT_STARTS: &str = include_str!("../../sql/queries/subagent_starts.sql");
pub const ATTRIBUTE_HOST_USAGE: &str = include_str!("../../sql/queries/attribute_host_usage.sql");
pub const REASSIGN_HOST_USAGE: &str = include_str!("../../sql/queries/reassign_host_usage.sql");
pub const HOST_USAGE_ROWS: &str = include_str!("../../sql/queries/host_usage_rows.sql");
pub const HOST_SPANS: &str = include_str!("../../sql/queries/host_spans.sql");
pub const HOST_RESPONSES: &str = include_str!("../../sql/queries/host_responses.sql");
pub const TASK_WAITS: &str = include_str!("../../sql/queries/task_waits.sql");
pub const HOST_TRANSCRIPTS: &str = include_str!("../../sql/queries/host_transcripts.sql");
pub const HOST_TOTALS: &str = include_str!("../../sql/queries/host_totals.sql");

/// Every query, by its file name, for the test that prepares them.
#[cfg(test)]
pub const QUERIES: &[(&str, &str)] = &[
    ("insert_message.sql", INSERT_MESSAGE),
    ("find_task.sql", FIND_TASK),
    ("upsert_task.sql", UPSERT_TASK),
    ("insert_run.sql", INSERT_RUN),
    ("insert_run_model.sql", INSERT_RUN_MODEL),
    ("usage.sql", USAGE),
    ("task_messages.sql", TASK_MESSAGES),
    ("history.sql", HISTORY),
    ("find_ref_work.sql", FIND_REF_WORK),
    ("find_task_work.sql", FIND_TASK_WORK),
    ("insert_work_item.sql", INSERT_WORK_ITEM),
    ("insert_work_ref.sql", INSERT_WORK_REF),
    ("insert_task_ref.sql", INSERT_TASK_REF),
    ("insert_task_work.sql", INSERT_TASK_WORK),
    ("merge_work_item.sql", MERGE_WORK_ITEM),
    ("move_work_refs.sql", MOVE_WORK_REFS),
    ("move_task_work.sql", MOVE_TASK_WORK),
    ("move_host_usage.sql", MOVE_HOST_USAGE),
    ("move_host_focus.sql", MOVE_HOST_FOCUS),
    ("item_refs.sql", ITEM_REFS),
    ("item_tasks.sql", ITEM_TASKS),
    ("item_conflicts.sql", ITEM_CONFLICTS),
    ("merged_items.sql", MERGED_ITEMS),
    ("work_items.sql", WORK_ITEMS),
    ("unreported_outcomes.sql", UNREPORTED_OUTCOMES),
    ("open_tasks.sql", OPEN_TASKS),
    ("runs.sql", RUNS),
    ("host_meta.sql", HOST_META),
    ("set_host_meta.sql", SET_HOST_META),
    ("find_transcript.sql", FIND_TRANSCRIPT),
    ("upsert_transcript.sql", UPSERT_TRANSCRIPT),
    ("rescan_transcripts.sql", RESCAN_TRANSCRIPTS),
    ("find_host_usage.sql", FIND_HOST_USAGE),
    ("upsert_host_usage.sql", UPSERT_HOST_USAGE),
    ("insert_host_span.sql", INSERT_HOST_SPAN),
    ("end_host_span.sql", END_HOST_SPAN),
    ("unpriced_host_usage.sql", UNPRICED_HOST_USAGE),
    ("price_host_usage.sql", PRICE_HOST_USAGE),
    ("host_focus.sql", HOST_FOCUS),
    ("insert_host_focus.sql", INSERT_HOST_FOCUS),
    ("unpinned_host_usage.sql", UNPINNED_HOST_USAGE),
    ("subagent_starts.sql", SUBAGENT_STARTS),
    ("attribute_host_usage.sql", ATTRIBUTE_HOST_USAGE),
    ("reassign_host_usage.sql", REASSIGN_HOST_USAGE),
    ("host_usage_rows.sql", HOST_USAGE_ROWS),
    ("host_spans.sql", HOST_SPANS),
    ("host_responses.sql", HOST_RESPONSES),
    ("task_waits.sql", TASK_WAITS),
    ("host_transcripts.sql", HOST_TRANSCRIPTS),
    ("host_totals.sql", HOST_TOTALS),
];
