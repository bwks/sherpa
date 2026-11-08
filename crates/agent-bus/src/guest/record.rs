//! A record of every `result` event Claude Code prints, to find out what its cost and time
//! figures mean across resumed runs.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use serde::Serialize;
use serde_json::Value;

/// The file in the state directory.
const FILE_NAME: &str = "results.jsonl";
/// The mode of the directory: private to the user.
const DIR_MODE: u32 = 0o700;
/// The mode of the file: private to the user.
const FILE_MODE: u32 = 0o600;

/// One line of the record: a `result` event as Claude Code printed it, and where it came from.
#[derive(Debug, Serialize)]
struct Entry<'a> {
    task: &'a str,
    session: &'a str,
    /// The run within the task: 1 is the first, 2 and up are the runs that answer.
    run: u32,
    /// When the line was recorded, in milliseconds since the Unix epoch.
    at_ms: u128,
    /// The event line, unchanged.
    raw: &'a str,
}

/// The file the result events are appended to.
#[derive(Debug, Clone)]
pub(super) struct ResultLog {
    path: PathBuf,
}

impl ResultLog {
    pub(super) fn new(path: PathBuf) -> Self {
        Self { path }
    }

    /// The record in `$XDG_STATE_HOME/agent-bus/`, or `~/.local/state/agent-bus/`. A relative
    /// `XDG_STATE_HOME` is ignored, as the spec says.
    pub(super) fn standard() -> Option<Self> {
        let state = std::env::var_os("XDG_STATE_HOME").map(PathBuf::from);
        let base = match state {
            Some(path) if path.is_absolute() => path,
            _ => {
                let home = std::env::var_os("HOME").map(PathBuf::from)?;
                if !home.is_absolute() {
                    return None;
                }
                home.join(".local").join("state")
            }
        };
        Some(Self::new(base.join("agent-bus").join(FILE_NAME)))
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

    /// Append a `result` event line to the record, wrapped with its task, session and run.
    ///
    /// # Errors
    ///
    /// Fails when the directory or the file cannot be written.
    pub(super) fn append(&self, task: &str, session: &str, run: u32, raw: &str) -> Result<()> {
        let at_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_millis());
        let mut line = serde_json::to_string(&Entry {
            task,
            session,
            run,
            at_ms,
            raw,
        })
        .context("Unable to render the result record")?;
        line.push('\n');
        if let Some(dir) = self.path.parent() {
            fs::create_dir_all(dir)
                .with_context(|| format!("Unable to create {}", dir.display()))?;
            fs::set_permissions(dir, fs::Permissions::from_mode(DIR_MODE))
                .with_context(|| format!("Unable to set the mode of {}", dir.display()))?;
        }
        let mut file = OpenOptions::new()
            .append(true)
            .create(true)
            .mode(FILE_MODE)
            .open(&self.path)
            .with_context(|| format!("Unable to open {}", self.path.display()))?;
        // `mode` is masked by the umask and ignored for a file that exists.
        fs::set_permissions(&self.path, fs::Permissions::from_mode(FILE_MODE))
            .with_context(|| format!("Unable to set the mode of {}", self.path.display()))?;
        file.write_all(line.as_bytes())
            .with_context(|| format!("Unable to write {}", self.path.display()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::env;

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
        let log = ResultLog::new(dir.join("state").join(FILE_NAME));
        log.append("t1", "s1", 1, RESULT).unwrap();
        log.append("t1", "s1", 2, RESULT).unwrap();

        let text = fs::read_to_string(&log.path).unwrap();
        let lines: Vec<Value> = text
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0]["task"], "t1");
        assert_eq!(lines[0]["session"], "s1");
        assert_eq!(lines[0]["run"], 1);
        assert_eq!(lines[1]["run"], 2);
        assert!(lines[0]["at_ms"].as_u64().unwrap() > 0);
        // The event is kept as the exact text Claude Code printed.
        assert_eq!(lines[0]["raw"], RESULT);

        let mode =
            |path: &std::path::Path| fs::metadata(path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&log.path), FILE_MODE);
        assert_eq!(mode(log.path.parent().unwrap()), DIR_MODE);
        fs::remove_dir_all(&dir).unwrap();
    }
}
