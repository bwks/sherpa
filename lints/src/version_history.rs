//! The `version_history` lint.

use clippy_utils::diagnostics::span_lint_and_help;
use clippy_utils::tokenize_with_text;
use rustc_ast::Crate;
use rustc_lexer::{LiteralKind, TokenKind};
use rustc_lint::{EarlyContext, EarlyLintPass, LintContext};
use rustc_session::{declare_lint, declare_lint_pass};
use rustc_span::def_id::LOCAL_CRATE;
use rustc_span::{BytePos, FileName, Span};

/// Phrases that tell what the code was instead of what it is.
const PHRASES: &[&str] = &[
    "no longer",
    "previously",
    "formerly",
    "used to",
    "any more",
    "anymore",
    "deprecated",
    "legacy",
    "originally",
    "backward compat",
    "backwards compat",
    "was renamed",
    "were renamed",
    "been renamed",
    "renamed from",
    "renamed in",
    "was removed",
    "were removed",
    "been removed",
    "removed in",
    "added in",
    "introduced in",
    "older version",
    "old version",
    "earlier version",
    "old name",
    "old format",
];

/// Words before `used to` that make it mean "serves to", not "once did".
const USED_TO_VERBS: &[&str] = &[
    "is", "are", "be", "been", "being", "was", "were", "get", "gets",
];

/// Words that turn a version number after them into history, as in `since 0.7.0`.
const VERSION_WORDS: &[&str] = &[
    "since", "before", "until", "after", "in", "from", "as of", "prior to",
];

/// The version words after which a two-part number (`before 0.7`) is a version too. After the
/// others it is more often a quantity (`until 2.5 seconds`).
const SHORT_VERSION_WORDS: &[&str] = &["since", "before", "as of", "prior to"];

declare_lint! {
    /// Refuses version-history narrative in a comment, a doc comment or a string literal: a
    /// phrase such as `no longer`, `previously` or `used to`, or a version number after `since`,
    /// `before`, `in` and the like. Code, comments and user-facing text describe current
    /// behaviour; history belongs in `CHANGELOG.md`.
    ///
    /// Every Rust file of the crate is read as tokens, so a phrase in an identifier is not
    /// reported, and text in a macro's input (`#[error("...")]`, `println!`) is.
    pub VERSION_HISTORY,
    Warn,
    "version-history narrative in a comment or a string literal"
}

declare_lint_pass!(VersionHistory => [VERSION_HISTORY]);

impl EarlyLintPass for VersionHistory {
    fn check_crate(&mut self, cx: &EarlyContext<'_>, _: &Crate) {
        for file in cx.sess().source_map().files().iter() {
            if file.cnum != LOCAL_CRATE {
                continue;
            }
            let FileName::Real(name) = &file.name else {
                continue;
            };
            let Some(path) = name.local_path() else {
                continue;
            };
            if path.extension().is_none_or(|extension| extension != "rs")
                || path.components().any(|part| part.as_os_str() == "target")
            {
                continue;
            }
            let Some(source) = &file.src else {
                continue;
            };
            for (kind, text, range) in tokenize_with_text(source) {
                let place = match kind {
                    TokenKind::LineComment { .. } | TokenKind::BlockComment { .. } => "comment",
                    TokenKind::Literal {
                        kind:
                            LiteralKind::Str { .. }
                            | LiteralKind::RawStr { .. }
                            | LiteralKind::ByteStr { .. }
                            | LiteralKind::RawByteStr { .. }
                            | LiteralKind::CStr { .. }
                            | LiteralKind::RawCStr { .. },
                        ..
                    } => "string literal",
                    _ => continue,
                };
                let Some((start, len)) = find_history(text) else {
                    continue;
                };
                let lo = file.start_pos + BytePos(u32::try_from(range.start + start).unwrap_or(0));
                let hi = lo + BytePos(u32::try_from(len).unwrap_or(0));
                span_lint_and_help(
                    cx,
                    VERSION_HISTORY,
                    Span::with_root_ctxt(lo, hi),
                    format!("version-history narrative in a {place}"),
                    None,
                    "describe what the code does today; history belongs in CHANGELOG.md",
                );
            }
        }
    }
}

/// The byte offset and length of the first piece of history narrative in `text`.
fn find_history(text: &str) -> Option<(usize, usize)> {
    let lower = text.to_ascii_lowercase();
    let phrase = PHRASES
        .iter()
        .filter_map(|phrase| find_phrase(&lower, phrase).map(|start| (start, phrase.len())))
        .min();
    let version = VERSION_WORDS
        .iter()
        .filter_map(|word| find_version_after(&lower, word))
        .min();
    match (phrase, version) {
        (Some(phrase), Some(version)) => Some(phrase.min(version)),
        (found, None) | (None, found) => found,
    }
}

/// The offset of `phrase` in `lower` as whole words, skipping `used to` where it means
/// "serves to".
fn find_phrase(lower: &str, phrase: &str) -> Option<usize> {
    lower
        .match_indices(phrase)
        .map(|(start, _)| start)
        .find(|&start| {
            let before = &lower[..start];
            let after = &lower[start + phrase.len()..];
            if before.ends_with(is_word_char) || after.starts_with(is_word_char) {
                return false;
            }
            if phrase != "used to" {
                return true;
            }
            let previous = before.split_whitespace().next_back().unwrap_or_default();
            !USED_TO_VERBS.contains(&previous) && !previous.ends_with("n't")
        })
}

/// The offset and length of `word` followed by a version number (`0.7`, `v0.7.0`), where the
/// number is not the start of an IP address.
fn find_version_after(lower: &str, word: &str) -> Option<(usize, usize)> {
    lower.match_indices(word).find_map(|(start, _)| {
        if lower[..start].ends_with(is_word_char) {
            return None;
        }
        let rest = lower[start + word.len()..].strip_prefix(' ')?;
        let rest = rest.strip_prefix("version ").unwrap_or(rest);
        let rest = rest.strip_prefix("sherpa ").unwrap_or(rest);
        let number = rest.strip_prefix('v').unwrap_or(rest);
        let end = number
            .find(|c: char| !c.is_ascii_digit() && c != '.')
            .unwrap_or(number.len());
        let tail = &number[end..];
        let number = number[..end].trim_end_matches('.');
        let parts: Vec<&str> = number.split('.').collect();
        let min_parts = if SHORT_VERSION_WORDS.contains(&word) {
            2
        } else {
            3
        };
        let is_version = (min_parts..=3).contains(&parts.len())
            && parts.iter().all(|part| !part.is_empty())
            && !tail.starts_with('/')
            && !tail.starts_with(is_word_char);
        is_version.then(|| {
            let len = lower.len() - start - tail.len();
            (start, len)
        })
    })
}

/// Whether `c` can be part of a word or an identifier.
fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}
