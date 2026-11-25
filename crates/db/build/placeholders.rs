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
use std::num::ParseIntError;
use std::str::CharIndices;

use thiserror::Error;

/// Why a query file's placeholders give no parameter count. Each byte offset is where the
/// refused placeholder, literal, identifier or comment starts.
#[derive(Debug, Error, PartialEq, Eq)]
pub(crate) enum PlaceholderError {
    #[error("unterminated comment at byte {start}")]
    UnterminatedComment { start: usize },
    #[error("unterminated quote at byte {start}")]
    UnterminatedQuote { start: usize },
    #[error("bare `?` at byte {start}: number every parameter, `?1`, `?2`, ...")]
    Bare { start: usize },
    #[error("`?0` at byte {start}: parameters are numbered from `?1`")]
    Zero { start: usize },
    #[error(transparent)]
    Number(#[from] ParseIntError),
    #[error("named parameter `{sigil}` at byte {start}: number every parameter, `?1`, `?2`, ...")]
    Named { sigil: char, start: usize },
    #[error("parameters go up to ?{count} but skip {}", numbered(missing))]
    Gap { count: usize, missing: Vec<usize> },
}

/// The number of parameters `sql` binds: its highest `?N`.
///
/// # Errors
///
/// Fails on a bare `?`, a named parameter, `?0`, a gap in the numbering, or an unterminated
/// literal, identifier or comment.
pub(crate) fn parameter_count(sql: &str) -> Result<usize, PlaceholderError> {
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
                    return Err(PlaceholderError::UnterminatedComment { start });
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
                    return Err(PlaceholderError::Bare { start });
                }
                let number: usize = digits.parse()?;
                if number == 0 {
                    return Err(PlaceholderError::Zero { start });
                }
                numbers.insert(number);
            }
            ':' | '@' | '$' => return Err(PlaceholderError::Named { sigil: c, start }),
            _ => {}
        }
    }
    let count = numbers.last().copied().unwrap_or_default();
    if numbers.len() != count {
        let missing = (1..=count)
            .filter(|number| !numbers.contains(number))
            .collect();
        return Err(PlaceholderError::Gap { count, missing });
    }
    Ok(count)
}

/// `numbers` as placeholders, `?2, ?4`.
fn numbered(numbers: &[usize]) -> String {
    numbers
        .iter()
        .map(|number| format!("?{number}"))
        .collect::<Vec<_>>()
        .join(", ")
}

/// Skip to the `close` that ends a literal or identifier opened at byte `start`. A doubled
/// `close` inside is an escaped one.
fn skip_quoted(
    chars: &mut Peekable<CharIndices<'_>>,
    close: char,
    start: usize,
) -> Result<(), PlaceholderError> {
    while let Some((_, c)) = chars.next() {
        if c == close {
            if close != ']' && chars.peek().is_some_and(|&(_, next)| next == close) {
                chars.next();
                continue;
            }
            return Ok(());
        }
    }
    Err(PlaceholderError::UnterminatedQuote { start })
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert_eq!(
            parameter_count("SELECT ?1, ?"),
            Err(PlaceholderError::Bare { start: 11 })
        );
    }

    #[test]
    fn named_parameters_are_refused() {
        for (sql, sigil) in [
            ("SELECT * FROM t WHERE a = :a", ':'),
            ("SELECT * FROM t WHERE a = @a", '@'),
            ("SELECT * FROM t WHERE a = $a", '$'),
        ] {
            assert_eq!(
                parameter_count(sql),
                Err(PlaceholderError::Named { sigil, start: 26 }),
                "{sql}"
            );
        }
    }

    #[test]
    fn question_zero_is_refused() {
        assert_eq!(
            parameter_count("SELECT ?0"),
            Err(PlaceholderError::Zero { start: 7 })
        );
    }

    #[test]
    fn a_gap_in_the_numbering_is_refused() {
        assert_eq!(
            parameter_count("SELECT ?1, ?3, ?5"),
            Err(PlaceholderError::Gap {
                count: 5,
                missing: vec![2, 4]
            })
        );
    }

    #[test]
    fn placeholders_in_literals_identifiers_and_comments_are_not_parameters() {
        let sql = r#"-- find by ?9 or :name
SELECT '?2 '' :x', "col?3", [a?4], `b@c` /* ?5 $y */ FROM t WHERE id = ?1"#;
        assert_eq!(parameter_count(sql).unwrap(), 1);
    }

    #[test]
    fn an_unterminated_literal_or_comment_is_refused() {
        assert_eq!(
            parameter_count("SELECT 'open"),
            Err(PlaceholderError::UnterminatedQuote { start: 7 })
        );
        assert_eq!(
            parameter_count("SELECT 1 /* open"),
            Err(PlaceholderError::UnterminatedComment { start: 9 })
        );
    }
}
