//! The bound parameters of a query file, counted from its placeholders.
//!
//! `build.rs` includes this file to give each generated query its parameter count; the crate
//! includes it under `cfg(test)` so `cargo test -p db` runs the tests below. Turso's public
//! `Statement` has no parameter count, so the count is read from the SQL text.
//!
//! Every query numbers its parameters: `?1`, `?2`, and so on, each number one or more times. The
//! count is the highest number. A bare `?`, a named parameter (`:name`, `@name`, `$name`), `?0`,
//! or a gap in the numbering is refused, so the highest number is always the count. Placeholders
//! inside string literals, quoted identifiers and comments are not parameters and are skipped.

use std::collections::BTreeSet;
use std::iter::Peekable;
use std::str::CharIndices;

use anyhow::{Result, bail};

/// The number of parameters `sql` binds: its highest `?N`.
///
/// # Errors
///
/// Fails on a bare `?`, a named parameter, `?0`, a gap in the numbering, or an unterminated
/// literal, identifier or comment.
pub(crate) fn parameter_count(sql: &str) -> Result<usize> {
    let mut numbers = BTreeSet::new();
    let mut chars = sql.char_indices().peekable();
    while let Some((start, c)) = chars.next() {
        match c {
            '\'' | '"' | '`' => skip_quoted(&mut chars, c, start)?,
            '[' => skip_quoted(&mut chars, ']', start)?,
            '-' if chars.peek().is_some_and(|&(_, next)| next == '-') => {
                for (_, skipped) in chars.by_ref() {
                    if skipped == '\n' {
                        break;
                    }
                }
            }
            '/' if chars.peek().is_some_and(|&(_, next)| next == '*') => {
                chars.next();
                let mut previous = ' ';
                let mut closed = false;
                for (_, skipped) in chars.by_ref() {
                    if previous == '*' && skipped == '/' {
                        closed = true;
                        break;
                    }
                    previous = skipped;
                }
                if !closed {
                    bail!("unterminated comment at byte {start}");
                }
            }
            '?' => {
                let mut digits = String::new();
                while let Some(&(_, next)) = chars.peek() {
                    if !next.is_ascii_digit() {
                        break;
                    }
                    digits.push(next);
                    chars.next();
                }
                if digits.is_empty() {
                    bail!("bare `?` at byte {start}: number every parameter, `?1`, `?2`, ...");
                }
                let number: usize = digits.parse()?;
                if number == 0 {
                    bail!("`?0` at byte {start}: parameters are numbered from `?1`");
                }
                numbers.insert(number);
            }
            ':' | '@' | '$' => {
                bail!(
                    "named parameter `{c}` at byte {start}: number every parameter, `?1`, `?2`, ..."
                )
            }
            _ => {}
        }
    }
    let count = numbers.last().copied().unwrap_or_default();
    if numbers.len() != count {
        let missing: Vec<String> = (1..=count)
            .filter(|number| !numbers.contains(number))
            .map(|number| format!("?{number}"))
            .collect();
        bail!(
            "parameters go up to ?{count} but skip {}",
            missing.join(", ")
        );
    }
    Ok(count)
}

/// Skip to the `close` that ends a literal or identifier opened at byte `start`. A doubled
/// `close` inside is an escaped one.
fn skip_quoted(chars: &mut Peekable<CharIndices<'_>>, close: char, start: usize) -> Result<()> {
    while let Some((_, c)) = chars.next() {
        if c == close {
            if close != ']' && chars.peek().is_some_and(|&(_, next)| next == close) {
                chars.next();
                continue;
            }
            return Ok(());
        }
    }
    bail!("unterminated quote at byte {start}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn refusal(sql: &str) -> String {
        format!("{:#}", parameter_count(sql).unwrap_err())
    }

    #[test]
    fn a_statement_without_parameters_takes_none() {
        assert_eq!(parameter_count("SELECT id FROM lab").unwrap(), 0);
    }

    #[test]
    fn the_count_is_the_highest_number() {
        assert_eq!(
            parameter_count("UPDATE t SET a = ?2, b = ?3 WHERE id = ?1").unwrap(),
            3
        );
    }

    #[test]
    fn a_number_used_twice_counts_once() {
        assert_eq!(
            parameter_count("SELECT ?1 WHERE a = ?1 OR b = ?2").unwrap(),
            2
        );
    }

    #[test]
    fn a_bare_placeholder_is_refused() {
        assert!(refusal("SELECT ?1, ?").contains("bare `?`"));
    }

    #[test]
    fn named_parameters_are_refused() {
        for sql in [
            "SELECT * FROM t WHERE a = :a",
            "SELECT * FROM t WHERE a = @a",
            "SELECT * FROM t WHERE a = $a",
        ] {
            assert!(refusal(sql).contains("named parameter"), "{sql}");
        }
    }

    #[test]
    fn question_zero_is_refused() {
        assert!(refusal("SELECT ?0").contains("`?0`"));
    }

    #[test]
    fn a_gap_in_the_numbering_is_refused() {
        let message = refusal("SELECT ?1, ?3, ?5");
        assert!(message.contains("skip ?2, ?4"), "{message}");
    }

    #[test]
    fn placeholders_in_literals_identifiers_and_comments_are_not_parameters() {
        let sql = "-- find by ?9 or :name\n\
                   SELECT '?2 '' :x', \"col?3\", [a?4], `b@c` /* ?5 $y */ FROM t WHERE id = ?1";
        assert_eq!(parameter_count(sql).unwrap(), 1);
    }

    #[test]
    fn an_unterminated_literal_or_comment_is_refused() {
        assert!(refusal("SELECT 'open").contains("unterminated quote"));
        assert!(refusal("SELECT 1 /* open").contains("unterminated comment"));
    }
}
