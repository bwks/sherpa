#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.15"
# dependencies = ["pytest==9.2.0"]
# ///
"""End-to-end test of the manifest commands of the sherpa CLI (Python 3.15+, pytest).

Checks the command cases of tests/specs/manifests.md, none of which brings a lab up: what
`sherpa validate` prints and exits with, with no manifest, with an unknown key, with a failing
check and with no login or server; what `sherpa new` writes, and that it leaves an existing
manifest unless forced; and how `sherpa up` reports a manifest the server refuses before it
creates anything. Runs wherever the `sherpa` CLI is logged in; see README.md.

A pytest module: the `prepared` fixture makes the run's folder, in which each case writes its
manifest in a folder of its own, and the `logged_in` fixture checks the CLI's login for the case
that sends a manifest to the server. The cases are the `test_<case_id>_*` functions, in case
order. Run as a script, it runs pytest on itself; `--self-test` runs the runner's own unit tests
instead.
"""

import argparse
import os
from pathlib import Path
import shutil
import signal
import sys
import tomllib
import unittest
from unittest import mock
import uuid

import pytest

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))

from labkit import (
    COMMAND_TIMEOUT,
    TestFailure,
    require,
    require_device_password,
    run_logged,
    stop_on_sigterm,
    value_toml,
)


LAB = Path(__file__).resolve().parent
ROOT = LAB.parent.parent.parent
RUNS = ROOT / ".tmp" / "lab-tests"

# What `sherpa validate` prints: a line as each check starts, a tick under it once it passes, and
# a last line once all have passed.
CHECK_STARTED = "→ Checking "
CHECK_PASSED = "  ✓ "
VALIDATE_PASSED = "Manifest validation passed!"
DESCRIPTION_CHECK = "→ Checking the description..."
REFUSED_ENVIRONMENT_KEY = "1BAD"
MAX_DESCRIPTION = 1000

# The nodes of the cases' manifests. Only the refused `sherpa up` sends one to the server, which
# refuses it before it looks for the node's image.
NODES = (
    {"name": "frr01", "model": "frr_linux"},
    {"name": "nats01", "model": "nats_bus"},
)

UP_TIMEOUT = 600


def render_manifest(name, nodes, description=None):
    """A manifest of `nodes` under the lab name `name`."""
    lines = [f"name = {value_toml(name)}"]
    if description is not None:
        lines.append(f"description = {value_toml(description)}")
    for node in nodes:
        lines += ["", "[[nodes]]", *(f"{key} = {value_toml(value)}" for key, value in node.items())]
    return "\n".join(lines) + "\n"


def validate_checks(output):
    """The checks `sherpa validate` printed it was starting, each with whether it was ticked as
    passed on the next line."""
    lines = output.splitlines()
    return [(line.strip(), index + 1 < len(lines) and lines[index + 1].startswith(CHECK_PASSED))
            for index, line in enumerate(lines) if line.startswith(CHECK_STARTED)]


def without_login(home):
    """The environment of a machine where the CLI has no login and no server: `home` as the home
    folder, and none of the CLI's SHERPA_ variables."""
    env = {key: value for key, value in os.environ.items() if not key.startswith("SHERPA_")}
    env["HOME"] = str(home)
    return env


# -- the run -------------------------------------------------------------------------------------


class Run:
    """The run's folder, its command log, and a folder of it for each case."""

    def __init__(self, root):
        self.root = root
        self.name = root.name
        self.log = (root / "commands.log").open("w")

    def case_directory(self, name):
        """An empty folder of the run for a case's manifest."""
        directory = self.root / name
        directory.mkdir()
        return directory

    def sherpa(self, label, arguments, cwd, timeout=COMMAND_TIMEOUT, check=True, env=None):
        outcome = run_logged(self.log, label, ["sherpa", *arguments], cwd, timeout, env)
        if check and outcome.code != 0:
            raise TestFailure(f"{label} failed (exit {outcome.code}); see {self.log.name}")
        return outcome


# -- the fixtures --------------------------------------------------------------------------------


@pytest.fixture(scope="module")
def prepared():
    """The run's folder under `.tmp/lab-tests/`, with the sherpa CLI on PATH; its case folders are
    removed at the end, and its command log kept."""
    signal.signal(signal.SIGTERM, stop_on_sigterm)
    require(shutil.which("sherpa"), "sherpa is not on PATH")
    root = RUNS / f"manifest-commands-{uuid.uuid4().hex[:12]}"
    root.mkdir(parents=True)
    print(f"Logs: {root}", flush=True)
    run = Run(root)
    try:
        yield run
    finally:
        run.log.close()
        for entry in root.iterdir():
            if entry.is_dir():
                shutil.rmtree(entry)


@pytest.fixture(scope="module")
def logged_in(prepared):
    """The run, once the CLI is checked to be logged in with a device password, which `sherpa up`
    needs before the server reads the manifest's nodes."""
    whoami = run_logged(prepared.log, "sherpa whoami", ["sherpa", "whoami"], prepared.root)
    require(whoami.code == 0, "the sherpa CLI is not logged in (run `sherpa login`)")
    require_device_password(prepared.log, prepared.root)
    return prepared


# -- cases ---------------------------------------------------------------------------------------


def test_manifest_file_001_validate_without_a_manifest_exits_1_saying_it_was_not_found(prepared):
    directory = prepared.case_directory("no-manifest")
    outcome = prepared.sherpa("sherpa validate (no manifest)", ["validate"], directory, check=False)
    require(outcome.code == 1, f"sherpa validate with no manifest.toml exited {outcome.code}, not 1")
    require("manifest.toml" in outcome.output and "No such file or directory" in outcome.output,
            f"sherpa validate did not say manifest.toml was not found: {outcome.output.strip()}")


def test_manifest_file_009_validate_names_the_file_place_and_key_of_an_unknown_key(prepared):
    directory = prepared.case_directory("unknown-key")
    (directory / "manifest.toml").write_text(
        f'name = "{prepared.name}-unknown-key"\n\n[[nodes]]\nname = "n1"\nmodel = "frr_linux"\n'
        'colour = "blue"\n')
    outcome = prepared.sherpa("sherpa validate (unknown key)", ["validate"], directory, check=False)
    require(outcome.code == 1, f"sherpa validate of an unknown key exited {outcome.code}, not 1")
    for expected in ("manifest.toml", "line 6, column 1", "unknown field `colour`",
                     "expected one of", "`model`"):
        require(expected in outcome.output,
                f"sherpa validate of an unknown key did not print {expected!r}: {outcome.output.strip()}")


def test_manifest_file_010_new_writes_two_linux_nodes_and_a_link_that_validate_passes(prepared):
    directory = prepared.case_directory("new")
    prepared.sherpa("sherpa new", ["new"], directory)
    manifest = tomllib.loads((directory / "manifest.toml").read_text())
    require(manifest.get("name"), "sherpa new wrote a manifest with no lab name")
    nodes = manifest.get("nodes", [])
    require(len(nodes) == 2 and all(node["model"].endswith("_linux") for node in nodes),
            f"sherpa new did not write two Linux nodes: {nodes}")
    links = manifest.get("links", [])
    ends = {end.split("::")[0] for link in links for end in (link["src"], link["dst"])}
    require(len(links) == 1 and ends == {node["name"] for node in nodes},
            f"sherpa new did not link its two nodes: {links}")
    outcome = prepared.sherpa("sherpa validate (new)", ["validate"], directory, check=False)
    require(outcome.code == 0 and VALIDATE_PASSED in outcome.output,
            f"sherpa validate did not pass the manifest sherpa new wrote (exit {outcome.code})")


def test_manifest_file_011_new_refuses_an_existing_manifest_and_force_writes_over_it(prepared):
    directory = prepared.case_directory("new-existing")
    path = directory / "manifest.toml"
    existing = render_manifest(f"{prepared.name}-existing", NODES[:1])
    path.write_text(existing)
    outcome = prepared.sherpa("sherpa new (existing manifest)", ["new"], directory, check=False)
    require(path.read_text() == existing, "sherpa new wrote over an existing manifest.toml")
    require(outcome.code != 0, "sherpa new on an existing manifest.toml exited 0")
    require("already exists" in outcome.output,
            f"sherpa new did not say manifest.toml already exists: {outcome.output.strip()}")
    prepared.sherpa("sherpa new --force", ["new", "--force"], directory)
    written = tomllib.loads(path.read_text())
    require(path.read_text() != existing and len(written.get("nodes", [])) == 2,
            "sherpa new --force did not write its example over the existing manifest.toml")


def test_validate_command_001_a_valid_manifest_passes_every_check_with_no_login_or_server(prepared):
    directory = prepared.case_directory("valid")
    (directory / "manifest.toml").write_text(render_manifest(f"{prepared.name}-valid", NODES))
    home = prepared.case_directory("empty-home")
    outcome = prepared.sherpa("sherpa validate (no login, no server)", ["validate"], directory,
                              check=False, env=without_login(home))
    require(outcome.code == 0, f"sherpa validate with no login exited {outcome.code}, not 0")
    checks = validate_checks(outcome.stdout)
    require(checks and all(ticked for _, ticked in checks),
            f"sherpa validate did not tick every check: {checks}")
    require(VALIDATE_PASSED in outcome.stdout, f"sherpa validate did not print {VALIDATE_PASSED!r}")


def test_validate_command_002_a_failing_check_exits_1_with_its_message_and_no_later_check(prepared):
    directory = prepared.case_directory("long-description")
    (directory / "manifest.toml").write_text(
        render_manifest(f"{prepared.name}-long-description", NODES[:1], "a" * (MAX_DESCRIPTION + 1)))
    outcome = prepared.sherpa("sherpa validate (description too long)", ["validate"], directory,
                              check=False)
    require(outcome.code == 1, f"sherpa validate of a long description exited {outcome.code}, not 1")
    require(f"{MAX_DESCRIPTION + 1} characters" in outcome.output,
            f"sherpa validate did not print the description check's message: {outcome.output.strip()}")
    checks = validate_checks(outcome.stdout)
    require(checks and checks[-1] == (DESCRIPTION_CHECK, False),
            f"sherpa validate went on past the failed description check: {checks}")
    require(VALIDATE_PASSED not in outcome.output, "sherpa validate said a failed manifest passed")


def test_validate_command_005_up_refused_by_the_server_exits_1_naming_the_check_and_node(logged_in):
    directory = logged_in.case_directory("refused-up")
    node = {**NODES[0], "environment_variables": [f"{REFUSED_ENVIRONMENT_KEY}=value"]}
    (directory / "manifest.toml").write_text(render_manifest(f"{logged_in.name}-refused", [node]))
    outcome = logged_in.sherpa("sherpa up (refused by the server)", ["up"], directory, UP_TIMEOUT,
                               check=False)
    built = (directory / ".sherpa" / "lab-info.toml").exists()
    if outcome.code != 1 or built:
        logged_in.sherpa("sherpa destroy (refused up)", ["destroy", "--yes"], directory, UP_TIMEOUT,
                         check=False)
    require(outcome.code == 1, f"sherpa up of a manifest the server refuses exited {outcome.code}, not 1")
    require(not built, "sherpa up of a manifest the server refuses wrote lab-info.toml")
    for expected in (f"environment variable key '{REFUSED_ENVIRONMENT_KEY}'", f"node '{node['name']}'"):
        require(expected in outcome.output,
                f"sherpa up did not print the server's refusal naming {expected}: {outcome.output.strip()}")


def main():
    parser = argparse.ArgumentParser(description=__doc__,
                                     formatter_class=argparse.RawDescriptionHelpFormatter,
                                     epilog="Other arguments, such as -k, are passed to pytest.")
    parser.add_argument("--self-test", action="store_true", help="Run the runner's own unit tests")
    args, pytest_args = parser.parse_known_args()
    if args.self_test:
        sys.argv = sys.argv[:1]
        unittest.main()
        return 0
    return pytest.main(["-v", "-p", "no:cacheprovider", __file__, *pytest_args])


class SelfTest(unittest.TestCase):
    # The runner's own checks, run with --self-test; pytest must not collect them as cases.
    __test__ = False

    def test_a_check_is_ticked_only_by_the_line_under_it(self):
        output = ("→ Checking one...\n  ✓ One passed\n→ Checking two...\nFailed\n"
                  "→ Checking three...")
        self.assertEqual(validate_checks(output), [("→ Checking one...", True),
                                                   ("→ Checking two...", False),
                                                   ("→ Checking three...", False)])

    def test_a_manifest_reads_back_as_its_nodes_and_description(self):
        node = {**NODES[0], "environment_variables": ["A=1"]}
        manifest = tomllib.loads(render_manifest("lab", [node, NODES[1]], "about"))
        self.assertEqual(manifest, {"name": "lab", "description": "about", "nodes": [node, NODES[1]]})

    def test_no_login_drops_the_cli_variables_and_moves_home(self):
        with mock.patch.dict(os.environ, {"SHERPA_SERVER": "x", "PATH": "/bin"}):
            env = without_login(Path("/tmp/home"))
        self.assertNotIn("SHERPA_SERVER", env)
        self.assertEqual((env["HOME"], env["PATH"]), ("/tmp/home", "/bin"))


if __name__ == "__main__":
    sys.exit(main())
