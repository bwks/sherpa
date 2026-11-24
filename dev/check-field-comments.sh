#! /usr/bin/bash
#
# Refuse comments on struct fields and enum variants.
#
# A field or a variant is named for what it holds; it carries no comment. This script scans
# every tracked Rust file, including `crates/ebpf-redirect`, and fails on any comment inside the
# body of a struct (named or tuple) or an enum, the fields of an enum struct variant included:
# `///`, `//`, `/* */`, `/** */`, a trailing `//` after a field, and a `#[doc = ...]` attribute.
# Bodies are followed by bracket depth, so the comments of the type itself, of functions, impls
# and modules are not checked, and a comment above or between attributes is.
#
# Types whose `#[derive(...)]` holds `Parser`, `Subcommand`, `Args` or `ValueEnum` (bare or as
# `clap::...`) are not checked: clap builds the command line help from their doc comments.
#
# What is not a comment: text in a string, a raw string or a char literal, followed across
# lines, so a string that holds `//` is not read as a comment.
#
# Before it scans, the script checks itself on a sample that holds each kind of comment it must
# find and each kind it must not. `--self-test` runs that check alone.
#
# Run by `.githooks/pre-commit` and by the Lint job in `.github/workflows/pr.yml`.

set -euo pipefail

# The awk program is kept in a variable so it can use a single quote (char literals).
program="$(
    cat <<'EOF'
function reset_file() {
    in_block = 0
    in_str = 0
    in_raw = 0
    raw_end = ""
    depth = 0
    since = ""
}

# Whether the innermost open bracket is the body of a checked struct or enum.
function in_fields() {
    return depth > 0 && body[depth] && !skip[depth]
}

# Whether the derives in `text` make the type a clap type.
function is_clap(text) {
    return text ~ /derive[ \t]*\(([^)]*[^A-Za-z0-9_:])?(clap::)?(Parser|Subcommand|Args|ValueEnum)[ \t]*[,)]/
}

# Opens bracket `tok`, marking it as a checked body when it opens a struct or an enum, or sits
# inside one. `since` holds the code since the last `{`, `}` or `;`.
function open(tok) {
    depth++
    if (depth > 1 && body[depth - 1]) {
        body[depth] = 1
        skip[depth] = skip[depth - 1]
    } else if (tok == "{" && since ~ /(^|[^A-Za-z0-9_#])(struct|enum)[ \t]+[A-Za-z_]/) {
        body[depth] = 1
        skip[depth] = is_clap(since)
    } else if (tok == "(" && since ~ /(^|[^A-Za-z0-9_#])struct[ \t]+[A-Za-z_][A-Za-z0-9_]*[ \t]*(<.*>)?[ \t]*$/) {
        body[depth] = 1
        skip[depth] = is_clap(since)
    } else {
        body[depth] = 0
        skip[depth] = 0
    }
}

# Follows the brackets of code text `seg`, which holds no comment and no literal, and marks the
# line when it holds a `#[doc = ...]` attribute in a checked body.
function code(seg,   tok) {
    while (match(seg, /[][(){};#]/)) {
        tok = substr(seg, RSTART, 1)
        since = since substr(seg, 1, RSTART - 1)
        seg = substr(seg, RSTART)
        if (tok == "#" && in_fields() && seg ~ /^#[ \t]*\[[ \t]*doc[ \t]*(=|$)/) {
            hit = 1
        }
        seg = substr(seg, 2)
        if (tok == "{" || tok == "(" || tok == "[") {
            open(tok)
        } else if (tok == "}" || tok == ")" || tok == "]") {
            if (depth > 0) {
                depth--
            }
        }
        if (tok == "{" || tok == "}" || tok == ";") {
            since = ""
        } else {
            since = since tok
        }
    }
    since = since seg
}

# Splits one line into code, comments and literals, carrying the state of a string, a raw string
# or a block comment to the next line, and marks the line when a comment sits in a checked body.
function lex(rest,   p, tok, before) {
    while (rest != "") {
        if (in_block) {
            if (in_fields()) {
                hit = 1
            }
            p = index(rest, "*/")
            if (p == 0) {
                return
            }
            rest = substr(rest, p + 2)
            in_block = 0
        } else if (in_raw) {
            p = index(rest, raw_end)
            if (p == 0) {
                return
            }
            rest = substr(rest, p + length(raw_end))
            in_raw = 0
        } else if (in_str) {
            if (!match(rest, /^([^"\\]|\\.)*"/)) {
                return
            }
            rest = substr(rest, RLENGTH + 1)
            in_str = 0
        } else {
            if (!match(rest, /\/\/|\/\*|r#*"|"|'(\\.|[^\\'])'/)) {
                code(rest)
                return
            }
            tok = substr(rest, RSTART, RLENGTH)
            before = substr(rest, 1, RSTART - 1)
            rest = substr(rest, RSTART + RLENGTH)
            code(before)
            if (tok == "//") {
                if (in_fields()) {
                    hit = 1
                }
                return
            } else if (tok == "/*") {
                in_block = 1
            } else if (tok ~ /^r/) {
                in_raw = 1
                raw_end = "\"" substr(tok, 2, length(tok) - 2)
            } else if (tok == "\"") {
                in_str = 1
            }
        }
    }
}

FNR == 1 {
    reset_file()
}

{
    hit = 0
    lex($0)
    since = since " "
    if (hit) {
        line = $0
        sub(/^[ \t]+/, "", line)
        print FILENAME ":" FNR ": " line
    }
}
EOF
)"

# Prints "<file>:<line>: <line>" for each line of the given files with a comment on a field or
# a variant.
scan() {
    awk "${program}" "$@"
}

# Fails unless `scan` finds exactly the field and variant comments of the sample.
self_test() {
    local dir sample expected actual
    dir="$(mktemp -d)"
    sample="${dir}/sample.rs"
    cat >"${sample}" <<'EOF'
//! Module docs.

/// Type docs.
#[derive(Debug, Clone)]
pub struct Named {
    /// Field doc.
    pub a: u8,
    // Plain comment.
    b: u16,
    c: u32, // Trailing comment.
    /** Block doc. */
    d: u64,
    /// Doc above a multi-line attribute.
    #[serde(
        default,
        rename = "e"
    )]
    e: String,
    #[doc = "Doc attribute."]
    f: bool,
    #[serde(rename = "a//b")]
    g: &'static str,
}

/// Tuple type docs.
pub struct Tuple<T: Fn(u8)>(
    /// Tuple field doc.
    pub T,
    String, // Trailing comment.
);

/// Enum docs.
pub enum Kind {
    /// Variant doc.
    Unit,
    Tuple(u8), // Trailing comment.
    /* Block comment. */
    Struct {
        /// Variant field doc.
        x: u8,
    },
}

/// Command line docs.
#[derive(
    Debug,
    clap::Parser,
)]
pub struct Cli {
    /// Help text.
    verbose: bool,
}

#[derive(Subcommand)]
enum Command {
    /// Help text.
    Run {
        /// Help text.
        fast: bool,
    },
}

/// Impl docs.
impl Named {
    /// Function docs.
    fn new() -> Self {
        // Body comment.
        let brace = '{';
        /// Local type docs.
        struct Local {
            /// Local field doc.
            x: u8,
        }
        todo!() // Body comment.
    }
}
EOF
    expected="${sample}:6: /// Field doc.
${sample}:8: // Plain comment.
${sample}:10: c: u32, // Trailing comment.
${sample}:11: /** Block doc. */
${sample}:13: /// Doc above a multi-line attribute.
${sample}:19: #[doc = \"Doc attribute.\"]
${sample}:27: /// Tuple field doc.
${sample}:29: String, // Trailing comment.
${sample}:34: /// Variant doc.
${sample}:36: Tuple(u8), // Trailing comment.
${sample}:37: /* Block comment. */
${sample}:39: /// Variant field doc.
${sample}:71: /// Local field doc."
    actual="$(scan "${sample}")"
    rm -rf "${dir}"
    if [[ "${actual}" != "${expected}" ]]; then
        echo "check-field-comments: self-test failed" >&2
        echo "expected:" >&2
        echo "${expected}" >&2
        echo "found:" >&2
        echo "${actual}" >&2
        return 1
    fi
}

self_test
if [[ "${1:-}" == "--self-test" ]]; then
    exit 0
fi

cd "$(git rev-parse --show-toplevel)"

mapfile -t files < <(git ls-files -- '*.rs')

if [[ ${#files[@]} -eq 0 ]]; then
    echo "check-field-comments: no Rust files found" >&2
    exit 1
fi

result="$(scan "${files[@]}")"

if [[ -z "${result}" ]]; then
    exit 0
fi

while read -r finding; do
    echo "${finding}: comment on a struct field or an enum variant" >&2
done <<<"${result}"
cat >&2 <<'EOF'

Struct fields and enum variants carry no comments. Name the field or the variant for what it
holds instead. Types deriving clap's `Parser`, `Subcommand`, `Args` or `ValueEnum` keep their
doc comments, because clap builds the help text from them.
EOF
exit 1
