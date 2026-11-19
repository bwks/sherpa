#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.15"
# dependencies = []
# ///
"""End-to-end test of `sherpa console` and `sherpa vnc` through the sherpa CLI (Python 3.15+,
stdlib only).

Brings up an Alpine VM and an Alpine container. Runs `sherpa console` on the VM in a
pseudo-terminal, as a user does: the console is relayed by the Sherpa server over its API, with no
system `telnet` or `ssh` on PATH. Checks the login prompt and typed input, leaving with Ctrl+] and
with `~.`, that a second console on the same node names who holds it, that `--force` takes it over,
that another Sherpa user and an admin are refused, and that the container is refused. Then runs
`sherpa vnc` on the VM and connects to its local address as a VNC viewer does: the server's RFB
banner comes back, a second `sherpa vnc` started then says who holds the console, a second viewer
is refused while the first is connected, the console is free again once it leaves, and Ctrl+C
stops the command; the other two accounts and the container are refused by `sherpa vnc` before it
prints a local address. Always destroys the lab. See README.md.
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
import socket
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
    {"name": CONTAINER, "model": "generic", "kind": "container", "image": "alpine", "version": "3.22"},
)
IMAGES = (("alpine_linux", None, None), ("generic", "container", "3.22"))

# The other two accounts: a normal user and an admin, neither owning the lab. Their passwords are
# only read from the environment.
ENV_OTHER_USER = "SHERPA_LAB_TEST_OTHER_USER"
ENV_OTHER_PASSWORD = "SHERPA_LAB_TEST_OTHER_PASSWORD"
ENV_ADMIN_USER = "SHERPA_LAB_TEST_ADMIN_USER"
ENV_ADMIN_PASSWORD = "SHERPA_LAB_TEST_ADMIN_PASSWORD"
# How `sherpa login` takes a user name and password without asking.
LOGIN_USER_ENV = "SHERPA_USER_NAME"
LOGIN_PASSWORD_ENV = "SHERPA_USER_PASSWORD"
# Tools a console must not need.
NOT_NEEDED = ("telnet", "ssh")

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
TAKEN_OVER = f"Connection to the console of {VM} closed: Taken over by "
IN_USE = f"The console of {VM} is in use by "
CONTAINER_REFUSED = f"{CONTAINER} is a container, and containers have no serial console."
# `sherpa vnc`: the line with the local address, the banner of QEMU's VNC server, and the messages.
VNC_LISTENING = re.compile(rf"VNC console of {VM} in lab .+: connect a VNC viewer to (127\.0\.0\.1:\d+)\. ")
# Any `sherpa vnc` listening line: a refused one must not print it.
VNC_ADDRESS = "connect a VNC viewer to"
RFB_BANNER = b"RFB 003.008\n"
VNC_IN_USE = f"The VNC console of {VM} is in use by "
VNC_CONTAINER_REFUSED = f"{CONTAINER} is a container, and containers have no VNC console."
VNC_STOPPED = f"Stopped relaying the VNC console of {VM}."
# The exit code of a `sherpa vnc` the server refused.
VNC_REFUSED_EXIT = 1
# How long `sherpa vnc` may take to print its address, to answer a viewer, or to end.
VNC_TIMEOUT = 60
# How long a finished command's remaining output is read for.
DRAIN_SECONDS = 5
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


def path_without(*tools):
    """PATH with every directory holding one of `tools` left out, so a check proves they are not
    needed. `sherpa` must stay reachable, so the result is checked by the caller."""
    kept = [entry for entry in os.environ.get("PATH", "").split(os.pathsep)
            if entry and not any((Path(entry) / tool).exists() for tool in tools)]
    return os.pathsep.join(kept)


def drain(read, limit=DRAIN_SECONDS):
    """Keep what a finished command printed: call `read` (which returns False at the end of the
    output) until the end, for at most `limit` seconds, so output held open by something else
    cannot hang the runner."""
    deadline = time.monotonic() + limit
    while time.monotonic() < deadline and read(0.1):
        pass


def refused_to(lab_id):
    """What the server says to anyone but the owner of lab `lab_id` asking for a console."""
    return f"Access denied: only the owner of lab '{lab_id}' can open its consoles"


class Account(NamedTuple):
    """A Sherpa account other than the lab's owner, logged in to a HOME of its own."""

    role: str
    username: str
    env: dict


def prepare_home(run_root, role):
    """A HOME for another account: the owner's client config and trusted server certificate, and
    no token."""
    home = run_root / f"{role}-home"
    (home / ".sherpa").mkdir(parents=True, mode=0o700)
    source = Path.home() / ".sherpa"
    for part in ("config", "trusted_certs"):
        if (source / part).is_dir():
            shutil.copytree(source / part, home / ".sherpa" / part)
    env = {key: value for key, value in os.environ.items() if key not in (LOGIN_USER_ENV, LOGIN_PASSWORD_ENV)}
    env["HOME"] = str(home)
    env["PATH"] = path_without(*NOT_NEEDED)
    return env


class Console:
    """`sherpa console <node>` in a pseudo-terminal of its own, as a user's terminal runs it."""

    def __init__(self, log, label, sherpa, node, cwd, env, options=()):
        self.log = log
        self.label = label
        self.transcript = bytearray()
        master, slave = os.openpty()
        self.master = master
        self.process = subprocess.Popen([sherpa, "console", node, *options], cwd=cwd, env=env, stdin=slave,
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
        code = self.process.poll()
        if code is None:
            self.process.kill()
            self.process.wait()
        drain(self.read)
        os.close(self.master)
        self.log.write(f"=== {self.label}\n--- exit {code}\n--- terminal\n"
                       f"{self.text()}\n")
        self.log.flush()
        require(code is not None, f"{self.label} did not end within {timeout}s; see {self.log.name}")
        return code

    def text(self):
        return self.transcript.decode(errors="replace").replace("\r\n", "\n")


class Vnc:
    """`sherpa vnc <node>`, run as a user runs it, with its local address read from its output."""

    def __init__(self, log, label, sherpa, node, cwd, env):
        self.log = log
        self.label = label
        self.output = bytearray()
        self.process = subprocess.Popen([sherpa, "vnc", node], cwd=cwd, env=env, stdin=subprocess.DEVNULL,
                                        stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
        print(f"[{cwd.name}] {label}", flush=True)
        found = self.expect(VNC_LISTENING, VNC_TIMEOUT)
        self.address = found.group(1).decode()

    def read(self, timeout):
        """Read what the command printed within `timeout` seconds; False once it has closed."""
        ready, _, _ = select.select([self.process.stdout], [], [], timeout)
        if not ready:
            return True
        try:
            data = os.read(self.process.stdout.fileno(), 4096)
        except OSError:
            return False
        self.output += data
        return bool(data)

    def expect(self, pattern, timeout):
        """Wait until the output matches `pattern`, a str pattern; return the match."""
        pattern = re.compile(pattern.pattern.encode()) if isinstance(pattern.pattern, str) else pattern
        deadline = time.monotonic() + timeout
        while True:
            found = pattern.search(self.output)
            if found:
                return found
            remaining = deadline - time.monotonic()
            require(remaining > 0, f"{self.label}: {pattern.pattern!r} not seen within {timeout}s; "
                    f"see {self.log.name}")
            require(self.read(min(remaining, 1)) or self.process.poll() is None,
                    f"{self.label} ended before {pattern.pattern!r}; see {self.log.name}")

    def viewer(self):
        """Connect to the local address as a VNC viewer; return the socket and the first bytes the
        server sent within the timeout, or what it sent before closing."""
        host, port = self.address.rsplit(":", 1)
        viewer = socket.create_connection((host, int(port)), VNC_TIMEOUT)
        viewer.settimeout(VNC_TIMEOUT)
        received = bytearray()
        while len(received) < len(RFB_BANNER):
            try:
                data = viewer.recv(len(RFB_BANNER) - len(received))
            except TimeoutError:
                break
            if not data:
                break
            received += data
        return viewer, bytes(received)

    def interrupt(self):
        """Ctrl+C, as a user stops it."""
        self.process.send_signal(signal.SIGINT)

    def finish(self, timeout=VNC_TIMEOUT):
        """Wait for the command to end and return its exit code, keeping what it printed."""
        deadline = time.monotonic() + timeout
        while self.process.poll() is None and time.monotonic() < deadline:
            self.read(0.5)
        code = self.process.poll()
        if code is None:
            self.process.kill()
            self.process.wait()
        drain(self.read)
        self.process.stdout.close()
        self.log.write(f"=== {self.label}\n--- exit {code}\n--- output\n{self.text()}\n")
        self.log.flush()
        require(code is not None, f"{self.label} did not end within {timeout}s; see {self.log.name}")
        return code

    def text(self):
        return self.output.decode(errors="replace")


class LabRun:
    def __init__(self, run_root, keep_failed, owner, others):
        self.name = f"console-{run_root.name[-6:]}"
        self.directory = run_root / "lab"
        self.run_root = run_root
        self.keep_failed = keep_failed
        self.owner = owner
        self.others = others
        self.started = False
        self.lab_id = None
        self.log = None
        self.sherpa_path = shutil.which("sherpa")
        # Every console runs with no system `telnet` or `ssh` on PATH.
        self.console_env = {**os.environ, "PATH": path_without(*NOT_NEEDED)}

    def sherpa(self, label, arguments, timeout=COMMAND_TIMEOUT, check=True):
        outcome = run_logged(self.log, label, [self.sherpa_path, *arguments], self.directory, timeout,
                             self.console_env)
        if check and outcome.code != 0:
            raise TestFailure(f"{label} failed (exit {outcome.code}); see {self.log.name}")
        return outcome

    def console(self, label, node=VM, options=()):
        return Console(self.log, label, self.sherpa_path, node, self.directory, self.console_env, options)

    def vnc(self, label):
        return Vnc(self.log, label, self.sherpa_path, VM, self.directory, self.console_env)

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
        for account in self.others:
            self.check_refused_to(account)
        first = self.open_at_login("console, taken over with --force")
        self.check_in_use()
        taken = self.open_at_login("console taking it over, left with Ctrl+]", ("--force",))
        self.check_taken_over(first)
        taken.send(CTRL_RIGHT_BRACKET)
        self.check_closed(taken)

        second = self.open_at_login("console, left with ~.")
        second.send(b"\r~.")
        self.check_closed(second)

        self.check_vnc_container_refused()
        for account in self.others:
            self.check_vnc_refused_to(account)
        self.check_vnc()

    def check_container_refused(self):
        outcome = self.sherpa(f"console {CONTAINER}", ["console", CONTAINER], check=False)
        require(outcome.code != 0, f"sherpa console {CONTAINER} succeeded")
        require(CONTAINER_REFUSED in outcome.output,
                f"sherpa console {CONTAINER} did not say containers have no console; see {self.log.name}")

    def check_refused_to(self, account):
        """Another Sherpa account, in a folder holding a copy of the owner's lab-info.toml, is
        refused the console by the server, before anything connects."""
        directory = self.run_root / account.role
        (directory / ".sherpa").mkdir(parents=True, exist_ok=True)
        shutil.copy(self.directory / ".sherpa" / "lab-info.toml", directory / ".sherpa" / "lab-info.toml")
        label = f"console as the {account.role} {account.username}"
        outcome = run_logged(self.log, label, [self.sherpa_path, "console", VM], directory, CONSOLE_TIMEOUT,
                             account.env)
        require(outcome.code not in (0, None), f"{label} succeeded")
        require(refused_to(self.lab_id) in outcome.output,
                f"{label} was not refused as not the owner; see {self.log.name}")

    def open_at_login(self, label, options=()):
        """A console on the VM at its login prompt, with typed text echoed back. Enter is pressed
        until the prompt shows, as the VM may still be booting."""
        deadline = time.monotonic() + LOGIN_TIMEOUT
        while True:
            console = self.console(label, options=options)
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
        require(f"{IN_USE}{self.owner} " in other.text(),
                f"a second console did not name who holds it; see {self.log.name}")
        require("--force" in other.text(), f"a second console did not offer --force; see {self.log.name}")

    def check_taken_over(self, console):
        """The console taken over ends, with code 0, saying who took it."""
        code = console.finish()
        require(code == 0, f"{console.label} exited {code}; see {self.log.name}")
        require(f"{TAKEN_OVER}{self.owner} " in console.text(),
                f"{console.label} did not say it was taken over; see {self.log.name}")

    def check_closed(self, console):
        code = console.finish()
        require(code == 0, f"{console.label} exited {code}; see {self.log.name}")
        require(console.text().count(f"Connection to the console of {VM} closed.") == 1,
                f"{console.label} did not print one close line; see {self.log.name}")
        require(CLOSED_AFTER_ESCAPE in console.text(),
                f"{console.label}: the close line did not start on a new line; see {self.log.name}")

    def check_vnc(self):
        """The owner's viewer gets the server's RFB banner; a second viewer is refused while the
        first is connected, naming who holds the console; the console is free again once the first
        leaves; Ctrl+C stops the command with code 0."""
        vnc = self.vnc("sherpa vnc as the owner")
        try:
            first, banner = vnc.viewer()
            require(banner == RFB_BANNER, f"the first viewer got {banner!r}, not the RFB banner; see {self.log.name}")
            self.check_vnc_held()
            second, refused = vnc.viewer()
            second.close()
            require(refused == b"", f"a second viewer got {refused!r} while the first was connected")
            vnc.expect(re.compile(re.escape(f"{VNC_IN_USE}{self.owner} ")), VNC_TIMEOUT)
            require(vnc.process.poll() is None, f"sherpa vnc ended after a refused viewer; see {self.log.name}")
            first.close()
            vnc.expect(re.compile(re.escape("disconnected from the VNC console of ")), VNC_TIMEOUT)
            # The server lets the console go once it has the first viewer's close, a moment later.
            deadline = time.monotonic() + VNC_TIMEOUT
            while True:
                again, banner = vnc.viewer()
                again.close()
                if banner == RFB_BANNER or time.monotonic() > deadline:
                    break
                time.sleep(1)
            require(banner == RFB_BANNER, f"a viewer after the first left got {banner!r}; see {self.log.name}")
        finally:
            vnc.interrupt()
            code = vnc.finish()
        require(code == 0, f"sherpa vnc exited {code} on Ctrl+C; see {self.log.name}")
        require(VNC_STOPPED in vnc.text(), f"sherpa vnc did not say it stopped; see {self.log.name}")

    def check_vnc_held(self):
        """A second `sherpa vnc` while a viewer holds the console is not refused: it prints its
        address, says the console is in use by the owner, and runs until Ctrl+C."""
        held = self.vnc("sherpa vnc while the console is held")
        try:
            held.expect(re.compile(re.escape(f"{VNC_IN_USE}{self.owner} ")), VNC_TIMEOUT)
            require(held.process.poll() is None, f"sherpa vnc ended on a held console; see {self.log.name}")
        finally:
            held.interrupt()
            code = held.finish()
        require(code == 0, f"sherpa vnc on a held console exited {code} on Ctrl+C; see {self.log.name}")

    def check_vnc_refused(self, label, node, cwd, env, message):
        """`sherpa vnc` asks the server before it opens a local port: refused, it ends with
        `message` and the refusal's exit code, and prints no address, so no viewer is needed."""
        outcome = run_logged(self.log, label, [self.sherpa_path, "vnc", node], cwd, VNC_TIMEOUT, env)
        require(outcome.code == VNC_REFUSED_EXIT,
                f"{label} exited {outcome.code}, not {VNC_REFUSED_EXIT}; see {self.log.name}")
        require(message in outcome.output, f"{label} did not say {message!r}; see {self.log.name}")
        require(VNC_ADDRESS not in outcome.output, f"{label} printed a local address; see {self.log.name}")

    def check_vnc_container_refused(self):
        self.check_vnc_refused(f"vnc {CONTAINER}", CONTAINER, self.directory, self.console_env,
                               VNC_CONTAINER_REFUSED)

    def check_vnc_refused_to(self, account):
        """Another Sherpa account is refused the VNC console by the server, as for the console."""
        self.check_vnc_refused(f"vnc as the {account.role} {account.username}", VM,
                               self.run_root / account.role, account.env, refused_to(self.lab_id))

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


INFO_ADMIN = re.compile(r"^\s*Admin:\s*(Yes|No)\s*$", re.MULTILINE)


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


def log_in(log, run_root, role, username, password, owner, admin):
    """Log `username` in to a HOME of its own and check it is another account than the owner's,
    and an admin when `admin` is set; return it."""
    require(username != owner, f"the {role} and the owner must be different accounts")
    env = prepare_home(run_root, role)
    login = run_logged(log, f"sherpa login ({role})", ["sherpa", "login"], run_root,
                       env={**env, LOGIN_USER_ENV: username, LOGIN_PASSWORD_ENV: password})
    require(login.code == 0, f"could not log in as {username}; see {log.name}")
    info = run_logged(log, f"sherpa server user info ({role})", ["sherpa", "server", "user", "info"], run_root,
                      env=env)
    found = INFO_ADMIN.search(info.stdout)
    require(info.code == 0 and found, f"could not read {username}'s details; see {log.name}")
    require((found.group(1) == "Yes") == admin, f"{username} is {'not ' if admin else ''}an admin")
    return Account(role, username, env)


def preflight(run_root, other_user, admin_user, passwords):
    """Check the server and the three accounts; return the owner and the other two accounts."""
    require(shutil.which("sherpa"), "sherpa is not on PATH")
    require(shutil.which("sherpa", path=path_without(*NOT_NEEDED)),
            "sherpa is in the same directory as telnet or ssh; install it elsewhere for this test")
    with (run_root / "preflight.log").open("w") as log:
        whoami = run_logged(log, "sherpa whoami", ["sherpa", "whoami"], run_root)
        require(whoami.code == 0, "the sherpa CLI is not logged in (run `sherpa login`)")
        owner = require_device_password(log, run_root)
        others = (log_in(log, run_root, "user", other_user, passwords[0], owner, admin=False),
                  log_in(log, run_root, "admin", admin_user, passwords[1], owner, admin=True))
        for model, kind, version in IMAGES:
            arguments = ["sherpa", "server", "image", "show", model]
            if kind:
                arguments += ["--kind", kind]
            if version:
                arguments += ["--version", version]
            image = run_logged(log, f"{model} image present", arguments, run_root)
            require(image.code == 0, f"the server has no {model} {version or 'default'} image")
    return owner, others


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

    def test_another_account_is_refused_in_the_servers_words(self):
        self.assertEqual(refused_to("abcd1234"),
                         "Access denied: only the owner of lab 'abcd1234' can open its consoles")

    def test_drain_stops_at_the_end_of_the_output_or_its_limit(self):
        calls = []
        drain(lambda timeout: calls.append(timeout) or len(calls) < 3)
        self.assertEqual(len(calls), 3)
        started = time.monotonic()
        drain(lambda timeout: True, limit=0.3)
        self.assertLess(time.monotonic() - started, 2)

    def test_the_vnc_address_is_read_from_the_listening_line(self):
        line = (f"VNC console of {VM} in lab console-abc123: connect a VNC viewer to 127.0.0.1:40123. "
                "Ctrl+C to stop.\n")
        self.assertEqual(VNC_LISTENING.search(line).group(1), "127.0.0.1:40123")
        self.assertIn(VNC_ADDRESS, line)

    def test_the_manifest_has_the_vm_and_the_container(self):
        manifest = tomllib.loads(render_manifest("console-abc123"))
        self.assertEqual(manifest["name"], "console-abc123")
        self.assertEqual([node["name"] for node in manifest["nodes"]], [VM, CONTAINER])
        self.assertEqual(manifest["nodes"][1]["image"], "alpine")

    def test_path_without_leaves_out_the_directories_holding_the_tools(self):
        directory = RUNS / f"self-test-{uuid.uuid4().hex[:8]}"
        for name in ("telnet", "ssh", "without"):
            (directory / name).mkdir(parents=True)
        (directory / "telnet" / "telnet").touch()
        (directory / "ssh" / "ssh").touch()
        original = os.environ["PATH"]
        os.environ["PATH"] = os.pathsep.join(str(directory / name) for name in ("telnet", "ssh", "without"))
        try:
            self.assertEqual(path_without(*NOT_NEEDED), str(directory / "without"))
        finally:
            os.environ["PATH"] = original
            shutil.rmtree(directory)


def main():
    parser = argparse.ArgumentParser(description=__doc__,
                                     formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--other-user", default=os.environ.get(ENV_OTHER_USER),
                        help=f"A normal user who does not own the lab (default: ${ENV_OTHER_USER}). Its "
                             f"password is read from ${ENV_OTHER_PASSWORD}")
    parser.add_argument("--admin-user", default=os.environ.get(ENV_ADMIN_USER),
                        help=f"An admin who does not own the lab (default: ${ENV_ADMIN_USER}). Its "
                             f"password is read from ${ENV_ADMIN_PASSWORD}")
    parser.add_argument("--keep-failed", action="store_true", help="Leave a failed lab running for debugging")
    parser.add_argument("--self-test", action="store_true", help="Run the runner's own unit tests")
    args = parser.parse_args()
    if args.self_test:
        sys.argv = sys.argv[:1]
        unittest.main()
        return 0
    passwords = (os.environ.get(ENV_OTHER_PASSWORD), os.environ.get(ENV_ADMIN_PASSWORD))
    missing = [name for name, value in ((f"--other-user or ${ENV_OTHER_USER}", args.other_user),
                                        (f"${ENV_OTHER_PASSWORD}", passwords[0]),
                                        (f"--admin-user or ${ENV_ADMIN_USER}", args.admin_user),
                                        (f"${ENV_ADMIN_PASSWORD}", passwords[1])) if not value]
    if missing:
        print(f"FAIL: missing {', '.join(missing)}", file=sys.stderr)
        return 1
    signal.signal(signal.SIGTERM, stop_on_sigterm)
    run_root = RUNS / f"console-{uuid.uuid4().hex[:12]}"
    run_root.mkdir(parents=True)
    print(f"Logs: {run_root}", flush=True)
    try:
        owner, others = preflight(run_root, args.other_user, args.admin_user, passwords)
        LabRun(run_root, args.keep_failed, owner, others).run()
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
