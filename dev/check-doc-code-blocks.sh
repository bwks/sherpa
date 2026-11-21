#! /usr/bin/bash
#
# Refuse code blocks in doc comments.
#
# Doc comments (`//!` and `///`) carry no fenced code blocks of any language, so there are no
# doctests. This script scans every tracked Rust file, including `crates/ebpf-redirect`, and
# fails on any doc comment line that opens or closes a fence: three or more backticks or tildes
# after the comment marker. `////` is a plain comment, not a
# doc comment, and is not checked. Indented code blocks are not detected.
#
# Before it scans, the script checks itself on a sample that holds each kind of fence it must
# find and the lines it must not. `--self-test` runs that check alone.
#
# Run by `.githooks/pre-commit` and by the Lint job in `.github/workflows/pr.yml`.

set -euo pipefail

# Prints "<file>:<line>: <line>" for each doc comment line in the given files that is a fence.
scan() {
    awk '
        /^[ \t]*(\/\/!|\/\/\/)[ \t]*(```|~~~)/ {
            line = $0
            sub(/^[ \t]+/, "", line)
            print FILENAME ":" FNR ": " line
        }
    ' "$@"
}

# Fails unless `scan` finds exactly the fences of the sample.
self_test() {
    local dir sample expected actual
    dir="$(mktemp -d)"
    sample="${dir}/sample.rs"
    cat >"${sample}" <<'EOF'
//! Module docs with `inline code`.
//! ```
//! let x = 1;
//! ```
/// Item docs.
///```rust
    /// ~~~toml
//// ```
// ```
/// Two backticks are inline code: `` a ` b ``.
let fence = "/// ```";
EOF
    expected="${sample}:2: //! \`\`\`
${sample}:4: //! \`\`\`
${sample}:6: ///\`\`\`rust
${sample}:7: /// ~~~toml"
    actual="$(scan "${sample}")"
    rm -rf "${dir}"
    if [[ "${actual}" != "${expected}" ]]; then
        echo "check-doc-code-blocks: self-test failed" >&2
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
    echo "check-doc-code-blocks: no Rust files found" >&2
    exit 1
fi

result="$(scan "${files[@]}")"

if [[ -z "${result}" ]]; then
    exit 0
fi

while read -r finding; do
    echo "${finding}: code block in a doc comment" >&2
done <<<"${result}"
cat >&2 <<'EOF'

Doc comments carry no code blocks, so there are no doctests. Describe the behaviour in prose,
with `inline code` for names, and prove it with a test instead.
EOF
exit 1
