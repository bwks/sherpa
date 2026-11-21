#! /usr/bin/bash
#
# Refuse code in `mod.rs` files.
#
# A `mod.rs` holds module docs, `mod` declarations and `pub use` re-exports, nothing else.
# This script scans every tracked `mod.rs`, test trees and `crates/ebpf-redirect` included,
# and fails on any other item: a function, a type, a constant, an `impl`, a macro, an inline
# `mod name { ... }`, and a private `use`.
#
# Items start a line in rustfmt-formatted code, so each line that starts in column 0 starts an
# item, an attribute or a comment. A `use` or an attribute is followed, across lines, to its end
# so its continuation lines are not read as items. Comments, blank lines, attributes and doc
# comments are allowed. A visibility is any `pub` form: `pub`, `pub(crate)`, `pub(super)`,
# `pub(in path)`.
#
# Before it scans, the script checks itself on a sample that holds each allowed kind of line
# and each refused kind of item. `--self-test` runs that check alone.
#
# Run by `.githooks/pre-commit` and by the Lint job in `.github/workflows/pr.yml`.

set -euo pipefail

# Prints "<file>:<line>: <first line of the item>" for each item in the given files that a
# `mod.rs` may not hold.
scan() {
    awk '
        FNR == 1 {
            in_use = 0
            depth = 0
        }

        # Brackets opened minus brackets closed in s.
        function depth_of(s,   t, opens, closes) {
            t = s
            opens = gsub(/[[({]/, "", t)
            t = s
            closes = gsub(/[])}]/, "", t)
            return opens - closes
        }

        # Inside a `pub use` or an attribute that spans lines: wait for its end.
        in_use {
            if ($0 ~ /;[ \t]*(\/\/.*)?$/) {
                in_use = 0
            }
            next
        }
        depth > 0 {
            depth += depth_of($0)
            next
        }

        /^[ \t]*$/ || /^[ \t]*\/\// {
            next
        }

        /^#!?\[/ {
            depth = depth_of($0)
            next
        }

        /^(pub(\([^)]*\))?[ \t]+)?mod[ \t]+[A-Za-z_][A-Za-z0-9_]*[ \t]*;/ {
            next
        }

        /^pub(\([^)]*\))?[ \t]+use[ \t]/ {
            if ($0 !~ /;[ \t]*(\/\/.*)?$/) {
                in_use = 1
            }
            next
        }

        /^[^ \t}\])]/ {
            print FILENAME ":" FNR ": " $0
        }
    ' "$@"
}

# Fails unless `scan` finds exactly the refused items of the sample.
self_test() {
    local dir sample expected actual
    dir="$(mktemp -d)"
    sample="${dir}/mod.rs"
    cat >"${sample}" <<'EOF'
//! Module docs.

/// A documented module.
mod allowed;
pub mod public;
#[cfg(test)]
pub(crate) mod tests;
#[cfg_attr(
    windows,
    path = "windows.rs"
)]
mod platform;
// A comment.
pub use allowed::Thing;
pub(crate) use public::{
    One,
    Two,
};
use std::fs;
mod inline {
    pub fn hidden() {}
}
const LIMIT: u8 = 3;
pub fn run() {
    let x = 1;
}
#[derive(Debug)]
struct Holder;
impl Holder {}
macro_rules! noop {
    () => {};
}
EOF
    expected="${sample}:19: use std::fs;
${sample}:20: mod inline {
${sample}:23: const LIMIT: u8 = 3;
${sample}:24: pub fn run() {
${sample}:28: struct Holder;
${sample}:29: impl Holder {}
${sample}:30: macro_rules! noop {"
    actual="$(scan "${sample}")"
    rm -rf "${dir}"
    if [[ "${actual}" != "${expected}" ]]; then
        echo "check-mod-rs: self-test failed" >&2
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

mapfile -t files < <(git ls-files -- 'mod.rs' '*/mod.rs')

if [[ ${#files[@]} -eq 0 ]]; then
    exit 0
fi

result="$(scan "${files[@]}")"

if [[ -z "${result}" ]]; then
    exit 0
fi

while read -r finding; do
    echo "${finding}: code in a mod.rs" >&2
done <<<"${result}"
cat >&2 <<'EOF'

A mod.rs holds only module docs, `mod` declarations and `pub use` re-exports. Move the code
into a named file of the module, or rename `name/mod.rs` to `name.rs` when it is the only file
in its folder; a test tree is no exception.
EOF
exit 1
