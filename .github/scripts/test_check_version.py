#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.15"
# dependencies = []
# ///
"""Tests for check_version.py (Python 3.15+, standard library only).

Each test builds a small Cargo workspace in a temporary git repository: a base commit tagged with
the last release, and a head commit with the pull request's changes. Run with:

    uv run .github/scripts/test_check_version.py
"""

import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

SCRIPT = Path(__file__).resolve().with_name("check_version.py")
LAST_RELEASE = "0.5.32"


def manifest(version):
    return (
        "[workspace]\n"
        'members = ["crates/a", "crates/b"]\n'
        "\n"
        "[workspace.package]\n"
        f'version = "{version}"\n'
    )


def crate(name, version="version.workspace = true"):
    return f'[package]\nname = "{name}"\n{version}\n'


def lockfile(version, b_version=None):
    return (
        "version = 4\n"
        "\n"
        "[[package]]\n"
        'name = "a"\n'
        f'version = "{version}"\n'
        "\n"
        "[[package]]\n"
        'name = "b"\n'
        f'version = "{b_version or version}"\n'
        "\n"
        "[[package]]\n"
        'name = "serde"\n'
        'version = "1.0.0"\n'
        'source = "registry+https://github.com/rust-lang/crates.io-index"\n'
    )


def changelog(*versions):
    sections = "".join(f"## [{v}] - 2026-11-11\n\n- A change\n\n" for v in versions)
    return f"# Changelog\n\n{sections}"


class CheckVersionTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.repo = Path(self.tmp.name)
        self.git("init", "--quiet", "--initial-branch=main")
        self.git("config", "user.name", "test")
        self.git("config", "user.email", "test@example.com")
        self.write(
            {
                "Cargo.toml": manifest(LAST_RELEASE),
                "crates/a/Cargo.toml": crate("a"),
                "crates/b/Cargo.toml": crate("b"),
                "Cargo.lock": lockfile(LAST_RELEASE),
                "CHANGELOG.md": changelog(LAST_RELEASE),
                "README.md": "Sherpa\n",
            }
        )
        self.commit("base")
        self.git("tag", f"v{LAST_RELEASE}")
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
        self.git("commit", "--quiet", "--message", message)

    def check(self, files):
        """Commit `files` on top of the base and run the check; return (exit code, output)."""
        self.write(files)
        self.commit("head")
        result = subprocess.run(
            [sys.executable, str(SCRIPT), self.base],
            cwd=self.repo,
            capture_output=True,
            text=True,
        )
        return result.returncode, result.stdout + result.stderr

    def bump(self, version, **extra):
        return {
            "Cargo.toml": manifest(version),
            "Cargo.lock": lockfile(version),
            "CHANGELOG.md": changelog(version, LAST_RELEASE),
            **extra,
        }

    def test_unchanged_version_passes(self):
        code, output = self.check({"README.md": "Sherpa, changed\n"})
        self.assertEqual(code, 0, output)
        self.assertIn(f"unchanged at {LAST_RELEASE}", output)

    def test_bump_passes(self):
        code, output = self.check(self.bump("0.5.33"))
        self.assertEqual(code, 0, output)
        self.assertIn(f"{LAST_RELEASE} -> 0.5.33", output)

    def test_version_change_with_other_files_fails(self):
        code, output = self.check(self.bump("0.5.33", **{"README.md": "Sherpa, changed\n"}))
        self.assertEqual(code, 1, output)
        self.assertIn("also changes README.md", output)

    def test_bump_not_higher_than_last_tag_fails(self):
        self.git("tag", "v0.5.40")
        code, output = self.check(self.bump("0.5.33"))
        self.assertEqual(code, 1, output)
        self.assertIn("not higher than the last release, 0.5.40", output)

    def test_bump_to_tagged_version_fails(self):
        self.git("tag", "v0.5.33")
        code, output = self.check(self.bump("0.5.33"))
        self.assertEqual(code, 1, output)
        self.assertIn("not higher than the last release, 0.5.33", output)

    def test_bump_without_changelog_section_fails(self):
        code, output = self.check(
            {"Cargo.toml": manifest("0.5.33"), "Cargo.lock": lockfile("0.5.33")}
        )
        self.assertEqual(code, 1, output)
        self.assertIn("no ## [0.5.33] section", output)

    def test_changelog_section_must_match_exactly(self):
        code, output = self.check(
            self.bump("0.5.3") | {"CHANGELOG.md": changelog("0.5.33", LAST_RELEASE)}
        )
        self.assertEqual(code, 1, output)
        self.assertIn("no ## [0.5.3] section", output)

    def test_bump_that_is_not_semver_fails(self):
        code, output = self.check(self.bump("0.6"))
        self.assertEqual(code, 1, output)
        self.assertIn("0.6 is not a X.Y.Z version", output)

    def test_crate_with_its_own_version_fails(self):
        code, output = self.check({"crates/b/Cargo.toml": crate("b", 'version = "9.9.9"')})
        self.assertEqual(code, 1, output)
        self.assertIn("crates/b/Cargo.toml sets its own version", output)

    def test_stale_lockfile_fails(self):
        code, output = self.check({"Cargo.lock": lockfile(LAST_RELEASE, b_version="0.5.31")})
        self.assertEqual(code, 1, output)
        self.assertIn("Cargo.lock does not have b at", output)


if __name__ == "__main__":
    unittest.main()
