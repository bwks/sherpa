//! `agent-bus listen`: print each message sent to an agent as one JSON line, and keep a journal
//! of them.
//!
//! The journal, `listen-<agent>.jsonl` in the state directory, gets each message before it is
//! printed and acknowledged, so a message the bus no longer delivers is in the journal even if
//! whoever read the listener's output was gone when it was printed. `--since` prints the
//! journal's recent entries first, so a restarted listener catches up. Each line is
//! `{"at_ms":<when it was received>,"message":<the message as printed>}`; the journal is pruned
//! to the stream's retention, like `results.jsonl`.

use std::fs;
use std::io::{self, ErrorKind, Write};
use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use crate::channel::{Channel, Endpoint};
use crate::files::{JsonLines, millis, now_ms, standard_state_dir};
use crate::message::{Message, check_agent_name};

/// One line of the journal.
#[derive(Debug, Serialize, Deserialize)]
struct Entry {
    /// When the listener received the message, in milliseconds since the Unix epoch.
    at_ms: u64,
    message: Message,
}

/// The messages a listener received, in the order it received them.
#[derive(Debug)]
pub struct Journal {
    lines: JsonLines,
}

impl Journal {
    pub fn new(path: PathBuf) -> Self {
        Self {
            lines: JsonLines::new(path),
        }
    }

    /// The journal of `agent`'s listener in the state directory.
    ///
    /// # Errors
    ///
    /// Fails when the name is not an agent name, or there is no state directory.
    pub fn standard(agent: &str) -> Result<Self> {
        check_agent_name(agent)?;
        Ok(Self::new(
            standard_state_dir()?.join(format!("listen-{agent}.jsonl")),
        ))
    }

    /// Append `message`, received at `now_ms`. It is in the file when this returns.
    ///
    /// # Errors
    ///
    /// Fails when the journal cannot be written.
    pub fn record(&mut self, message: &Message, now_ms: u64) -> Result<()> {
        let line = serde_json::to_string(&Entry {
            at_ms: now_ms,
            message: message.clone(),
        })
        .context("Unable to render the journal entry")?;
        self.lines.append(&line, now_ms).with_context(|| {
            format!(
                "Unable to keep the message in the journal {}: not printing or acknowledging it",
                self.lines.path().display()
            )
        })
    }

    /// Drop the entries older than the stream's retention; how many were dropped.
    ///
    /// # Errors
    ///
    /// Fails when the journal cannot be read or rewritten.
    pub fn prune(&mut self, now_ms: u64) -> Result<usize> {
        self.lines.prune(now_ms)
    }

    /// The messages received at or after `cutoff_ms`, oldest first. A missing journal has none;
    /// a line that is not an entry is reported and skipped.
    ///
    /// # Errors
    ///
    /// Fails when the journal exists and cannot be read.
    pub fn since(&self, cutoff_ms: u64) -> Result<Vec<Message>> {
        let path = self.lines.path();
        let text = match fs::read_to_string(path) {
            Ok(text) => text,
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => {
                return Err(error).with_context(|| format!("Unable to read {}", path.display()));
            }
        };
        let mut messages = Vec::new();
        for (number, line) in text.lines().enumerate() {
            match serde_json::from_str::<Entry>(line) {
                Ok(entry) if entry.at_ms >= cutoff_ms => messages.push(entry.message),
                Ok(_) => {}
                Err(error) => eprintln!(
                    "Skipping line {} of {}: {error}",
                    number + 1,
                    path.display()
                ),
            }
        }
        Ok(messages)
    }
}

/// Parse a `--since` duration: a whole number and a unit, `s`, `m`, `h` or `d`, such as `30m`.
///
/// # Errors
///
/// Fails when the text is not such a duration.
pub fn parse_since(text: &str) -> Result<Duration> {
    let Some(unit) = text.chars().last() else {
        bail!("Give a duration such as 30m");
    };
    let seconds: u64 = match unit {
        's' => 1,
        'm' => 60,
        'h' => 60 * 60,
        'd' => 24 * 60 * 60,
        _ => bail!("'{text}' has no unit: give a number and s, m, h or d, such as 30m"),
    };
    let number = &text[..text.len() - unit.len_utf8()];
    if number.is_empty() || !number.bytes().all(|byte| byte.is_ascii_digit()) {
        bail!("'{text}' is not a whole number and a unit, such as 30m");
    }
    let count: u64 = number
        .parse()
        .with_context(|| format!("'{text}' is too long a duration"))?;
    let total = count
        .checked_mul(seconds)
        .with_context(|| format!("'{text}' is too long a duration"))?;
    Ok(Duration::from_secs(total))
}

/// Print each message sent to `agent` as one JSON line, until the bus closes. Each is kept in
/// the journal first, then printed, then acknowledged. With `since`, the journal's entries from
/// that long ago are printed first.
///
/// # Errors
///
/// Fails when the inbox cannot be opened or read, or a message cannot be journaled, printed or
/// acknowledged. A message that is not acknowledged is delivered again to the next listener.
pub async fn listen(
    server: &Endpoint,
    seed: &str,
    agent: &str,
    since: Option<Duration>,
) -> Result<()> {
    let journal = Journal::standard(agent)?;
    let channel = Channel::connect(server, seed).await?;
    serve(&channel, agent, journal, since, &mut io::stdout()).await
}

/// `listen`, on `channel`, keeping `journal` and printing to `out`.
///
/// # Errors
///
/// As `listen`.
pub async fn serve(
    channel: &Channel,
    agent: &str,
    mut journal: Journal,
    since: Option<Duration>,
    out: &mut impl Write,
) -> Result<()> {
    let mut inbox = channel.inbox(agent).await?;
    let now = now_ms();
    match journal.prune(now) {
        Ok(0) => {}
        Ok(dropped) => eprintln!("Pruned {dropped} old journal entries"),
        Err(error) => eprintln!("{error:#}"),
    }
    if let Some(since) = since {
        for message in journal.since(now.saturating_sub(millis(since)))? {
            print(out, &message)?;
        }
    }
    while let Some(delivery) = inbox.next().await? {
        relay(&mut journal, &delivery.message, out, now_ms())?;
        delivery.ack().await?;
    }
    Ok(())
}

/// Keep `message` in the journal, then print it. When the journal cannot be written nothing is
/// printed, and the caller must not acknowledge the message.
fn relay(
    journal: &mut Journal,
    message: &Message,
    out: &mut impl Write,
    now_ms: u64,
) -> Result<()> {
    journal.record(message, now_ms)?;
    print(out, message)
}

/// Print `message` as one JSON line, flushed.
fn print(out: &mut impl Write, message: &Message) -> Result<()> {
    let line = serde_json::to_string(message).context("Unable to render the message")?;
    writeln!(out, "{line}")
        .and_then(|()| out.flush())
        .context("Unable to print the message")
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::path::Path;

    use crate::message::Kind;

    const NOW: u64 = 1_800_000_000_000;
    const MINUTE: u64 = 60 * 1000;
    const DAY: u64 = 24 * 60 * MINUTE;

    fn scratch(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("agent-bus-listen-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    fn message(text: &str) -> Message {
        Message {
            task: "1a2b3c4d".to_owned(),
            from: "dev".to_owned(),
            to: "host".to_owned(),
            kind: Kind::Completed,
            text: text.to_owned(),
            usage: None,
            task_usage: None,
            github: false,
            model: None,
        }
    }

    /// Stdout, checking on each write that the journal already holds the message.
    struct Checked<'a> {
        journal: &'a Path,
        printed: Vec<u8>,
    }

    impl Write for Checked<'_> {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            let journal = fs::read_to_string(self.journal)?;
            let text = String::from_utf8_lossy(buf);
            let line = text.trim_end();
            if !line.is_empty() {
                assert!(journal.contains(line), "printed before journaled: {line}");
            }
            self.printed.extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn a_message_is_journaled_before_it_is_printed() {
        let dir = scratch("order");
        let path = dir.join("state").join("listen-host.jsonl");
        let mut journal = Journal::new(path.clone());
        let mut out = Checked {
            journal: &path,
            printed: Vec::new(),
        };
        relay(&mut journal, &message("done"), &mut out, NOW).unwrap();

        let printed = String::from_utf8(out.printed).unwrap();
        let expected = serde_json::to_string(&message("done")).unwrap();
        assert_eq!(printed, format!("{expected}\n"));
        // The journal line is the printed message with the time it came.
        let entry: Entry = serde_json::from_str(fs::read_to_string(&path).unwrap().trim()).unwrap();
        assert_eq!(entry.at_ms, NOW);
        assert_eq!(entry.message, message("done"));
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_message_that_cannot_be_journaled_is_not_printed() {
        let dir = scratch("unwritable");
        fs::create_dir_all(&dir).unwrap();
        // The journal's directory is a file, so the journal cannot be written.
        fs::write(dir.join("state"), "").unwrap();
        let mut journal = Journal::new(dir.join("state").join("listen-host.jsonl"));
        let mut out = Vec::new();
        let error = relay(&mut journal, &message("done"), &mut out, NOW).unwrap_err();
        assert!(format!("{error:#}").contains("not printing"), "{error:#}");
        assert_eq!(out, Vec::<u8>::new());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn since_gives_the_entries_from_that_long_ago_in_order() {
        let dir = scratch("since");
        let path = dir.join("listen-host.jsonl");
        let mut journal = Journal::new(path.clone());
        assert_eq!(journal.since(0).unwrap(), Vec::<Message>::new());
        journal
            .record(&message("an hour ago"), NOW - 60 * MINUTE)
            .unwrap();
        journal
            .record(&message("20 min ago"), NOW - 20 * MINUTE)
            .unwrap();
        journal.record(&message("now"), NOW).unwrap();
        // A broken line is skipped, not fatal.
        let mut text = fs::read_to_string(&path).unwrap();
        text.push_str("{\"at_ms\":\n");
        fs::write(&path, text).unwrap();

        let since = millis(parse_since("30m").unwrap());
        let texts: Vec<String> = journal
            .since(NOW - since)
            .unwrap()
            .into_iter()
            .map(|message| message.text)
            .collect();
        assert_eq!(texts, ["20 min ago", "now"]);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_journal_is_pruned_to_the_retention() {
        let dir = scratch("prune");
        let mut journal = Journal::new(dir.join("listen-host.jsonl"));
        journal.record(&message("old"), NOW - 8 * DAY).unwrap();
        journal.record(&message("new"), NOW - DAY).unwrap();
        assert_eq!(journal.prune(NOW).unwrap(), 1);
        let texts: Vec<String> = journal
            .since(0)
            .unwrap()
            .into_iter()
            .map(|message| message.text)
            .collect();
        assert_eq!(texts, ["new"]);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_journal_is_named_after_a_valid_agent() {
        assert!(Journal::standard("../etc").is_err());
    }

    #[test]
    fn since_takes_a_number_and_a_unit() {
        assert_eq!(parse_since("90s").unwrap(), Duration::from_secs(90));
        assert_eq!(parse_since("30m").unwrap(), Duration::from_mins(30));
        assert_eq!(parse_since("2h").unwrap(), Duration::from_hours(2));
        assert_eq!(parse_since("7d").unwrap(), Duration::from_hours(7 * 24));
        for bad in [
            "",
            "30",
            "m",
            "-5m",
            "1.5h",
            "30 m",
            "30x",
            "99999999999999999999d",
        ] {
            assert!(parse_since(bad).is_err(), "{bad}");
        }
    }
}
