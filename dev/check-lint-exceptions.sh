#! /usr/bin/bash
#
# Refuse new lint exceptions: `allow` and `expect` attributes, including inside `cfg_attr`.
#
# `clippy::allow_attributes` already refuses `#[allow]` in workspace crates, but no lint refuses
# `#[expect]`. This script scans every tracked Rust file, including `crates/ebpf-redirect`
# (outside the workspace and its lints), and compares every exception it finds with the
# allowlist below. It fails when an exception is not on the list, and when a listed exception
# no longer exists, so the list shrinks as exceptions are removed.
#
# Adding to the allowlist needs the user's approval (AGENTS.md, "Clippy").
#
# Run by `.githooks/pre-commit` and by the Lint job in `.github/workflows/pr.yml`.

set -euo pipefail

# One line per exception: <file> <lint>. A lint named twice in one attribute, or in two
# attributes of the same file, needs a line for each.
allowlist="
crates/client/src/main.rs unsafe_code
crates/network/src/tc.rs clippy::cast_possible_truncation
crates/network/src/tc.rs clippy::cast_sign_loss
"

cd "$(git rev-parse --show-toplevel)"

mapfile -t files < <(git ls-files -- '*.rs')

if [[ ${#files[@]} -eq 0 ]]; then
    echo "check-lint-exceptions: no Rust files found" >&2
    exit 1
fi

# Attributes start a line in rustfmt-formatted code. From such a line the attribute is
# collected, across lines, until its brackets balance; string literals (the `reason`) are
# blanked first so brackets in them do not count. Every `allow(...)`/`expect(...)` in the
# collected text, at the top level or inside `cfg_attr`, gives one finding per lint path.
# Output: <file> <line> <lint>
scan() {
    awk '
        FNR == 1 { collecting = 0 }

        function strip_strings(s) {
            gsub(/\\./, "", s)
            gsub(/"[^"]*"/, "\"\"", s)
            return s
        }

        function depth_of(s,   t, opens, closes) {
            t = s
            opens = gsub(/[[(]/, "", t)
            t = s
            closes = gsub(/[])]/, "", t)
            return opens - closes
        }

        function report(text, file, line,   args, n, parts, i, lint) {
            while (match(text, /(^|[^A-Za-z0-9_])(allow|expect)[ \t]*\([^)]*\)/)) {
                args = substr(text, RSTART, RLENGTH)
                text = substr(text, RSTART + RLENGTH)
                sub(/^[^(]*\(/, "", args)
                sub(/\)$/, "", args)
                n = split(args, parts, ",")
                for (i = 1; i <= n; i++) {
                    lint = parts[i]
                    gsub(/^[ \t]+|[ \t]+$/, "", lint)
                    if (lint ~ /^[A-Za-z_][A-Za-z0-9_]*(::[A-Za-z_][A-Za-z0-9_]*)*$/) {
                        print file, line, lint
                    }
                }
            }
        }

        {
            code = strip_strings($0)
            if (!collecting) {
                if (code !~ /^[ \t]*#!?\[/) {
                    next
                }
                collecting = 1
                start = FNR
                text = ""
                depth = 0
            }
            text = text " " code
            depth += depth_of(code)
            if (depth <= 0) {
                collecting = 0
                report(text, FILENAME, start)
            }
        }
    ' "${files[@]}"
}

# Matches the findings against the allowlist, one listed line per finding.
# Prints "new <file> <line> <lint>" and "stale <file> <lint>" lines.
compare() {
    awk -v allowlist="${allowlist}" '
        BEGIN {
            n = split(allowlist, lines, "\n")
            for (i = 1; i <= n; i++) {
                if (split(lines[i], f, " ") == 2) {
                    allowed[f[1] " " f[2]]++
                }
            }
        }
        {
            key = $1 " " $3
            if (allowed[key] > 0) {
                allowed[key]--
            } else {
                print "new", $1, $2, $3
            }
        }
        END {
            for (key in allowed) {
                while (allowed[key]-- > 0) {
                    print "stale", key
                }
            }
        }
    '
}

result="$(scan | compare | sort -k1,2 -k3,3n)"

if [[ -z "${result}" ]]; then
    exit 0
fi

status=0
while read -r kind file rest; do
    case "${kind}" in
        new)
            read -r line lint <<<"${rest}"
            echo "${file}:${line}: new lint exception \`${lint}\`" >&2
            status=1
            ;;
        stale)
            echo "${file}: allowlisted lint exception \`${rest}\` no longer exists; remove it from the allowlist in dev/check-lint-exceptions.sh" >&2
            status=1
            ;;
    esac
done <<<"${result}"

if grep -q '^new ' <<<"${result}"; then
    cat >&2 <<'EOF'

Lint exceptions (`allow`/`expect` attributes) are not accepted: fix the code so the lint
passes instead. See AGENTS.md, "Clippy". An exception the user has approved goes in the
allowlist at the top of dev/check-lint-exceptions.sh.
EOF
fi

exit "${status}"
