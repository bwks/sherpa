//! The SQL of the record, one statement per file under `sql/`, as `crates/db` keeps its own.
//! `database::tests` applies every migration to an empty database and prepares every query
//! against the result, so a statement that does not fit the schema fails `cargo test`.

/// Every migration, in order: the first takes an empty database to schema version 1.
pub const MIGRATIONS: &[&str] = &[include_str!("../../sql/migrations/0001_record.sql")];

pub const INSERT_MESSAGE: &str = include_str!("../../sql/queries/insert_message.sql");
pub const FIND_TASK: &str = include_str!("../../sql/queries/find_task.sql");
pub const UPSERT_TASK: &str = include_str!("../../sql/queries/upsert_task.sql");
pub const INSERT_RUN: &str = include_str!("../../sql/queries/insert_run.sql");
pub const INSERT_RUN_MODEL: &str = include_str!("../../sql/queries/insert_run_model.sql");
pub const USAGE: &str = include_str!("../../sql/queries/usage.sql");
pub const TASK_MESSAGES: &str = include_str!("../../sql/queries/task_messages.sql");
pub const HISTORY: &str = include_str!("../../sql/queries/history.sql");

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
];
