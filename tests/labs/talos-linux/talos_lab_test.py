#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.15"
# dependencies = ["pytest==9.2.0"]
# ///
"""End-to-end test of Sherpa-built Talos clusters through the sherpa CLI (Python 3.15+, pytest).

Runs the pytest module of each lab in this folder, `single_lab_test.py` and `ha_lab_test.py`,
against the server the logged-in `sherpa` CLI points at, each in its own pytest process and
both at the same time, in one run directory. Each module brings its lab up once, checks its
cases from the lab's util node with talosctl and kubectl, and always destroys the lab. Ctrl-C
reaches both processes, and SIGTERM is passed on to them. With `--collect-only` it collects the
modules in this process instead; `--self-test` runs the runner's own unit tests. See README.md.
"""

import argparse
from pathlib import Path
import signal
import subprocess
import sys
import tempfile
import tomllib
import unittest
from unittest import mock

import pytest

sys.path.insert(0, str(Path(__file__).resolve().parent))
sys.path.insert(0, str(Path(__file__).resolve().parent.parent))

from labkit import Outcome, TestFailure, stop_on_sigterm
from talos import LAB_NAMES, LABS, Lab, new_run_root, node_conditions, owner_only, realm_member


MODULES = {lab: LABS / f"{lab}_lab_test.py" for lab in LAB_NAMES}
PYTEST = ("-v", "-p", "no:cacheprovider")
COLLECT_ONLY = ("--collect-only", "--co")


def split_labs(arguments):
    """The labs named first on the command line, and the rest, which go to pytest."""
    labs = []
    for argument in arguments:
        if argument not in LAB_NAMES:
            break
        labs.append(argument)
    return labs, arguments[len(labs):]


def child_command(lab, run_root, pytest_args):
    """The pytest process that runs one lab's module in the shared run directory."""
    return [sys.executable, "-m", "pytest", *PYTEST, str(MODULES[lab]), "--run-root", str(run_root),
            *pytest_args]


def run_labs(labs, run_root, pytest_args):
    """Run every lab's module at once, each in its own pytest process so an interrupt reaches each
    lab's teardown. Returns the labs that failed."""
    children = {lab: subprocess.Popen(child_command(lab, run_root, pytest_args)) for lab in labs}
    try:
        codes = {lab: child.wait() for lab, child in children.items()}
    except KeyboardInterrupt:
        # Ctrl-C reaches the children directly; SIGTERM reaches only this process, so pass it on.
        for child in children.values():
            if child.poll() is None:
                child.send_signal(signal.SIGTERM)
        for child in children.values():
            child.wait()
        raise
    return [lab for lab, code in codes.items() if code != 0]


def main():
    parser = argparse.ArgumentParser(description=__doc__,
                                     formatter_class=argparse.RawDescriptionHelpFormatter,
                                     usage=f"%(prog)s [{'|'.join(LAB_NAMES)} ...] [--self-test] [pytest options]",
                                     epilog=f"Name labs first to run only those: {', '.join(LAB_NAMES)} "
                                            "(default: all). Other arguments, such as --keep-failed, "
                                            "--collect-only or -k, are passed to pytest.")
    parser.add_argument("--self-test", action="store_true", help="Run the runner's own unit tests")
    args, rest = parser.parse_known_args()
    if args.self_test:
        sys.argv = sys.argv[:1]
        unittest.main()
        return 0
    labs, pytest_args = split_labs(rest)
    labs = labs or list(LAB_NAMES)
    if any(argument in COLLECT_ONLY for argument in pytest_args):
        return pytest.main([*PYTEST, *(str(MODULES[lab]) for lab in labs), *pytest_args])
    signal.signal(signal.SIGTERM, stop_on_sigterm)
    run_root = new_run_root()
    print(f"Logs: {run_root}", flush=True)
    failed = run_labs(labs, run_root, pytest_args)
    print(f"{'FAIL' if failed else 'PASS'}: {len(labs) - len(failed)}/{len(labs)} labs passed"
          + (f" (failed: {', '.join(failed)})" if failed else "") + f". Logs: {run_root}")
    return 1 if failed else 0


class SelfTest(unittest.TestCase):
    # The runner's own checks, run with --self-test; pytest must not collect them as lab cases.
    __test__ = False

    def test_labs_named_first_are_run_and_the_rest_goes_to_pytest(self):
        self.assertEqual(split_labs(["single", "--keep-failed", "-k", "ha"]),
                         (["single"], ["--keep-failed", "-k", "ha"]))
        self.assertEqual(split_labs(["-k", "single"]), ([], ["-k", "single"]))

    def test_each_lab_runs_its_own_module_in_the_shared_run_directory(self):
        command = child_command("ha", Path("/run/x"), ["--keep-failed"])
        self.assertEqual(command[-4:], [str(LABS / "ha_lab_test.py"), "--run-root", "/run/x", "--keep-failed"])

    def test_a_partly_created_lab_keeps_its_id_for_cleanup(self):
        # `sherpa up` exits 1 on a partly created lab after writing lab-info.toml; the run must
        # still know the lab's ID, so destroy checks for leftovers instead of skipping them.
        with tempfile.TemporaryDirectory() as temp:
            lab = Lab(LAB_NAMES[0], Path(temp) / "talos-linux-000000abcdef")
            lab.prepare()
            self.addCleanup(lab.log.close)
            (lab.directory / ".sherpa").mkdir()
            (lab.directory / ".sherpa" / "lab-info.toml").write_text(
                f'id = "4ed6360e"\nname = "{lab.name}"\nipv4_network = "172.31.0.0/24"\n')

            def failing_up(log, label, arguments, *rest, **options):
                return Outcome(1 if arguments[:2] == ["sherpa", "up"] else 0, "", "")

            with mock.patch("talos.run_logged", failing_up), self.assertRaises(TestFailure):
                lab.up()
            self.assertEqual(lab.lab_id, "4ed6360e")

            def leftover(log, label, arguments, *rest, **options):
                return Outcome(1 if "inspect" in arguments else 0, "node-4ed6360e-x\n", "")

            with (mock.patch("talos.run_logged", leftover),
                  mock.patch("shutil.which", return_value="/usr/bin/true"),
                  self.assertRaises(TestFailure)):
                lab.destroy()

    def test_the_keycloak_owner_is_the_only_realm_user_and_the_admin_is_apart(self):
        # The layout the client writes (crates/client/src/keycloak_credentials.rs): the master
        # realm admin is its own table, not one of the realm users.
        credentials = tomllib.loads(
            'address = "https://172.31.1.23:8443"\nrealm = "sherpa"\n'
            '[admin]\nusername = "admin"\npassword = "kc-admin-pw"\n'
            '[[users]]\nusername = "alice"\npassword = "s3cret"\n')
        self.assertEqual(realm_member(credentials["users"])["username"], "alice")
        self.assertEqual(credentials["admin"]["username"], "admin")
        with self.assertRaises(TestFailure):
            realm_member([])

    def test_node_conditions(self):
        nodes = node_conditions({"items": [
            {"metadata": {"name": "cp1"}, "status": {
                "conditions": [{"type": "Ready", "status": "True"}],
                "addresses": [{"type": "Hostname", "address": "cp1"},
                              {"type": "InternalIP", "address": "172.31.1.21"}]}},
            {"metadata": {"name": "w1"}, "status": {
                "conditions": [{"type": "Ready", "status": "Unknown"}], "addresses": []}},
        ]})
        self.assertEqual(nodes, {"cp1": (True, "172.31.1.21"), "w1": (False, None)})

    def test_owner_only(self):
        with tempfile.TemporaryDirectory() as temp:
            path = Path(temp) / "file"
            path.write_text("x")
            path.chmod(0o600)
            self.assertTrue(owner_only(path))
            path.chmod(0o644)
            self.assertFalse(owner_only(path))


if __name__ == "__main__":
    sys.exit(main())
