//! A record of every `result` event Claude Code prints, to find out what its cost and time
//! figures mean across resumed runs. Lines older than the stream's retention are pruned (see
//! `files::JsonLines`).

use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::Serialize;
use serde_json::Value;

use crate::files::{JsonLines, standard_state_dir};

/// The file in the state directory.
const FILE_NAME: &str = "results.jsonl";

/// One line of the record: a `result` event as Claude Code printed it, and where it came from.
#[derive(Debug, Serialize)]
struct Entry<'a> {
    task: &'a str,
    session: &'a str,
    /// The run within the task: 1 is the first, 2 and up are the runs that answer.
    run: u32,
    /// When the line was recorded, in milliseconds since the Unix epoch. Pruning reads it.
    at_ms: u64,
    /// The event line, unchanged.
    raw: &'a str,
}

/// The file the result events are appended to.
#[derive(Debug, Clone)]
pub(super) struct ResultLog {
    lines: JsonLines,
}

impl ResultLog {
    pub(super) fn new(path: PathBuf) -> Self {
        Self {
            lines: JsonLines::new(path),
        }
    }

    /// The record in the state directory; `None` when there is none.
    pub(super) fn standard() -> Option<Self> {
        standard_state_dir()
            .ok()
            .map(|dir| Self::new(dir.join(FILE_NAME)))
    }

    /// The session of a line if it is a `result` event, and `None` for any other line: only
    /// result events are recorded.
    pub(super) fn result_session(line: &str) -> Option<String> {
        let event = serde_json::from_str::<Value>(line).ok()?;
        if event["type"].as_str() != Some("result") {
            return None;
        }
        Some(event["session_id"].as_str().unwrap_or_default().to_owned())
    }

    /// Drop the lines older than the stream's retention; how many were dropped.
    ///
    /// # Errors
    ///
    /// Fails when the file cannot be read or rewritten.
    pub(super) fn prune(&mut self, now_ms: u64) -> Result<usize> {
        self.lines.prune(now_ms)
    }

    /// Append a `result` event line to the record, wrapped with its task, session and run. The
    /// record is pruned first if it was not in the last day.
    ///
    /// # Errors
    ///
    /// Fails when the directory or the file cannot be written.
    pub(super) fn append(
        &mut self,
        task: &str,
        session: &str,
        run: u32,
        raw: &str,
        now_ms: u64,
    ) -> Result<()> {
        let line = serde_json::to_string(&Entry {
            task,
            session,
            run,
            at_ms: now_ms,
            raw,
        })
        .context("Unable to render the result record")?;
        self.lines.append(&line, now_ms)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::env;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;

    use crate::files::{DIR_MODE, PRIVATE_FILE_MODE, now_ms};

    const RESULT: &str =
        r#"{"type":"result","session_id":"s1","total_cost_usd":0.05,"result":"ok"}"#;

    #[test]
    fn only_result_events_are_recorded() {
        assert_eq!(ResultLog::result_session(RESULT), Some("s1".to_owned()));
        for line in [
            "",
            "not json",
            r#"{"type":"assistant","session_id":"s1"}"#,
            r#"{"type":"system","session_id":"s1"}"#,
        ] {
            assert_eq!(ResultLog::result_session(line), None, "{line}");
        }
    }

    #[test]
    fn lines_are_wrapped_appended_and_private() {
        let dir = env::temp_dir().join(format!("agent-bus-record-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let mut log = ResultLog::new(dir.join("state").join(FILE_NAME));
        let now = now_ms();
        log.append("t1", "s1", 1, RESULT, now).unwrap();
        log.append("t1", "s1", 2, RESULT, now).unwrap();
        let path = log.lines.path();

        let text = fs::read_to_string(path).unwrap();
        let lines: Vec<Value> = text
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0]["task"], "t1");
        assert_eq!(lines[0]["session"], "s1");
        assert_eq!(lines[0]["run"], 1);
        assert_eq!(lines[1]["run"], 2);
        assert_eq!(lines[0]["at_ms"].as_u64(), Some(now));
        // The event is kept as the exact text Claude Code printed.
        assert_eq!(lines[0]["raw"], RESULT);

        let mode =
            |path: &std::path::Path| fs::metadata(path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(path), PRIVATE_FILE_MODE);
        assert_eq!(mode(path.parent().unwrap()), DIR_MODE);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn results_older_than_the_retention_are_pruned() {
        const DAY: u64 = 24 * 60 * 60 * 1000;
        let dir = env::temp_dir().join(format!("agent-bus-record-prune-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let now = now_ms();
        let mut log = ResultLog::new(dir.join(FILE_NAME));
        log.append("old", "s1", 1, RESULT, now - 8 * DAY).unwrap();
        log.append("new", "s2", 1, RESULT, now - DAY).unwrap();
        assert_eq!(log.prune(now).unwrap(), 1);
        let text = fs::read_to_string(log.lines.path()).unwrap();
        let tasks: Vec<String> = text
            .lines()
            .map(|line| serde_json::from_str::<Value>(line).unwrap()["task"].to_string())
            .collect();
        assert_eq!(tasks, ["\"new\""]);
        let mode = fs::metadata(log.lines.path()).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, PRIVATE_FILE_MODE);
        fs::remove_dir_all(&dir).unwrap();
    }
}
