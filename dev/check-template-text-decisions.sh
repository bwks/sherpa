#! /usr/bin/bash
#
# Refuse template tags that decide from an enum's text.
#
# A template decides from typed values that Rust gives it, never from how an enum is spelled:
# renaming a variant's text must not change what a page shows or offers. This script scans
# every tracked web UI template (`crates/server/templates/**/*.jinja`) and fails on any `{% ... %}` tag that calls
# `to_string()` or compares with a non-empty string literal (`== "x"` or `!= "x"`). A
# comparison with the empty string, `== ""`, is allowed.
#
# Before it scans, the script checks itself on a sample that holds one tag of each kind it must
# find and one it must not. `--self-test` runs that check alone.
#
# Run by `.githooks/pre-commit` and by the Lint job in `.github/workflows/pr.yml`.

set -euo pipefail

# Prints "<file>:<line>: <tag>" for each tag that decides from an enum's text in the given
# files. A tag that spans lines is printed on one, with its first line's number.
scan() {
    awk '
        function report(file, text,   line, rest, at, tag, stop, found, op, ops, i, pos, after) {
            line = 1
            rest = text
            while ((at = index(rest, "{%")) > 0) {
                line += newlines(substr(rest, 1, at - 1))
                rest = substr(rest, at + 2)
                stop = index(rest, "%}")
                if (stop == 0) {
                    return
                }
                tag = substr(rest, 1, stop - 1)
                found = index(tag, "to_string()") > 0
                split("== \"|!= \"", ops, "|")
                for (i = 1; i <= 2 && !found; i++) {
                    op = ops[i]
                    after = tag
                    while ((pos = index(after, op)) > 0) {
                        after = substr(after, pos + length(op))
                        if (substr(after, 1, 1) != "\"") {
                            found = 1
                            break
                        }
                    }
                }
                if (found) {
                    gsub(/[ \t\n]+/, " ", tag)
                    gsub(/^ | $/, "", tag)
                    print file ":" line ": " tag
                }
            }
        }

        function newlines(s,   t) {
            t = s
            return gsub(/\n/, "", t)
        }

        FNR == 1 && NR != 1 {
            report(previous, text)
        }
        FNR == 1 {
            text = $0
            previous = FILENAME
            next
        }
        {
            text = text "\n" $0
        }
        END {
            if (NR > 0) {
                report(previous, text)
            }
        }
    ' "$@"
}

# Fails unless `scan` finds exactly the two tags of the sample that decide from an enum's text.
self_test() {
    local dir sample expected actual
    dir="$(mktemp -d)"
    sample="${dir}/sample.html.jinja"
    cat >"${sample}" <<'EOF'
{% if state.to_string() == "running" %}{% elif kind == "vm" %}{% if ip != "" %}
<a data-lab-id="{{ lab_id }}">{% if
    phase == "ready" %}{% endif %}</a>
EOF
    expected="${sample}:1: if state.to_string() == \"running\"
${sample}:1: elif kind == \"vm\"
${sample}:2: if phase == \"ready\""
    actual="$(scan "${sample}")"
    rm -rf "${dir}"
    if [[ "${actual}" != "${expected}" ]]; then
        echo "check-template-text-decisions: self-test failed" >&2
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

mapfile -t files < <(git ls-files -- 'crates/server/templates/*.jinja')

if [[ ${#files[@]} -eq 0 ]]; then
    echo "check-template-text-decisions: no templates found" >&2
    exit 1
fi

result="$(scan "${files[@]}")"

if [[ -z "${result}" ]]; then
    exit 0
fi

while read -r finding; do
    echo "${finding}: decides from an enum's text" >&2
done <<<"${result}"
cat >&2 <<'EOF'

A template tag calls `to_string()` or compares with a string literal. Give the template a typed
value, such as a bool or an enum, from Rust and decide from that instead. See AGENTS.md,
"HTML Templates".
EOF
exit 1
