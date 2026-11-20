#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.15"
# dependencies = []
# ///
"""Tests for changes_code.py (Python 3.15+, standard library only).

Each test builds a temporary git repository: a base commit with docs and code, and a head commit
with the pull request's changes. Run with:

    uv run .github/scripts/test_changes_code.py
"""

import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

SCRIPT = Path(__file__).resolve().with_name("changes_code.py")


class ChangesCodeTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.repo = Path(self.tmp.name)
        self.git("init", "--quiet", "--initial-branch=main")
        self.git("config", "user.name", "test")
        self.git("config", "user.email", "test@example.com")
        self.write(
            {
                "README.md": "Sherpa\n",
                "docs/architecture/RELEASE.md": "Release\n",
                "tests/specs/lab.toml": "[lab]\n",
                "crates/a/src/lib.rs": "pub fn a() {}\n",
                ".github/workflows/pr.yml": "name: PR\n",
            }
        )
        self.commit("base")
        self.base = self.git("rev-parse", "HEAD").strip()

    def tearDown(self):
        self.tmp.cleanup()

    def git(self, *args):
        return subprocess.run(
            ["git", *args], cwd=self.repo, check=True, capture_output=True, text=True
        ).stdout

    def write(self, files):
        for path, content in files.items():
            target = self.repo / path
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_text(content)

    def commit(self, message):
        self.git("add", "--all")
        self.git("commit", "--quiet", "--allow-empty", "--message", message)

    def run_script(self):
        """Run the script from the base to HEAD; return its output line."""
        result = subprocess.run(
            [sys.executable, str(SCRIPT), self.base, "HEAD"],
            cwd=self.repo,
            capture_output=True,
            text=True,
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        return result.stdout.strip()

    def check(self, files):
        """Commit `files` on top of the base and return the script's output line."""
        self.write(files)
        self.commit("head")
        return self.run_script()

    def test_only_docs_is_not_code(self):
        output = self.check(
            {
                "README.md": "Sherpa, changed\n",
                "docs/architecture/RELEASE.md": "Release, changed\n",
                "docs/images/lab.svg": "<svg/>\n",
                "tests/specs/lab.toml": "[lab]\nchanged = true\n",
            }
        )
        self.assertEqual(output, "code=false")

    def test_one_code_file_among_docs_is_code(self):
        output = self.check(
            {
                "README.md": "Sherpa, changed\n",
                "docs/architecture/RELEASE.md": "Release, changed\n",
                "crates/a/src/lib.rs": "pub fn a() -> u8 { 1 }\n",
            }
        )
        self.assertEqual(output, "code=true")

    def test_github_change_is_code(self):
        output = self.check({".github/workflows/pr.yml": "name: PR, changed\n"})
        self.assertEqual(output, "code=true")

    def test_markdown_under_crates_is_not_code(self):
        output = self.check({"crates/a/README.md": "Crate a\n"})
        self.assertEqual(output, "code=false")

    def test_empty_diff_is_not_code(self):
        self.commit("empty")
        self.assertEqual(self.run_script(), "code=false")

    def test_rename_counts_both_paths(self):
        self.git("mv", "crates/a/src/lib.rs", "docs/lib.rs")
        self.commit("code into docs")
        self.assertEqual(self.run_script(), "code=true")

    def test_rename_of_docs_into_code_is_code(self):
        self.git("mv", "docs/architecture/RELEASE.md", "crates/a/RELEASE.txt")
        self.commit("docs into code")
        self.assertEqual(self.run_script(), "code=true")


if __name__ == "__main__":
    unittest.main()
