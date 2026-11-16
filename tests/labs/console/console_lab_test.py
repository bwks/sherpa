#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.15"
# dependencies = []
# ///
"""End-to-end test of `sherpa console` through the sherpa CLI (Python 3.15+, stdlib only).

Brings up an Alpine VM and an Alpine container. Runs `sherpa console` on the VM in a
pseudo-terminal, as a user does: the console is reached through the Sherpa server with the
client's own telnet, with no system `telnet` on PATH. Checks the login prompt and typed input,
leaving with Ctrl+] and with `~.`, that a second console on the same node reports it in use, and
that the container is refused. Always destroys the lab. See README.md.
"""

import argparse
import json
import os
from pathlib import Path
import re
import select
import shlex
import shutil
import signal
import subprocess
import sys
import tempfile
import time
import tomllib
from typing import NamedTuple
import unittest
from unittest import mock
import uuid


LAB = Path(__file__).resolve().parent
ROOT = LAB.parent.parent.parent
RUNS = ROOT / ".tmp" / "lab-tests"
VM = "alp01"
CONTAINER = "ctr01"
# The lab, as `render_manifest` writes it for each run under a unique name.
NODES = (
    {"name": VM, "model": "alpine_linux"},
    {"name": CONTAINER, "model": "generic_container", "image": "alpine", "version": "3.22"},
)
IMAGES = (("alpine_linux", None), ("generic_container", "3.22"))

COMMAND_TIMEOUT = 180
UP_TIMEOUT = 1200
# How long the VM may take to show its login prompt on the console after `sherpa up`.
LOGIN_TIMEOUT = 600
# How long `sherpa console` may take to connect, or to end after an escape.
CONSOLE_TIMEOUT = 60
POLL_INTERVAL = 5
CTRL_RIGHT_BRACKET = b"\x1d"
LOGIN_PROMPT = re.compile(rb"login: ")
CONNECTED = f"Connected to the console of {VM} in lab "
CLOSED_AFTER_ESCAPE = f"\nConnection to the console of {VM} closed."
IN_USE = f"The console of {VM} is in use: another client holds it"
CONTAINER_REFUSED = f"{CONTAINER} is a container, and containers have no serial console."
# The text typed at the login prompt, checked for in the guest's echo. It is never submitted.
TYPED = "sherpa-console-check"


class TestFailure(Exception):
    """A check failed; the message says which and why."""


class Outcome(NamedTuple):
    """A finished command. Search `output` for messages, which may be on either stream."""

    code: int | None
    stdout: str
    stderr: str

    @property
    def output(self):
        return self.stdout + self.stderr


def require(condition, message):
    if not condition:
        raise TestFailure(message)


def run_logged(log, label, arguments, cwd, timeout=COMMAND_TIMEOUT, env=None):
    """Run a command, record it and its output in the log, and return its Outcome."""
    print(f"[{cwd.name}] {label}", flush=True)
    try:
        result = subprocess.run(arguments, cwd=cwd, capture_output=True, text=True, timeout=timeout,
                                env=env, stdin=subprocess.DEVNULL)
        outcome = Outcome(result.returncode, result.stdout, result.stderr)
    except subprocess.TimeoutExpired as error:
        partial = [part.decode(errors="replace") if isinstance(part, bytes) else part or ""
                   for part in (error.stdout, error.stderr)]
        outcome = Outcome(None, partial[0], f"{partial[1]}\ntimed out after {timeout}s")
    log.write(f"=== {label}: {shlex.join(arguments)}\n--- exit {outcome.code}\n"
              f"--- stdout\n{outcome.stdout}\n--- stderr\n{outcome.stderr}\n")
    log.flush()
    return outcome


def stop_on_sigterm(_signal, _frame):
    raise KeyboardInterrupt("SIGTERM")


def value_toml(value):
    if isinstance(value, str):
        return json.dumps(value)
    if isinstance(value, dict):
        return "{ " + ", ".join(f"{key} = {value_toml(item)}" for key, item in value.items()) + " }"
    raise TypeError(f"Unsupported TOML value: {value!r}")


def render_manifest(name):
    """The lab manifest for one run, under a unique lab name."""
    lines = [f"name = {value_toml(name)}", "", "nodes = ["]
    lines += [f"  {value_toml(node)}," for node in NODES]
    return "\n".join([*lines, "]"]) + "\n"


def path_without(tool):
    """PATH with every directory holding `tool` left out, so a check proves it is not needed.
    `sherpa` must stay reachable, so the result is checked by the caller."""
    kept = [entry for entry in os.environ.get("PATH", "").split(os.pathsep)
            if entry and not (Path(entry) / tool).exists()]
    return os.pathsep.join(kept)


class Console:
    """`sherpa console <node>` in a pseudo-terminal of its own, as a user's terminal runs it."""

    def __init__(self, log, label, sherpa, node, cwd, env):
        self.log = log
        self.label = label
        self.transcript = bytearray()
        master, slave = os.openpty()
        self.master = master
        self.process = subprocess.Popen([sherpa, "console", node], cwd=cwd, env=env, stdin=slave,
                                        stdout=slave, stderr=slave, start_new_session=True)
        os.close(slave)
        print(f"[{cwd.name}] {label}", flush=True)

    def read(self, timeout):
        """Read what the console printed within `timeout` seconds; False once it has closed."""
        ready, _, _ = select.select([self.master], [], [], timeout)
        if not ready:
            return True
        try:
            data = os.read(self.master, 4096)
        except OSError:
            return False
        self.transcript += data
        return bool(data)

    def expect(self, pattern, timeout, start=0):
        """Wait until the transcript from `start` matches `pattern`; return the end of the match."""
        deadline = time.monotonic() + timeout
        while True:
            found = pattern.search(self.transcript, start)
            if found:
                return found.end()
            remaining = deadline - time.monotonic()
            require(remaining > 0, f"{self.label}: {pattern.pattern!r} not seen within {timeout}s; "
                    f"see {self.log.name}")
            require(self.read(min(remaining, 1)) or self.process.poll() is None,
                    f"{self.label} ended before {pattern.pattern!r}; see {self.log.name}")

    def send(self, data):
        os.write(self.master, data)

    def finish(self, timeout=CONSOLE_TIMEOUT):
        """Wait for the command to end and return its exit code, keeping what it printed."""
        deadline = time.monotonic() + timeout
        while self.process.poll() is None and time.monotonic() < deadline:
            self.read(0.5)
        while self.read(0.1):
            pass
        code = self.process.poll()
        if code is None:
            self.process.kill()
            self.process.wait()
        os.close(self.master)
        self.log.write(f"=== {self.label}\n--- exit {code}\n--- terminal\n"
                       f"{self.text()}\n")
        self.log.flush()
        require(code is not None, f"{self.label} did not end within {timeout}s; see {self.log.name}")
        return code

    def text(self):
        return self.transcript.decode(errors="replace").replace("\r\n", "\n")


class LabRun:
    def __init__(self, run_root, keep_failed):
        self.name = f"console-{run_root.name[-6:]}"
        self.directory = run_root / "lab"
        self.keep_failed = keep_failed
        self.started = False
        self.lab_id = None
        self.log = None
        self.sherpa_path = shutil.which("sherpa")
        # Every console runs with no system `telnet` on PATH.
        self.console_env = {**os.environ, "PATH": path_without("telnet")}

    def sherpa(self, label, arguments, timeout=COMMAND_TIMEOUT, check=True):
        outcome = run_logged(self.log, label, [self.sherpa_path, *arguments], self.directory, timeout,
                             self.console_env)
        if check and outcome.code != 0:
            raise TestFailure(f"{label} failed (exit {outcome.code}); see {self.log.name}")
        return outcome

    def console(self, label, node=VM):
        return Console(self.log, label, self.sherpa_path, node, self.directory, self.console_env)

    def run(self):
        self.directory.mkdir(parents=True)
        with (self.directory / "commands.log").open("w") as self.log:
            (self.directory / "manifest.toml").write_text(render_manifest(self.name))
            failure = None
            try:
                self.up()
                self.checks()
            except (TestFailure, OSError, KeyError, ValueError) as error:
                failure = error
            except BaseException:
                # Interrupted: never leave the lab behind, then stop.
                self.destroy()
                raise
            if failure and self.keep_failed:
                print(f"keeping failed lab {self.name} ({self.lab_id}) in {self.directory}")
            else:
                cleanup = self.destroy()
                failure = failure or cleanup
        if failure:
            raise TestFailure(str(failure))

    def up(self):
        self.started = True
        outcome = self.sherpa("sherpa up", ["up"], UP_TIMEOUT, check=False)
        info_path = self.directory / ".sherpa" / "lab-info.toml"
        if info_path.exists():
            self.lab_id = tomllib.loads(info_path.read_text())["id"]
        require(outcome.code == 0, f"sherpa up failed (exit {outcome.code}); see {self.log.name}")
        require(self.lab_id is not None, "sherpa up wrote no .sherpa/lab-info.toml")

    def checks(self):
        self.check_container_refused()
        first = self.open_at_login("console, left with Ctrl+]")
        self.check_in_use()
        first.send(CTRL_RIGHT_BRACKET)
        self.check_closed(first)

        second = self.open_at_login("console, left with ~.")
        second.send(b"\r~.")
        self.check_closed(second)

    def check_container_refused(self):
        outcome = self.sherpa(f"console {CONTAINER}", ["console", CONTAINER], check=False)
        require(outcome.code != 0, f"sherpa console {CONTAINER} succeeded")
        require(CONTAINER_REFUSED in outcome.output,
                f"sherpa console {CONTAINER} did not say containers have no console; see {self.log.name}")

    def open_at_login(self, label):
        """A console on the VM at its login prompt, with typed text echoed back. Enter is pressed
        until the prompt shows, as the VM may still be booting."""
        deadline = time.monotonic() + LOGIN_TIMEOUT
        while True:
            console = self.console(label)
            try:
                console.expect(re.compile(re.escape(CONNECTED.encode())), CONSOLE_TIMEOUT)
                break
            except TestFailure:
                console.finish()
                require(time.monotonic() < deadline, f"{label}: the console never connected")
                time.sleep(POLL_INTERVAL)
        start = len(console.transcript)
        while True:
            console.send(b"\r")
            try:
                start = console.expect(LOGIN_PROMPT, POLL_INTERVAL, start)
                break
            except TestFailure:
                require(console.process.poll() is None, f"{label} ended; see {self.log.name}")
                require(time.monotonic() < deadline,
                        f"{label}: no login prompt within {LOGIN_TIMEOUT}s; see {self.log.name}")
        console.send(TYPED.encode())
        console.expect(re.compile(re.escape(TYPED.encode())), CONSOLE_TIMEOUT, start)
        # Clear the line, so the text is never submitted as a user name.
        console.send(b"\x15")
        return console

    def check_in_use(self):
        other = self.console("second console while one is open")
        code = other.finish()
        require(code != 0, "a second console on the node succeeded")
        require(IN_USE in other.text(), f"a second console did not report it in use; see {self.log.name}")

    def check_closed(self, console):
        code = console.finish()
        require(code == 0, f"{console.label} exited {code}; see {self.log.name}")
        require(console.text().count(f"Connection to the console of {VM} closed.") == 1,
                f"{console.label} did not print one close line; see {self.log.name}")
        require(CLOSED_AFTER_ESCAPE in console.text(),
                f"{console.label}: the close line did not start on a new line; see {self.log.name}")

    def destroy(self):
        """Destroy the lab and check it is gone. Returns a failure, or None."""
        if not self.started:
            return None
        try:
            self.sherpa("sherpa destroy", ["destroy", "--yes"], UP_TIMEOUT)
            if self.lab_id is not None:
                inspect = self.sherpa("lab gone", ["inspect"], check=False)
                require(inspect.code != 0, f"lab {self.lab_id} still exists after destroy")
        except TestFailure as error:
            return error
        return None


DEVICE_PASSWORD_SET = re.compile(r"^\s*Device password:\s*set\s*$", re.MULTILINE)
INFO_USERNAME = re.compile(r"^\s*Username:\s*(\S+)\s*$", re.MULTILINE)


def require_device_password(log, run_root):
    """Stop unless the lab owner has a device password, which `sherpa up` needs; return the owner."""
    info = run_logged(log, "sherpa server user info", ["sherpa", "server", "user", "info"], run_root)
    require(info.code == 0, "could not read the user's details (`sherpa server user info`)")
    require(DEVICE_PASSWORD_SET.search(info.stdout),
            "the user has no device password, which `sherpa up` needs: "
            "run `sherpa server user device-password`")
    owner = INFO_USERNAME.search(info.stdout)
    require(owner, "`sherpa server user info` did not name the user")
    return owner.group(1)


def preflight(run_root):
    require(shutil.which("sherpa"), "sherpa is not on PATH")
    require(shutil.which("sherpa", path=path_without("telnet")),
            "sherpa is in the same directory as telnet; install it elsewhere for this test")
    with (run_root / "preflight.log").open("w") as log:
        whoami = run_logged(log, "sherpa whoami", ["sherpa", "whoami"], run_root)
        require(whoami.code == 0, "the sherpa CLI is not logged in (run `sherpa login`)")
        require_device_password(log, run_root)
        for model, version in IMAGES:
            arguments = ["sherpa", "server", "image", "show", model]
            if version:
                arguments += ["--version", version]
            image = run_logged(log, f"{model} image present", arguments, run_root)
            require(image.code == 0, f"the server has no {model} {version or 'default'} image")


class SelfTest(unittest.TestCase):
    def test_device_password_check_reads_the_info_output(self):
        def info(text, code=0):
            return lambda *args, **kwargs: Outcome(code, text, "")

        with tempfile.TemporaryDirectory() as temp:
            run_root = Path(temp)
            ready = "\nUser Information:\n  Username: alice\n  Device password: set\n"
            with mock.patch(f"{__name__}.run_logged", info(ready)):
                self.assertEqual(require_device_password(None, run_root), "alice")
            for text, code in (("  Username: alice\n  Device password: not set\n", 0), (ready, 1)):
                with mock.patch(f"{__name__}.run_logged", info(text, code)):
                    with self.assertRaises(TestFailure):
                        require_device_password(None, run_root)

    def test_the_manifest_has_the_vm_and_the_container(self):
        manifest = tomllib.loads(render_manifest("console-abc123"))
        self.assertEqual(manifest["name"], "console-abc123")
        self.assertEqual([node["name"] for node in manifest["nodes"]], [VM, CONTAINER])
        self.assertEqual(manifest["nodes"][1]["image"], "alpine")

    def test_path_without_leaves_out_the_directories_holding_the_tool(self):
        directory = RUNS / f"self-test-{uuid.uuid4().hex[:8]}"
        (directory / "with").mkdir(parents=True)
        (directory / "without").mkdir()
        (directory / "with" / "telnet").touch()
        original = os.environ["PATH"]
        os.environ["PATH"] = os.pathsep.join([str(directory / "with"), str(directory / "without")])
        try:
            self.assertEqual(path_without("telnet"), str(directory / "without"))
        finally:
            os.environ["PATH"] = original
            shutil.rmtree(directory)


def main():
    parser = argparse.ArgumentParser(description=__doc__,
                                     formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--keep-failed", action="store_true", help="Leave a failed lab running for debugging")
    parser.add_argument("--self-test", action="store_true", help="Run the runner's own unit tests")
    args = parser.parse_args()
    if args.self_test:
        sys.argv = sys.argv[:1]
        unittest.main()
        return 0
    signal.signal(signal.SIGTERM, stop_on_sigterm)
    run_root = RUNS / f"console-{uuid.uuid4().hex[:12]}"
    run_root.mkdir(parents=True)
    print(f"Logs: {run_root}", flush=True)
    try:
        preflight(run_root)
        LabRun(run_root, args.keep_failed).run()
    except TestFailure as error:
        print(f"FAIL: {error}", file=sys.stderr)
        return 1
    except KeyboardInterrupt:
        print("interrupted", file=sys.stderr)
        return 130
    print("PASS")
    return 0


if __name__ == "__main__":
    sys.exit(main())
