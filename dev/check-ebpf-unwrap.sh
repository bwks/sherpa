#! /usr/bin/bash
#
# Refuse `.unwrap()` and `.expect(` in `crates/ebpf-redirect/src`.
#
# The workspace denies Clippy's `unwrap_used` and `expect_used`, but `crates/ebpf-redirect` is
# outside the workspace and has its own toolchain, so neither CI nor the hook runs Clippy on it.
# This script holds it to the same rule. Text after `//` on a line is a comment and is not
# scanned.
#
# Before it scans, the script checks itself on a sample that holds each kind of call it must
# find and each kind of text it must not. `--self-test` runs that check alone.
#
# Run by `.githooks/pre-commit` and by the Lint job in `.github/workflows/pr.yml`.

set -euo pipefail

# Prints "<file>:<line>: <line>" for each line of the given files whose code calls `unwrap` or
# `expect`.
scan() {
    awk '
        {
            code = $0
            sub(/\/\/.*/, "", code)
            if (code ~ /\.[ \t]*(unwrap[ \t]*\([ \t]*\)|expect[ \t]*\()/) {
                print FILENAME ":" FNR ": " $0
            }
        }
    ' "$@"
}

# Fails unless `scan` finds exactly the calls of the sample.
self_test() {
    local dir sample expected actual
    dir="$(mktemp -d)"
    sample="${dir}/main.rs"
    cat >"${sample}" <<'EOF'
let a = map.get(&key).unwrap();
let b = map.get(&key).expect("present");
let c = map
    .get(&key)
    .unwrap();
// A comment may say .unwrap() and .expect("x").
let d = map.get(&key).unwrap_or(0); // not .unwrap()
let e = map.get(&key).ok_or(Error::Missing)?;
EOF
    expected="${sample}:1: let a = map.get(&key).unwrap();
${sample}:2: let b = map.get(&key).expect(\"present\");
${sample}:5:     .unwrap();"
    actual="$(scan "${sample}")"
    rm -rf "${dir}"
    if [[ "${actual}" != "${expected}" ]]; then
        echo "check-ebpf-unwrap: self-test failed" >&2
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

mapfile -t files < <(git ls-files -- 'crates/ebpf-redirect/src/*.rs')

if [[ ${#files[@]} -eq 0 ]]; then
    echo "check-ebpf-unwrap: no Rust files found in crates/ebpf-redirect/src" >&2
    exit 1
fi

result="$(scan "${files[@]}")"

if [[ -z "${result}" ]]; then
    exit 0
fi

while read -r finding; do
    echo "${finding}: unwrap or expect in ebpf-redirect" >&2
done <<<"${result}"
cat >&2 <<'EOF'

crates/ebpf-redirect may not call `.unwrap()` or `.expect(`, the same rule the workspace's
Clippy `unwrap_used` and `expect_used` lints enforce on every other crate. Handle the `None`
or `Err` case instead, as `p2p_redirect` does with the `Result` of `try_redirect`.
EOF
exit 1
