#! /usr/bin/bash
#
# Refuse GitHub issue and pull request numbers in comments.
#
# A comment says why the code is as it is; it does not point at the tracker, whose numbers say
# nothing to a reader of the code and outlive what they named. This script scans the comments
# of every tracked Rust file, including `crates/ebpf-redirect`, and fails on a reference:
# `#123`, `issue 123`, `PR 123`, `pull request 123` (each also with `#` before the number), and
# a `github.com/<owner>/<repo>/issues/123` or `.../pull/123` link. A reference fails even when
# the code holds the same number: a comment that describes fixture data names it in words.
#
# What is not a reference:
# - Text in a string, a raw string or a char literal, followed across lines, so a string that
#   holds `//` is not read as a comment.
# - Text in a code span (`` `#12` ``) in a comment: it shows a value, such as an input format.
#
# Before it scans, the script checks itself on a sample that holds each kind of reference it
# must find and each kind of text it must not. `--self-test` runs that check alone.
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
}

# Sets `comment` to the comment text of one line, skipping its code and strings and carrying
# the state of a string, a raw string or a block comment to the next line.
function lex(rest,   p, tok) {
    comment = ""
    while (rest != "") {
        if (in_block) {
            p = index(rest, "*/")
            if (p == 0) {
                comment = comment " " rest
                return
            }
            comment = comment " " substr(rest, 1, p - 1)
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
                return
            }
            tok = substr(rest, RSTART, RLENGTH)
            rest = substr(rest, RSTART + RLENGTH)
            if (tok == "//") {
                comment = comment " " rest
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

# Prints each reference in comment `text` at `line`.
function report(file, line, text,   found) {
    gsub(/`[^`]*`/, "", text)
    while (match(text, /(^|[^A-Za-z0-9_&#\/])#[0-9]+|(^|[^A-Za-z0-9_])([Ii]ssues?|PRs?|[Pp]ull[ \t]+[Rr]equests?)[ \t]+#?[0-9]+|github\.com\/[^\/ \t]+\/[^\/ \t]+\/(issues|pull)\/[0-9]+/)) {
        found = substr(text, RSTART, RLENGTH)
        text = substr(text, RSTART + RLENGTH)
        sub(/^[^#A-Za-z]+/, "", found)
        print file ":" line ": " found
    }
}

FNR == 1 {
    reset_file()
}

{
    lex($0)
    if (comment != "") {
        report(FILENAME, FNR, comment)
    }
}
EOF
)"

# Prints "<file>:<line>: <reference>" for each reference in the comments of the given files.
scan() {
    awk "${program}" "$@"
}

# Fails unless `scan` finds exactly the references of the sample.
self_test() {
    local dir sample expected actual
    dir="$(mktemp -d)"
    sample="${dir}/sample.rs"
    cat >"${sample}" <<'EOF'
//! Fixed in #101.
/// See issue 102 and PR #103, and pull request 104.
let pr = 7; // On #7, a fixture value. Issue 8 and PR 9 are fixture values too.
let numbers = [8, 9];
// https://github.com/owner/repo/pull/105 and github.com/o/r/issues/106.
/// Parses `#12`, `PR #34` and `https://github.com/o/r/pull/56`.
let url = "https://github.com/o/r/pull/57 // #58";
let long = "first line \
//github.com/o/r/pull/59; and #60";
let quote = '"'; // #61
let raw = r#"// #62 "quoted""#; // #63
/* Block #64,
   still the block, issue #65 */ let after = 1;
// HTML &#66; and a fragment page#67 and a colour #fff are not references.
EOF
    expected="${sample}:1: #101
${sample}:2: issue 102
${sample}:2: PR #103
${sample}:2: pull request 104
${sample}:3: #7
${sample}:3: Issue 8
${sample}:3: PR 9
${sample}:5: github.com/owner/repo/pull/105
${sample}:5: github.com/o/r/issues/106
${sample}:10: #61
${sample}:11: #63
${sample}:12: #64
${sample}:13: issue #65"
    actual="$(scan "${sample}")"
    rm -rf "${dir}"
    if [[ "${actual}" != "${expected}" ]]; then
        echo "check-comment-issue-numbers: self-test failed" >&2
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
    echo "check-comment-issue-numbers: no Rust files found" >&2
    exit 1
fi

result="$(scan "${files[@]}")"

if [[ -z "${result}" ]]; then
    exit 0
fi

while read -r finding; do
    echo "${finding}: issue or pull request number in a comment" >&2
done <<<"${result}"
cat >&2 <<'EOF'

A comment names an issue or a pull request. Say in the comment what the reader needs to know
instead; the history belongs in the commit message and the pull request.
EOF
exit 1
