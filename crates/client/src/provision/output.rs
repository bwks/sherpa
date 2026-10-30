//! Output from nodes provisioned at once: every line starts with its node's name, so lines
//! from different nodes can be told apart, and each line is written whole, so lines from
//! different nodes never mix.
use std::io::{self, Write};

use anyhow::{Context, Result};

/// The lines shown for one node, with what the node has sent of a line it has not ended.
pub struct NodeOutput {
    prefix: String,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

impl NodeOutput {
    /// Output for `node`, its name padded to `width` so the lines of every node line up.
    pub fn new(node: &str, width: usize) -> Self {
        Self {
            prefix: line_prefix(node, width),
            stdout: Vec::new(),
            stderr: Vec::new(),
        }
    }

    /// Show a line about the provisioner's progress, such as a file being copied.
    pub fn status(&self, text: &str) -> Result<()> {
        let mut line = Vec::new();
        end_line(&self.prefix, &mut text.as_bytes().to_vec(), &mut line);
        write_whole(&mut io::stdout().lock(), &line)
    }

    /// Show the complete lines in `bytes`, from a command's stdout.
    pub fn stdout(&mut self, bytes: &[u8]) -> Result<()> {
        let mut lines = Vec::new();
        prefix_lines(&self.prefix, &mut self.stdout, bytes, &mut lines);
        write_whole(&mut io::stdout().lock(), &lines)
    }

    /// Show the complete lines in `bytes`, from a command's stderr.
    pub fn stderr(&mut self, bytes: &[u8]) -> Result<()> {
        let mut lines = Vec::new();
        prefix_lines(&self.prefix, &mut self.stderr, bytes, &mut lines);
        write_whole(&mut io::stderr().lock(), &lines)
    }

    /// Show the last line of a command that ended without a newline.
    pub fn finish(&mut self) -> Result<()> {
        let mut line = Vec::new();
        end_line(&self.prefix, &mut self.stdout, &mut line);
        write_whole(&mut io::stdout().lock(), &line)?;
        line.clear();
        end_line(&self.prefix, &mut self.stderr, &mut line);
        write_whole(&mut io::stderr().lock(), &line)
    }
}

/// `node`, padded to `width`, and a separator.
fn line_prefix(node: &str, width: usize) -> String {
    format!("{node:<width$} | ")
}

/// Append to `out` each line that `bytes` ends, after what `pending` holds of it, with
/// `prefix` before it. What follows the last newline is kept in `pending`.
fn prefix_lines(prefix: &str, pending: &mut Vec<u8>, bytes: &[u8], out: &mut Vec<u8>) {
    for &byte in bytes {
        pending.push(byte);
        if byte == b'\n' {
            out.extend_from_slice(prefix.as_bytes());
            out.append(pending);
        }
    }
}

/// Append to `out` the line `pending` holds, with `prefix` and a newline, if there is one.
fn end_line(prefix: &str, pending: &mut Vec<u8>, out: &mut Vec<u8>) {
    if pending.is_empty() {
        return;
    }
    out.extend_from_slice(prefix.as_bytes());
    out.append(pending);
    out.push(b'\n');
}

/// Write `bytes` in one go and flush them, so another node's line cannot land in between.
fn write_whole(out: &mut impl Write, bytes: &[u8]) -> Result<()> {
    if bytes.is_empty() {
        return Ok(());
    }
    out.write_all(bytes)
        .context("Cannot show the provisioner's output")?;
    out.flush().context("Cannot show the provisioner's output")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn prefixed(pending: &mut Vec<u8>, bytes: &str) -> String {
        let mut out = Vec::new();
        prefix_lines("web | ", pending, bytes.as_bytes(), &mut out);
        String::from_utf8(out).unwrap()
    }

    #[test]
    fn node_names_are_padded_so_lines_line_up() {
        assert_eq!(line_prefix("db", 5), "db    | ");
        assert_eq!(line_prefix("web01", 5), "web01 | ");
    }

    #[test]
    fn each_complete_line_gets_the_prefix() {
        let mut pending = Vec::new();
        assert_eq!(
            prefixed(&mut pending, "one\ntwo\n"),
            "web | one\nweb | two\n"
        );
        assert!(pending.is_empty());
    }

    #[test]
    fn a_line_split_across_reads_is_shown_once_it_ends() {
        let mut pending = Vec::new();
        assert_eq!(prefixed(&mut pending, "one\ntw"), "web | one\n");
        assert_eq!(prefixed(&mut pending, "o"), "");
        assert_eq!(
            prefixed(&mut pending, "\nthree\n"),
            "web | two\nweb | three\n"
        );
    }

    #[test]
    fn an_unended_last_line_is_shown_at_the_end() {
        let mut pending = Vec::new();
        assert_eq!(prefixed(&mut pending, "done"), "");
        let mut out = Vec::new();
        end_line("web | ", &mut pending, &mut out);
        assert_eq!(String::from_utf8(out).unwrap(), "web | done\n");
        assert!(pending.is_empty());

        let mut out = Vec::new();
        end_line("web | ", &mut pending, &mut out);
        assert!(out.is_empty());
    }
}
