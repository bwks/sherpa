//! `host import` and `host status`, the `SessionEnd` hook, and the import every record command
//! runs after catching up.
//!
//! The host's project directory (Claude Code's directory of the host's checkout under
//! `~/.claude/projects/`) comes from `--project`, else `AGENT_BUS_HOST_PROJECT`; nothing is
//! assumed. Without it, record commands import nothing. Each import writes how it went to
//! `host-import.toml` beside the record, which `host status` shows.

use std::fmt::Write as _;
use std::io::{self, ErrorKind, Read};
use std::path::{Path, PathBuf};
use std::{env, fs, process};

use anyhow::{Context, Result, bail};
use clap::Subcommand;
use serde::{Deserialize, Serialize};
use shared::util::{TableLayout, render_table};

use crate::files::{PRIVATE_FILE_MODE, now_ms, standard_state_dir, write_atomic};
use crate::host::hook::{Exit, project, read_input};
use crate::host::import::{Imported, Options, import};
use crate::record::{BusAccess, Busy, HOOK_LOCK_WAIT, LOCK_WAIT, Paths, Record};
use crate::report::history::utc_minute;
use crate::store::host::{Totals, TranscriptRow, totals, transcripts};

/// The variable naming the host's project directory under `~/.claude/projects/`.
pub const PROJECT_ENV: &str = "AGENT_BUS_HOST_PROJECT";
/// The right-aligned columns of the transcripts table: offset and skipped.
const TRANSCRIPT_NUMBER_COLUMNS: [usize; 2] = [2, 4];

/// What `host` does.
#[derive(Subcommand)]
pub enum HostAction {
    /// Import the host's Claude Code transcripts into the record, after catching it up from
    /// the bus. Every record command does this too when `AGENT_BUS_HOST_PROJECT` is set
    Import {
        /// The bus to catch the record up from, for example `nats://192.0.2.1:4222`. Without
        /// it: `AGENT_BUS_URL`, then where `bus start` recorded the bus
        #[arg(long)]
        server: Option<String>,
        /// Run as Claude Code's `SessionEnd` hook: read the hook's JSON on stdin, print nothing
        /// on stdout, wait 1 s for a busy record and leave it, and exit 0, or 1 on a failure
        #[arg(long)]
        hook: bool,
        /// Claude Code's directory of the host's project, under `~/.claude/projects/`. Without
        /// it: `AGENT_BUS_HOST_PROJECT`; in hook mode, else the hook transcript's directory
        #[arg(long)]
        project: Option<PathBuf>,
        /// Read every transcript from the start again. Nothing is counted twice
        #[arg(long)]
        rescan: bool,
        /// Price the responses that have no cost and have a rate now. Priced ones are never
        /// priced again
        #[arg(long)]
        reprice: bool,
    },
    /// Show the last import, its error if any, the host's totals and every transcript's cursor
    Status {
        /// Print it as JSON instead of tables
        #[arg(long)]
        json: bool,
    },
}

/// How the last import went, kept beside the record.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImportStatus {
    pub project: Option<String>,
    pub last_success_ms: Option<u64>,
    pub last: Option<Imported>,
    pub last_error_ms: Option<u64>,
    pub last_error: Option<String>,
}

/// What `host status` prints.
#[derive(Debug, Serialize)]
struct Status {
    import: ImportStatus,
    totals: Totals,
    transcripts: Vec<TranscriptRow>,
}

/// Run the `host` command `action` names, with `access` to catch the record up and `home_repo`
/// the host's repository.
///
/// # Errors
///
/// Fails when the project is not known, or the record cannot be opened, read or written.
pub async fn run(
    action: HostAction,
    access: Result<BusAccess>,
    home_repo: Option<&str>,
) -> Result<()> {
    let paths = Paths::in_dir(&standard_state_dir()?);
    match action {
        HostAction::Import {
            hook: true,
            project,
            rescan,
            reprice,
            ..
        } => {
            let mut stdin = String::new();
            let read = io::stdin().read_to_string(&mut stdin);
            let exit = match read {
                Ok(_) => {
                    let project = configured(project);
                    let request = Request {
                        project: project.as_deref(),
                        rescan,
                        reprice,
                    };
                    hook(&stdin, &paths, &request, access, home_repo, now_ms()).await
                }
                Err(error) => {
                    eprintln!("agent-bus host import --hook: unable to read stdin: {error}");
                    Exit::Failed
                }
            };
            // The hook's only output is its exit code: 0 or 1, never 2.
            process::exit(i32::from(exit.code()));
        }
        HostAction::Import {
            project,
            rescan,
            reprice,
            ..
        } => {
            let Some(project) = configured(project) else {
                bail!(
                    "Give the host's project directory under ~/.claude/projects/ with --project \
                     or {PROJECT_ENV}"
                );
            };
            let mut record = Record::open(&paths, access, home_repo, LOCK_WAIT).await?;
            if let Some(warning) = record.behind() {
                eprintln!("{warning}");
            }
            let options = Options {
                project,
                rescan,
                reprice,
            };
            let imported =
                import_noted(&mut record, &paths.host_status, &options, now_ms()).await?;
            drop(record);
            println!("{}", summary(&imported));
            Ok(())
        }
        HostAction::Status { json } => {
            let record = Record::open(&paths, access, home_repo, LOCK_WAIT).await?;
            if let Some(warning) = record.behind() {
                eprintln!("{warning}");
            }
            let status = Status {
                import: read_status(&paths.host_status)?,
                totals: totals(record.conn()).await?,
                transcripts: transcripts(record.conn()).await?,
            };
            drop(record);
            if json {
                println!("{}", serde_json::to_string_pretty(&status)?);
            } else {
                println!("{}", status_text(&status));
            }
            Ok(())
        }
    }
}

/// What the hook imports: its project, when configured, and whether to read again or price.
pub struct Request<'a> {
    pub project: Option<&'a Path>,
    pub rescan: bool,
    pub reprice: bool,
}

/// The host's project directory: `flag`, else `AGENT_BUS_HOST_PROJECT` when it is set and not
/// empty.
pub fn configured(flag: Option<PathBuf>) -> Option<PathBuf> {
    flag.or_else(|| {
        env::var_os(PROJECT_ENV)
            .filter(|project| !project.is_empty())
            .map(PathBuf::from)
    })
}

/// Run the `SessionEnd` hook with its `stdin`: catch the record at `paths` up through `access`
/// and import the project `request` names, else the transcript's. A record busy for longer
/// than `HOOK_LOCK_WAIT` is left to the command holding it.
pub async fn hook(
    stdin: &str,
    paths: &Paths,
    request: &Request<'_>,
    access: Result<BusAccess>,
    home_repo: Option<&str>,
    now_ms: u64,
) -> Exit {
    let fail = |error: anyhow::Error| {
        eprintln!("agent-bus host import --hook: {error:#}");
        Exit::Failed
    };
    let input = match read_input(stdin) {
        Ok(input) => input,
        Err(error) => return fail(error),
    };
    let project = match project(request.project, &input) {
        Ok(project) => project,
        Err(error) => return fail(error),
    };
    let mut record = match Record::open(paths, access, home_repo, HOOK_LOCK_WAIT).await {
        Ok(record) => record,
        Err(error) if error.downcast_ref::<Busy>().is_some() => return Exit::Done,
        Err(error) => return fail(error),
    };
    if let Some(warning) = record.behind() {
        eprintln!("{warning}");
    }
    let options = Options {
        project,
        rescan: request.rescan,
        reprice: request.reprice,
    };
    match import_noted(&mut record, &paths.host_status, &options, now_ms).await {
        Ok(_) => Exit::Done,
        Err(error) => fail(error),
    }
}

/// Import `options` into `record` at `now_ms`, and keep how it went in the status file at
/// `status`.
///
/// # Errors
///
/// Fails when the import fails, which is kept as the last error, or the status file cannot be
/// written.
pub async fn import_noted(
    record: &mut Record,
    status: &Path,
    options: &Options,
    now_ms: u64,
) -> Result<Imported> {
    let mut noted = read_status(status).unwrap_or_default();
    noted.project = Some(options.project.display().to_string());
    let result = import(record.conn_mut(), options, now_ms).await;
    match &result {
        Ok(imported) => {
            noted.last_success_ms = Some(now_ms);
            noted.last = Some(*imported);
        }
        Err(error) => {
            noted.last_error_ms = Some(now_ms);
            noted.last_error = Some(format!("{error:#}"));
        }
    }
    let text = toml::to_string(&noted).context("Unable to write the import's status")?;
    write_atomic(status, &text, PRIVATE_FILE_MODE)?;
    result
}

/// The status file at `path`; empty when there is none.
///
/// # Errors
///
/// Fails when it cannot be read or is not one.
pub fn read_status(path: &Path) -> Result<ImportStatus> {
    match fs::read_to_string(path) {
        Ok(text) => toml::from_str(&text)
            .with_context(|| format!("{} is not an import status", path.display())),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(ImportStatus::default()),
        Err(error) => Err(error).with_context(|| format!("Unable to read {}", path.display())),
    }
}

/// The warning to print when the last import failed after the last success; `None` otherwise.
pub fn failing(status: &ImportStatus) -> Option<String> {
    let error_ms = status.last_error_ms?;
    if status
        .last_success_ms
        .is_some_and(|success| success >= error_ms)
    {
        return None;
    }
    Some(format!(
        "The last import of the host's transcripts failed at {} UTC: {}",
        utc_minute(error_ms),
        status.last_error.as_deref().unwrap_or("unknown error")
    ))
}

/// The one line `host import` prints.
fn summary(imported: &Imported) -> String {
    let mut line = format!(
        "Imported {} responses from {} of {} transcripts",
        imported.responses, imported.read, imported.files
    );
    if imported.skipped > 0 {
        let _ = write!(line, ", {} lines not parsed", imported.skipped);
    }
    if imported.priced > 0 {
        let _ = write!(line, ", {} priced", imported.priced);
    }
    if imported.attributed > 0 {
        let _ = write!(line, ", {} moved to another item", imported.attributed);
    }
    let _ = write!(
        line,
        "; lines from {} UTC on",
        utc_minute(imported.installed_at_ms)
    );
    line
}

/// What `host status` prints as text.
fn status_text(status: &Status) -> String {
    let import = &status.import;
    let when =
        |ms: Option<u64>| ms.map_or_else(|| "never".to_owned(), |ms| utc_minute(ms) + " UTC");
    let mut lines = vec![
        format!(
            "Project: {}",
            import.project.as_deref().unwrap_or("not imported yet")
        ),
        format!("Last import: {}", when(import.last_success_ms)),
    ];
    if let Some(warning) = failing(import) {
        lines.push(warning);
    }
    let totals = &status.totals;
    lines.push(format!("Imported from: {}", when(totals.installed_at_ms)));
    lines.push(format!(
        "Host responses: {}, ${:.4} (not billed){}",
        totals.responses,
        totals.cost_usd,
        if totals.unpriced > 0 {
            format!(", {} unpriced", totals.unpriced)
        } else {
            String::new()
        }
    ));
    if !status.transcripts.is_empty() {
        let rows: Vec<[String; 5]> = status
            .transcripts
            .iter()
            .map(|transcript| {
                let name = Path::new(&transcript.path)
                    .file_name()
                    .map_or_else(String::new, |name| name.to_string_lossy().into_owned());
                [
                    name,
                    transcript.session.clone(),
                    transcript.offset.to_string(),
                    utc_minute(transcript.imported_at_ms),
                    transcript.skipped.to_string(),
                ]
            })
            .collect();
        lines.push(render_table(
            ["Transcript", "Session", "Offset", "Read (UTC)", "Skipped"],
            &rows,
            &TableLayout {
                title: Some("Transcripts"),
                right: &TRANSCRIPT_NUMBER_COLUMNS,
                ..TableLayout::default()
            },
        ));
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::time::{Duration, Instant};

    use serde_json::json;

    /// A wait that is over at once.
    const SHORT: Duration = Duration::from_millis(1);

    fn scratch(name: &str) -> PathBuf {
        let dir = env::temp_dir().join(format!("agent-bus-hook-{name}-{}", process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("project")).unwrap();
        dir
    }

    fn no_bus() -> Result<BusAccess> {
        bail!("no bus in this test")
    }

    fn stdin(dir: &Path) -> String {
        json!({
            "session_id": "s1",
            "transcript_path": dir.join("project/s1.jsonl"),
            "hook_event_name": "SessionEnd"
        })
        .to_string()
    }

    fn request() -> Request<'static> {
        Request {
            project: None,
            rescan: false,
            reprice: false,
        }
    }

    #[tokio::test]
    async fn the_hook_imports_and_exits_zero() {
        let dir = scratch("valid");
        let paths = Paths::in_dir(&dir.join("state"));
        let exit = hook(&stdin(&dir), &paths, &request(), no_bus(), None, 1_000).await;
        assert_eq!(exit, Exit::Done);
        let status = read_status(&paths.host_status).unwrap();
        assert_eq!(status.last_success_ms, Some(1_000));
        assert_eq!(status.project.as_deref(), dir.join("project").to_str());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[tokio::test]
    async fn empty_or_garbage_input_exits_one_and_imports_nothing() {
        let dir = scratch("garbage");
        let paths = Paths::in_dir(&dir.join("state"));
        for input in ["", "garbage", "{\"session_id\":\"s1\"}"] {
            let exit = hook(input, &paths, &request(), no_bus(), None, 1_000).await;
            assert_eq!(exit, Exit::Failed, "{input}");
            assert_ne!(exit.code(), 2);
        }
        assert!(!paths.record.exists());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[tokio::test]
    async fn a_missing_project_is_a_failure_and_recorded_as_the_last_error() {
        let dir = scratch("missing");
        let paths = Paths::in_dir(&dir.join("state"));
        let request = Request {
            project: Some(Path::new("/nonexistent/agent-bus-project")),
            ..request()
        };
        let exit = hook(&stdin(&dir), &paths, &request, no_bus(), None, 2_000).await;
        assert_eq!(exit, Exit::Failed);
        let status = read_status(&paths.host_status).unwrap();
        assert_eq!(status.last_error_ms, Some(2_000));
        assert!(failing(&status).is_some());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[tokio::test]
    async fn a_busy_record_is_left_to_its_holder_and_exits_zero() {
        let dir = scratch("busy");
        let paths = Paths::in_dir(&dir.join("state"));
        let held = Record::open(&paths, no_bus(), None, SHORT).await.unwrap();
        let started = Instant::now();
        let exit = hook(&stdin(&dir), &paths, &request(), no_bus(), None, 1_000).await;
        assert_eq!(exit, Exit::Done);
        assert!(started.elapsed() >= HOOK_LOCK_WAIT);
        assert!(started.elapsed() < LOCK_WAIT);
        drop(held);
        assert_eq!(
            read_status(&paths.host_status).unwrap(),
            ImportStatus::default()
        );
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_failure_after_the_last_success_is_warned_about() {
        let mut status = ImportStatus {
            last_success_ms: Some(10),
            ..ImportStatus::default()
        };
        assert_eq!(failing(&status), None);
        status.last_error_ms = Some(5);
        assert_eq!(failing(&status), None);
        status.last_error_ms = Some(20);
        status.last_error = Some("no such directory".to_owned());
        assert!(failing(&status).unwrap().contains("no such directory"));
    }
}
