//! `host import`: read the host's Claude Code transcripts into the record.
//!
//! Every `*.jsonl` under the project directory is a transcript: the main ones at
//! `<project>/<session>.jsonl`, a subagent's at `<project>/<session>/subagents/agent-<id>.jsonl`.
//! Each file has a cursor: the inode and the offset just after the last complete line read. A
//! file whose size equals its offset is not opened; one that grew is read from the offset; one
//! that shrank, or whose inode changed, is read from the start. A half-written last line is left
//! for the next run; a complete line that does not parse is counted and passed. Reading from the
//! start is always safe: every row is keyed (a response by its `message.id`, a prompt by its
//! line's `uuid`, a tool call by its ID) so the same bytes twice change nothing.
//!
//! Lines from before the first import (the install time, kept in the record) are not imported.
//! Each file is one transaction: its rows and its cursor commit together. After the files, every
//! unpinned response is attributed again.

use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom};
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::str;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use tracing::instrument;
use turso::Connection;

use crate::host::transcript::{Entry, parse};
use crate::store::host::{
    Cursor, Transcript, attribute, cursor, end_call, installed_at, keep_call, keep_prompt,
    keep_response, reprice, rescan, save_cursor,
};

/// What a transcript's name ends with.
const TRANSCRIPT_EXTENSION: &str = "jsonl";
/// The directory a session keeps its subagents' transcripts in.
const SUBAGENTS_DIR: &str = "subagents";
/// What a subagent's transcript name starts with, before its ID.
const SUBAGENT_PREFIX: &str = "agent-";
/// How deep under the project directory transcripts are looked for.
const MAX_DEPTH: usize = 3;

/// What to import.
#[derive(Debug, Clone)]
pub struct Options {
    /// Claude Code's directory of the host's project, under `~/.claude/projects/`.
    pub project: PathBuf,
    /// Read every file from the start again.
    pub rescan: bool,
    /// Price the responses that have no cost and have a rate now.
    pub reprice: bool,
}

/// What an import did.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Imported {
    /// Transcripts found.
    pub files: u64,
    /// Transcripts opened: new or grown.
    pub read: u64,
    /// Responses new to the record.
    pub responses: u64,
    /// Complete lines that did not parse.
    pub skipped: u64,
    /// Responses priced by `--reprice`.
    pub priced: u64,
    /// Responses whose item changed.
    pub attributed: u64,
    /// Lines are imported from this time on.
    pub installed_at_ms: u64,
}

/// One transcript file and whose it is.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Found {
    path: PathBuf,
    session: String,
    agent_id: Option<String>,
}

/// Import the transcripts of `options.project` into the record on `conn`, at `now_ms`.
///
/// # Errors
///
/// Fails when the project directory cannot be read, or the record cannot be written; files
/// committed before the failure stay imported.
#[instrument(skip(conn), fields(project = %options.project.display()))]
pub async fn import(conn: &mut Connection, options: &Options, now_ms: u64) -> Result<Imported> {
    let project = options
        .project
        .to_str()
        .context("The project directory's path is not valid UTF-8")?;
    let mut imported = Imported {
        installed_at_ms: installed_at(conn, now_ms).await?,
        ..Imported::default()
    };
    if options.rescan {
        rescan(conn, project).await?;
    }
    let mut found = Vec::new();
    find(&options.project, 0, &mut found).with_context(|| {
        format!(
            "Unable to read the transcripts in {}",
            options.project.display()
        )
    })?;
    found.sort_by(|a, b| a.path.cmp(&b.path));
    for file in &found {
        imported.files += 1;
        let path = file
            .path
            .to_str()
            .context("A transcript's path is not valid UTF-8")?;
        // A file deleted since it was listed is passed.
        let Ok(metadata) = fs::metadata(&file.path) else {
            continue;
        };
        let known = cursor(conn, path).await?;
        let start = start_offset(known.as_ref(), metadata.ino(), metadata.len());
        let Some(start) = start else {
            continue;
        };
        imported.read += 1;
        let bytes = read_from(&file.path, start)
            .with_context(|| format!("Unable to read {}", file.path.display()))?;
        let complete = bytes
            .iter()
            .rposition(|byte| *byte == b'\n')
            .map_or(0, |end| end + 1);
        let tx = conn
            .transaction()
            .await
            .context("Unable to start a transaction")?;
        let mut kept = known.filter(|_| start > 0).unwrap_or(Cursor {
            inode: metadata.ino(),
            ..Cursor::default()
        });
        for line in bytes[..complete].split(|byte| *byte == b'\n') {
            if line.iter().all(u8::is_ascii_whitespace) {
                continue;
            }
            let entry = str::from_utf8(line)
                .ok()
                .and_then(|line| parse(line, &file.session, file.agent_id.as_deref()).ok());
            let Some(entry) = entry else {
                kept.skipped += 1;
                imported.skipped += 1;
                continue;
            };
            imported.responses +=
                u64::from(keep(&tx, &entry, imported.installed_at_ms, &mut kept).await?);
        }
        kept.inode = metadata.ino();
        kept.offset = start + u64::try_from(complete).unwrap_or(u64::MAX);
        save_cursor(
            &tx,
            &Transcript {
                path,
                project,
                session: &file.session,
                agent_id: file.agent_id.as_deref(),
                cursor: kept,
                imported_at_ms: now_ms,
            },
        )
        .await?;
        tx.commit()
            .await
            .with_context(|| format!("Unable to commit {}", file.path.display()))?;
    }
    if options.reprice {
        imported.priced = reprice(conn).await?;
    }
    imported.attributed = attribute(conn).await?;
    Ok(imported)
}

/// Where to start reading a file of `inode` and `size`, given where reading it stopped; `None`
/// when there is nothing new.
fn start_offset(known: Option<&Cursor>, inode: u64, size: u64) -> Option<u64> {
    match known {
        None => Some(0),
        Some(cursor) if cursor.inode != inode || size < cursor.offset => Some(0),
        Some(cursor) if size == cursor.offset => None,
        Some(cursor) => Some(cursor.offset),
    }
}

/// Record what `entry` says, unless it is from before `installed_at_ms`, and keep the time of
/// the file's first line in `cursor`. Returns whether a response was new to the record.
async fn keep(
    conn: &Connection,
    entry: &Entry,
    installed_at_ms: u64,
    cursor: &mut Cursor,
) -> Result<bool> {
    let at_ms = match entry {
        Entry::Response(response) => response.at_ms,
        Entry::Results { at_ms, .. } | Entry::Prompt { at_ms, .. } => *at_ms,
        Entry::Nothing => return Ok(false),
    };
    cursor.started_at_ms = Some(cursor.started_at_ms.map_or(at_ms, |first| first.min(at_ms)));
    if at_ms < installed_at_ms {
        return Ok(false);
    }
    match entry {
        Entry::Response(response) => {
            for (id, class) in &response.calls {
                keep_call(
                    conn,
                    id,
                    &response.session,
                    response.agent_id.as_deref(),
                    at_ms,
                    *class,
                )
                .await?;
            }
            keep_response(conn, response).await
        }
        Entry::Results { ids, .. } => {
            for id in ids {
                end_call(conn, id, at_ms).await?;
            }
            Ok(false)
        }
        Entry::Prompt {
            id,
            session,
            agent_id,
            from_user,
            ..
        } => {
            keep_prompt(conn, id, session, agent_id.as_deref(), at_ms, *from_user).await?;
            Ok(false)
        }
        Entry::Nothing => Ok(false),
    }
}

/// The bytes of the file at `path` from `offset` to its end.
fn read_from(path: &Path, offset: u64) -> Result<Vec<u8>> {
    let mut file = File::open(path)?;
    file.seek(SeekFrom::Start(offset))?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    Ok(bytes)
}

/// Add every transcript under `dir`, `depth` levels below the project directory, to `found`.
fn find(dir: &Path, depth: usize, found: &mut Vec<Found>) -> Result<()> {
    for entry in fs::read_dir(dir)? {
        let path = entry?.path();
        if path.is_dir() {
            if depth + 1 < MAX_DEPTH {
                find(&path, depth + 1, found)?;
            }
        } else if path
            .extension()
            .is_some_and(|extension| extension == TRANSCRIPT_EXTENSION)
        {
            found.push(whose(path));
        }
    }
    Ok(())
}

/// The session and subagent of the transcript at `path`, from where it is: a subagent's is
/// `<session>/subagents/agent-<id>.jsonl`; any other's session is its name.
fn whose(path: PathBuf) -> Found {
    let stem = path
        .file_stem()
        .map_or_else(String::new, |stem| stem.to_string_lossy().into_owned());
    let parent = path.parent();
    let in_subagents = parent
        .and_then(Path::file_name)
        .is_some_and(|name| name == SUBAGENTS_DIR);
    let session_dir = parent
        .and_then(Path::parent)
        .and_then(Path::file_name)
        .map(|name| name.to_string_lossy().into_owned());
    match (
        in_subagents,
        stem.strip_prefix(SUBAGENT_PREFIX),
        session_dir,
    ) {
        (true, Some(agent), Some(session)) => Found {
            agent_id: Some(agent.to_owned()),
            session,
            path,
        },
        _ => Found {
            session: stem,
            agent_id: None,
            path,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::fs::OpenOptions;
    use std::io::Write;
    use std::{env, process};

    use jiff::Timestamp;
    use serde_json::{Value, json};

    use crate::store::Store;
    use crate::store::host::{HostRow, host_rows, host_times};
    use crate::timing::Times;

    /// The install time of the tests, and the time of their lines from it.
    const T0: u64 = 1_794_600_000_000;

    fn scratch(name: &str) -> PathBuf {
        let dir = env::temp_dir().join(format!("agent-bus-import-{name}-{}", process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn at(ms: u64) -> String {
        Timestamp::from_millisecond(i64::try_from(T0 + ms).unwrap())
            .unwrap()
            .to_string()
    }

    fn assistant(session: &str, id: &str, ms: u64, output: u64, content: &Value) -> Value {
        json!({
            "type": "assistant", "sessionId": session, "isSidechain": false,
            "timestamp": at(ms), "uuid": format!("{id}-{ms}"),
            "message": {
                "id": id, "model": "claude-opus-5-5", "role": "assistant", "content": content,
                "usage": {
                    "input_tokens": 10, "output_tokens": output,
                    "cache_read_input_tokens": 1_000, "cache_creation_input_tokens": 100,
                    "cache_creation": {
                        "ephemeral_5m_input_tokens": 0, "ephemeral_1h_input_tokens": 100
                    },
                    "speed": "standard"
                }
            }
        })
    }

    fn prompt(session: &str, uuid: &str, ms: u64) -> Value {
        json!({
            "type": "user", "sessionId": session, "timestamp": at(ms), "uuid": uuid,
            "message": { "role": "user", "content": "go" }
        })
    }

    fn result(session: &str, tool: &str, ms: u64) -> Value {
        json!({
            "type": "user", "sessionId": session, "timestamp": at(ms), "uuid": format!("r-{tool}"),
            "message": { "role": "user", "content": [
                { "type": "tool_result", "tool_use_id": tool, "content": "ok" }
            ] }
        })
    }

    fn tool(id: &str, command: &str) -> Value {
        json!([{ "type": "tool_use", "id": id, "name": "Bash", "input": { "command": command } }])
    }

    fn write(path: &Path, lines: &[Value]) {
        let mut text = String::new();
        for line in lines {
            text.push_str(&line.to_string());
            text.push('\n');
        }
        fs::write(path, text).unwrap();
    }

    fn append(path: &Path, text: &str) {
        OpenOptions::new()
            .append(true)
            .open(path)
            .unwrap()
            .write_all(text.as_bytes())
            .unwrap();
    }

    async fn record() -> (Store, Connection) {
        let store = Store::open_in_memory().await.unwrap();
        let conn = store.connect().unwrap();
        (store, conn)
    }

    fn options(project: &Path) -> Options {
        Options {
            project: project.to_owned(),
            rescan: false,
            reprice: false,
        }
    }

    async fn rows(conn: &Connection) -> Vec<HostRow> {
        host_rows(conn, None, None).await.unwrap()
    }

    fn output(rows: &[HostRow]) -> u64 {
        rows.iter().map(|row| row.tokens.output).sum()
    }

    #[tokio::test]
    async fn a_split_message_counts_once_and_the_same_id_in_two_files_too() {
        let dir = scratch("split");
        // One response written as three lines (thinking, text, tool use), as Claude Code does.
        write(
            &dir.join("s1.jsonl"),
            &[
                prompt("s1", "p1", 0),
                assistant("s1", "m1", 1_000, 40, &json!([{ "type": "thinking" }])),
                assistant(
                    "s1",
                    "m1",
                    1_100,
                    40,
                    &json!([{ "type": "text", "text": "x" }]),
                ),
                assistant("s1", "m1", 1_200, 40, &tool("t1", "cargo test")),
                result("s1", "t1", 61_200),
                assistant(
                    "s1",
                    "m2",
                    62_000,
                    5,
                    &json!([{ "type": "text", "text": "done" }]),
                ),
            ],
        );
        // A forked session copies the history: m1 again, then its own m3.
        write(
            &dir.join("s2.jsonl"),
            &[
                assistant("s2", "m1", 1_000, 40, &json!([{ "type": "thinking" }])),
                assistant(
                    "s2",
                    "m3",
                    90_000,
                    7,
                    &json!([{ "type": "text", "text": "y" }]),
                ),
            ],
        );
        let (_store, mut conn) = record().await;
        let imported = import(&mut conn, &options(&dir), T0).await.unwrap();
        assert_eq!(imported.files, 2);
        assert_eq!(imported.installed_at_ms, T0);
        let found = rows(&conn).await;
        assert_eq!(found.len(), 3);
        assert_eq!(output(&found), 40 + 5 + 7);
        // One opus response of 10 input, 40 output, 1,000 cache read and 100 one-hour writes.
        assert_eq!(found[0].cost_usd, Some(0.001_84));
        // The turn: 1.2 s of work, the 60 s test, then 0.8 s of work.
        let times = host_times(&conn, None).await.unwrap();
        assert_eq!(
            times.get(&None).copied().unwrap_or_default(),
            Times {
                work: 1_200 + 800,
                testing: 60_000,
                waiting: 0,
            }
        );
        fs::remove_dir_all(&dir).unwrap();
    }

    #[tokio::test]
    async fn synthetic_garbage_and_a_half_written_line_are_handled() {
        let dir = scratch("garbage");
        let path = dir.join("s1.jsonl");
        let mut synthetic = assistant("s1", "m9", 10, 3, &json!([]));
        synthetic["message"]["model"] = json!("<synthetic>");
        let mut no_usage = assistant("s1", "m8", 10, 3, &json!([]));
        no_usage["message"].as_object_mut().unwrap().remove("usage");
        write(
            &path,
            &[
                synthetic,
                no_usage,
                json!({ "type": "cost-state", "totalCostUSD": 1.0 }),
                assistant("s1", "m1", 20, 4, &json!([])),
            ],
        );
        append(&path, "{not json\n");
        let half = assistant("s1", "m2", 30, 6, &json!([])).to_string();
        let (first, rest) = half.split_at(half.len() / 2);
        append(&path, first);
        let (_store, mut conn) = record().await;
        let imported = import(&mut conn, &options(&dir), T0).await.unwrap();
        assert_eq!(imported.skipped, 1);
        assert_eq!(output(&rows(&conn).await), 4);
        // The rest of the half-written line arrives: the next run reads it whole.
        append(&path, &format!("{rest}\n"));
        let imported = import(&mut conn, &options(&dir), T0 + 1).await.unwrap();
        assert_eq!(imported.skipped, 0);
        assert_eq!(imported.responses, 1);
        assert_eq!(output(&rows(&conn).await), 4 + 6);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[tokio::test]
    async fn appended_truncated_and_replaced_files_are_read_from_the_right_place() {
        let dir = scratch("cursor");
        let path = dir.join("s1.jsonl");
        write(&path, &[assistant("s1", "m1", 10, 1, &json!([]))]);
        let (_store, mut conn) = record().await;
        import(&mut conn, &options(&dir), T0).await.unwrap();
        // Unchanged: not opened.
        let imported = import(&mut conn, &options(&dir), T0).await.unwrap();
        assert_eq!((imported.files, imported.read), (1, 0));
        // Appended: read from the cursor.
        append(
            &path,
            &format!("{}\n", assistant("s1", "m2", 20, 2, &json!([]))),
        );
        let imported = import(&mut conn, &options(&dir), T0).await.unwrap();
        assert_eq!((imported.read, imported.responses), (1, 1));
        // Truncated to a shorter file: read from 0.
        write(&path, &[assistant("s1", "m3", 30, 4, &json!([]))]);
        let imported = import(&mut conn, &options(&dir), T0).await.unwrap();
        assert_eq!(imported.responses, 1);
        // Replaced by a new file of the same size (another inode): read from 0.
        let replacement = dir.join("new");
        write(&replacement, &[assistant("s1", "m4", 30, 8, &json!([]))]);
        fs::rename(&replacement, &path).unwrap();
        let imported = import(&mut conn, &options(&dir), T0).await.unwrap();
        assert_eq!(imported.responses, 1);
        assert_eq!(output(&rows(&conn).await), 1 + 2 + 4 + 8);
        // A rescan reads everything again and changes nothing.
        let rescan = Options {
            rescan: true,
            ..options(&dir)
        };
        let imported = import(&mut conn, &rescan, T0).await.unwrap();
        assert_eq!((imported.read, imported.responses), (1, 0));
        assert_eq!(output(&rows(&conn).await), 15);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[tokio::test]
    async fn importing_twice_gives_equal_totals_and_a_failed_file_leaves_the_others() {
        let dir = scratch("twice");
        write(
            &dir.join("a.jsonl"),
            &[assistant("a", "m1", 10, 1, &json!([]))],
        );
        write(
            &dir.join("b.jsonl"),
            &[assistant("b", "m2", 10, 2, &json!([]))],
        );
        let (_store, mut conn) = record().await;
        import(&mut conn, &options(&dir), T0).await.unwrap();
        let first = rows(&conn).await;
        // As if the run had stopped after the first file: the second's cursor is gone, so the
        // next run reads it again, and nothing is counted twice.
        conn.execute("DELETE FROM host_transcript WHERE path LIKE '%b.jsonl'", ())
            .await
            .unwrap();
        let imported = import(&mut conn, &options(&dir), T0).await.unwrap();
        assert_eq!((imported.read, imported.responses), (1, 0));
        assert_eq!(rows(&conn).await, first);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[tokio::test]
    async fn lines_from_before_the_first_import_are_not_imported() {
        let dir = scratch("install");
        let path = dir.join("s1.jsonl");
        write(&path, &[assistant("s1", "m1", 10, 1, &json!([]))]);
        let (_store, mut conn) = record().await;
        // The first import is at T0 + 1 s: the line at T0 + 10 ms is history.
        let imported = import(&mut conn, &options(&dir), T0 + 1_000).await.unwrap();
        assert_eq!(imported.installed_at_ms, T0 + 1_000);
        assert_eq!(rows(&conn).await, []);
        append(
            &path,
            &format!("{}\n", assistant("s1", "m2", 2_000, 2, &json!([]))),
        );
        import(&mut conn, &options(&dir), T0 + 5_000).await.unwrap();
        assert_eq!(output(&rows(&conn).await), 2);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[tokio::test]
    async fn subagent_transcripts_are_found_under_their_session() {
        let dir = scratch("subagents");
        fs::create_dir_all(dir.join("s1/subagents")).unwrap();
        fs::create_dir_all(dir.join("s1/tool-results")).unwrap();
        fs::write(dir.join("s1/tool-results/x.txt"), "not a transcript").unwrap();
        let mut line = assistant("s1", "m1", 10, 3, &json!([]));
        line["isSidechain"] = json!(true);
        line["agentId"] = json!("a1");
        write(&dir.join("s1/subagents/agent-a1.jsonl"), &[line]);
        fs::write(dir.join("s1/subagents/agent-a1.meta.json"), "{}").unwrap();
        let (_store, mut conn) = record().await;
        let imported = import(&mut conn, &options(&dir), T0).await.unwrap();
        assert_eq!(imported.files, 1);
        assert_eq!(output(&rows(&conn).await), 3);
        let found = cursor(
            &conn,
            dir.join("s1/subagents/agent-a1.jsonl").to_str().unwrap(),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(found.started_at_ms, Some(T0 + 10));
        assert_eq!(
            whose(dir.join("s1/subagents/agent-a1.jsonl")),
            Found {
                path: dir.join("s1/subagents/agent-a1.jsonl"),
                session: "s1".to_owned(),
                agent_id: Some("a1".to_owned()),
            }
        );
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_cursor_says_where_to_start() {
        let at = |inode, offset| Cursor {
            inode,
            offset,
            ..Cursor::default()
        };
        assert_eq!(start_offset(None, 1, 100), Some(0));
        assert_eq!(start_offset(Some(&at(1, 100)), 1, 100), None);
        assert_eq!(start_offset(Some(&at(1, 100)), 1, 150), Some(100));
        assert_eq!(start_offset(Some(&at(1, 100)), 1, 50), Some(0));
        assert_eq!(start_offset(Some(&at(1, 100)), 2, 150), Some(0));
    }
}
