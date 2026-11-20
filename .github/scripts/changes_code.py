#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.15"
# dependencies = []
# ///
"""Decide whether a pull request changes code (Python 3.15+, standard library only).

A pull request changes code unless every path it changes is docs: a `*.md` file anywhere, or
anything under `docs/` or `tests/specs/`. Everything else is code, `.github/` included. A rename
counts both its old and its new path. pr.yml skips its build jobs when this prints `code=false`.
See docs/architecture/RELEASE.md.

Run from the root of a checkout that has both revisions:

    changes_code.py BASE_REV HEAD_REV

It prints `code=true` or `code=false`, a line for `$GITHUB_OUTPUT`, and lists the code paths on
standard error.
"""

import subprocess
import sys

DOCS_DIRS = ("docs/", "tests/specs/")


def changed_paths(base, head):
    """Every path changed between two revisions; `--no-renames` lists a rename as both paths."""
    return subprocess.run(
        ["git", "diff", "--name-only", "--no-renames", base, head],
        check=True,
        capture_output=True,
        text=True,
    ).stdout.splitlines()


def is_docs(path):
    return path.endswith(".md") or path.startswith(DOCS_DIRS)


def main():
    if len(sys.argv) != 3:
        sys.exit(__doc__)
    code = [path for path in changed_paths(sys.argv[1], sys.argv[2]) if not is_docs(path)]
    for path in code:
        print(f"code: {path}", file=sys.stderr)
    print(f"code={'true' if code else 'false'}")


if __name__ == "__main__":
    main()
