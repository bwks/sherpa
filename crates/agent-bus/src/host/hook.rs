//! `host import --hook`: what the `SessionEnd` hook reads, and the project it sweeps, with no
//! I/O.
//!
//! Claude Code gives a command hook a JSON object on stdin. The hook uses `transcript_path`
//! (whose directory is the project to sweep, when `--project` and `AGENT_BUS_HOST_PROJECT` name
//! none), and logs `session_id` and `hook_event_name`; it ignores the rest. It prints nothing on
//! stdout, exits 0 when it imported or the record was busy, and 1 on a failure, with one line on
//! stderr. It never exits 2, which would stop Claude.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, ensure};
use serde::Deserialize;

/// What the hook reads from stdin.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Input {
    pub transcript_path: PathBuf,
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default)]
    pub hook_event_name: Option<String>,
}

/// How the hook exits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Exit {
    /// Imported, or the record was busy: 0.
    Done,
    /// A real failure, said on stderr: 1.
    Failed,
}

impl Exit {
    /// The process's exit code: 0 or 1, never 2.
    pub fn code(self) -> u8 {
        match self {
            Self::Done => 0,
            Self::Failed => 1,
        }
    }
}

/// The hook's input from `stdin`.
///
/// # Errors
///
/// Fails when it is empty, not JSON, or has no `transcript_path`.
pub fn read_input(stdin: &str) -> Result<Input> {
    ensure!(!stdin.trim().is_empty(), "The hook's input is empty");
    let input: Input = serde_json::from_str(stdin)
        .context("The hook's input is not the JSON Claude Code sends")?;
    ensure!(
        input.transcript_path.is_absolute(),
        "The hook's transcript_path is not an absolute path"
    );
    Ok(input)
}

/// The project directory to sweep: `configured` (`--project`, then `AGENT_BUS_HOST_PROJECT`)
/// when set, else the directory of the hook's transcript.
///
/// # Errors
///
/// Fails when neither names a directory.
pub fn project(configured: Option<&Path>, input: &Input) -> Result<PathBuf> {
    if let Some(project) = configured {
        return Ok(project.to_owned());
    }
    input
        .transcript_path
        .parent()
        .map(Path::to_owned)
        .context("The hook's transcript_path has no directory")
}

#[cfg(test)]
mod tests {
    use super::*;

    use serde_json::json;

    #[test]
    fn the_hooks_input_names_the_transcript_and_the_rest_is_ignored() {
        let stdin = json!({
            "session_id": "s1",
            "transcript_path": "/home/u/.claude/projects/-home-u-code/s1.jsonl",
            "cwd": "/home/u/code",
            "hook_event_name": "SessionEnd",
            "reason": "exit"
        })
        .to_string();
        let input = read_input(&stdin).unwrap();
        assert_eq!(input.session_id.as_deref(), Some("s1"));
        assert_eq!(input.hook_event_name.as_deref(), Some("SessionEnd"));
        assert_eq!(
            project(None, &input).unwrap(),
            Path::new("/home/u/.claude/projects/-home-u-code")
        );
        assert_eq!(
            project(Some(Path::new("/elsewhere")), &input).unwrap(),
            Path::new("/elsewhere")
        );
    }

    #[test]
    fn empty_garbage_or_incomplete_input_is_refused() {
        for stdin in [
            String::new(),
            "  \n".to_owned(),
            "garbage".to_owned(),
            json!({ "session_id": "s1" }).to_string(),
            json!({ "transcript_path": "relative.jsonl" }).to_string(),
        ] {
            assert!(read_input(&stdin).is_err(), "{stdin:?}");
        }
    }

    #[test]
    fn the_exit_code_is_never_two() {
        assert_eq!(Exit::Done.code(), 0);
        assert_eq!(Exit::Failed.code(), 1);
    }
}
