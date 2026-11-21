#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.15"
# dependencies = ["pytest==9.2.0"]
# ///
"""End-to-end test of `sherpa console` and `sherpa vnc` through the sherpa CLI (Python 3.15+,
pytest).

Brings up an Alpine VM and an Alpine container, and checks the e2e cases of
tests/specs/consoles.md. `sherpa console` runs on the VM in a pseudo-terminal, as a user does: the
console is relayed by the Sherpa server over its API, with no system `telnet` or `ssh` on PATH.
`sherpa vnc` runs on the VM and is connected to as a VNC viewer does. The cases: the login prompt
and typed input, leaving with Ctrl+] and with `~.`; the VNC banner, Ctrl+C, a second `sherpa vnc`
and a second viewer while the console is held, and a viewer after the first left; another Sherpa
user, and an admin, refused; a second console naming who holds it, and `--force`; the container
refused both consoles. Always destroys the lab. See README.md.

A pytest module: the `prepared` fixture checks the server and the accounts and writes the lab's
manifest, the `lab` fixture brings the lab up once for the module and destroys it at the end, and
the function fixtures `at_login`, `vnc` and `held_vnc` give a case the console it starts from and
end it after. The cases are the `test_<case_id>_*` functions, in case order. Run as a script, it
runs pytest on itself; `--self-test` runs the runner's own unit tests instead.
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
import time
import tomllib
from typing import NamedTuple
import unittest
import uuid

import pytest


LAB = Path(__file__).resolve().parent
ROOT = LAB.parent.parent.parent
RUNS = ROOT / ".tmp" / "lab-tests"
VM = "alp01"
CONTAINER = "ctr01"
# The lab, as `render_manifest` writes it for each run under a unique name. `--no-container`
# leaves the container out.
VM_NODE = {"name": VM, "model": "alpine_linux"}
CONTAINER_NODE = {"name": CONTAINER, "model": "generic", "kind": "container", "image": "alpine",
                  "version": "3.22"}
VM_IMAGE = ("alpine_linux", None, None)
CONTAINER_IMAGE = ("generic", "container", "3.22")

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
CLOSED = f"Connection to the console of {VM} closed."
CLOSED_AFTER_ESCAPE = f"\n{CLOSED}"
TAKEN_OVER = f"Connection to the console of {VM} closed: Taken over by "
IN_USE = f"The console of {VM} is in use by "
CONTAINER_REFUSED = f"{CONTAINER} is a container, and containers have no serial console."
# `sherpa vnc`: the line with the local address, the banner of QEMU's VNC server, and the messages.
VNC_LISTENING = re.compile(rf"VNC console of {VM} in lab .+: connect a VNC viewer to (\S+:\d+)\. ")
VNC_LOCAL_HOST = "127.0.0.1"
# Any `sherpa vnc` listening line: a refused one must not print it.
VNC_ADDRESS = "connect a VNC viewer to"
RFB_BANNER = b"RFB 003.008\n"
VNC_IN_USE = f"The VNC console of {VM} is in use by "
VNC_VIEWER_LEFT = f"disconnected from the VNC console of {VM}."
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
NO_CONTAINER = "--no-container: the lab has no container"


class TestFailure(Exception):
    """A check failed; the message says which and why."""

    __test__ = False


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
    except KeyboardInterrupt:
        log.write(f"=== {label}: {shlex.join(arguments)}\n--- interrupted\n")
        log.flush()
        raise
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


def render_manifest(name, nodes):
    """The lab manifest for one run, under a unique lab name."""
    lines = [f"name = {value_toml(name)}", "", "nodes = ["]
    lines += [f"  {value_toml(node)}," for node in nodes]
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


def connected_line(lab_name):
    """The line `sherpa console` prints once the console is open: the node, the lab and both
    escapes."""
    return f"{CONNECTED}{lab_name}. Escape: Ctrl+] or ~. at the start of a line."


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
        self.finished = False
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
        self.finished = True
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
        self.finished = False
        self.process = subprocess.Popen([sherpa, "vnc", node], cwd=cwd, env=env, stdin=subprocess.DEVNULL,
                                        stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
        print(f"[{cwd.name}] {label}", flush=True)
        try:
            found = self.expect(VNC_LISTENING, VNC_TIMEOUT)
        except TestFailure:
            self.finish()
            raise
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
        self.finished = True
        self.log.write(f"=== {self.label}\n--- exit {code}\n--- output\n{self.text()}\n")
        self.log.flush()
        require(code is not None, f"{self.label} did not end within {timeout}s; see {self.log.name}")
        return code

    def text(self):
        return self.output.decode(errors="replace")



# -- the lab -------------------------------------------------------------------------------------


class Lab:
    """The lab every case uses: its manifest written by `prepare`, brought up once by `up` and
    always destroyed by `destroy`."""

    def __init__(self, run_root, owner, user, admin, with_container):
        self.name = f"console-{run_root.name[-6:]}"
        self.directory = run_root / "lab"
        self.run_root = run_root
        self.owner = owner
        self.user = user
        self.admin = admin
        self.nodes = (VM_NODE, CONTAINER_NODE) if with_container else (VM_NODE,)
        self.with_container = with_container
        self.started = False
        self.lab_id = None
        self.log = None
        self.sherpa_path = shutil.which("sherpa")
        # Every console runs with no system `telnet` or `ssh` on PATH.
        self.console_env = {**os.environ, "PATH": path_without(*NOT_NEEDED)}

    # -- commands ----------------------------------------------------------------------------

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

    def folder_of(self, account):
        """A folder of the account's own holding a copy of the owner's lab-info.toml, as if the
        owner's lab folder had been copied to them."""
        directory = self.run_root / account.role
        (directory / ".sherpa").mkdir(parents=True, exist_ok=True)
        shutil.copy(self.directory / ".sherpa" / "lab-info.toml", directory / ".sherpa" / "lab-info.toml")
        return directory

    def as_account(self, account, label, arguments, timeout):
        """Run `sherpa` as another account, in its folder of the lab."""
        return run_logged(self.log, label, [self.sherpa_path, *arguments], self.folder_of(account), timeout,
                          account.env)

    # -- lifecycle ---------------------------------------------------------------------------

    def prepare(self):
        self.directory.mkdir(parents=True)
        self.log = (self.directory / "commands.log").open("w")
        (self.directory / "manifest.toml").write_text(render_manifest(self.name, self.nodes))

    def up(self):
        self.started = True
        outcome = self.sherpa("sherpa up", ["up"], UP_TIMEOUT, check=False)
        # A partly created lab exits non-zero after writing lab-info.toml: record its ID first, so
        # destroy can check the lab is gone.
        info_path = self.directory / ".sherpa" / "lab-info.toml"
        if info_path.exists():
            self.lab_id = tomllib.loads(info_path.read_text())["id"]
        require(outcome.code == 0, f"sherpa up failed (exit {outcome.code}); see {self.log.name}")
        require(self.lab_id is not None, "sherpa up wrote no .sherpa/lab-info.toml")

    def destroy(self):
        """Destroy the lab and check it is gone. Returns a failure, or None."""
        if not self.started:
            return None
        if self.lab_id is None:
            # `sherpa up` failed early; destroy whatever it created, found by the manifest's lab name.
            self.sherpa("sherpa destroy (after failed up)", ["destroy", "--yes"], UP_TIMEOUT, check=False)
            return None
        try:
            self.sherpa("sherpa destroy", ["destroy", "--yes"], UP_TIMEOUT)
            inspect = self.sherpa("lab gone", ["inspect"], check=False)
            require(inspect.code != 0, f"lab {self.lab_id} still exists after destroy")
        except TestFailure as error:
            return error
        return None


# -- helpers of the cases ------------------------------------------------------------------------


def open_at_login(lab, label, options=()):
    """A console on the VM at its login prompt, with typed text echoed back. It is opened again
    until it connects, as the server may not yet have let go of a session that just ended, and
    Enter is pressed until the prompt shows, as the VM may still be booting."""
    deadline = time.monotonic() + LOGIN_TIMEOUT
    while True:
        console = lab.console(label, options=options)
        try:
            console.expect(re.compile(re.escape(CONNECTED.encode())), CONSOLE_TIMEOUT)
            break
        except TestFailure:
            console.finish()
            require(time.monotonic() < deadline, f"{label}: the console never connected")
            time.sleep(POLL_INTERVAL)
    try:
        start = len(console.transcript)
        while True:
            console.send(b"\r")
            try:
                start = console.expect(LOGIN_PROMPT, POLL_INTERVAL, start)
                break
            except TestFailure:
                require(console.process.poll() is None, f"{label} ended; see {lab.log.name}")
                require(time.monotonic() < deadline,
                        f"{label}: no login prompt within {LOGIN_TIMEOUT}s; see {lab.log.name}")
        console.send(TYPED.encode())
        console.expect(re.compile(re.escape(TYPED.encode())), CONSOLE_TIMEOUT, start)
        # Clear the line, so the text is never submitted as a user name.
        console.send(b"\x15")
    except BaseException:
        leave(console)
        raise
    return console


def leave(console):
    """End a console a case left open, with Ctrl+], so the next case finds the console free. What
    it shows is not checked."""
    if console.finished:
        return
    if console.process.poll() is None:
        console.send(CTRL_RIGHT_BRACKET)
    try:
        console.finish()
    except TestFailure:
        pass


def require_closed(lab, console):
    """The console, left with an escape, ends with code 0 and one close line, on a new line."""
    code = console.finish()
    require(code == 0, f"{console.label} exited {code}; see {lab.log.name}")
    require(console.text().count(CLOSED) == 1, f"{console.label} did not print one close line; see {lab.log.name}")
    require(CLOSED_AFTER_ESCAPE in console.text(),
            f"{console.label}: the close line did not start on a new line; see {lab.log.name}")


def viewer_with_banner(vnc):
    """A viewer of `vnc` and what it got: the RFB banner, or what the last viewer got when none
    got it within the timeout. The server lets the console go a moment after a viewer leaves, so
    a viewer is tried again until then."""
    deadline = time.monotonic() + VNC_TIMEOUT
    while True:
        viewer, banner = vnc.viewer()
        if banner == RFB_BANNER or time.monotonic() > deadline:
            return viewer, banner
        viewer.close()
        time.sleep(1)


def stop(vnc):
    """Ctrl+C a `sherpa vnc` a case left running. What it shows is not checked."""
    if vnc.finished:
        return
    vnc.interrupt()
    try:
        vnc.finish()
    except TestFailure:
        pass


def require_vnc_refused(lab, label, outcome, message):
    """`sherpa vnc` asks the server before it opens a local port: refused, it ends with `message`
    and the refusal's exit code, and prints no address, so no viewer is needed."""
    require(outcome.code == VNC_REFUSED_EXIT,
            f"{label} exited {outcome.code}, not {VNC_REFUSED_EXIT}; see {lab.log.name}")
    require(message in outcome.output, f"{label} did not say {message!r}; see {lab.log.name}")
    require(VNC_ADDRESS not in outcome.output, f"{label} printed a local address; see {lab.log.name}")


# -- the fixtures --------------------------------------------------------------------------------


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


def preflight(run_root, other_user, admin_user, passwords, with_container):
    """Check the server and the three accounts; return the owner and the other two accounts."""
    require(shutil.which("sherpa"), "sherpa is not on PATH")
    require(shutil.which("sherpa", path=path_without(*NOT_NEEDED)),
            "sherpa is in the same directory as telnet or ssh; install it elsewhere for this test")
    with (run_root / "preflight.log").open("w") as log:
        whoami = run_logged(log, "sherpa whoami", ["sherpa", "whoami"], run_root)
        require(whoami.code == 0, "the sherpa CLI is not logged in (run `sherpa login`)")
        owner = require_device_password(log, run_root)
        user = log_in(log, run_root, "user", other_user, passwords[0], owner, admin=False)
        admin = log_in(log, run_root, "admin", admin_user, passwords[1], owner, admin=True)
        images = (VM_IMAGE, CONTAINER_IMAGE) if with_container else (VM_IMAGE,)
        for model, kind, version in images:
            arguments = ["sherpa", "server", "image", "show", model]
            if kind:
                arguments += ["--kind", kind]
            if version:
                arguments += ["--version", version]
            image = run_logged(log, f"{model} image present", arguments, run_root)
            require(image.code == 0, f"the server has no {model} {version or 'default'} image")
    return owner, user, admin


@pytest.fixture(scope="module")
def prepared(request):
    """The lab before it is up: the server and the three accounts checked and the run's manifest
    written."""
    signal.signal(signal.SIGTERM, stop_on_sigterm)
    other_user = request.config.getoption("other_user")
    admin_user = request.config.getoption("admin_user")
    passwords = (os.environ.get(ENV_OTHER_PASSWORD), os.environ.get(ENV_ADMIN_PASSWORD))
    missing = [name for name, value in ((f"--other-user or ${ENV_OTHER_USER}", other_user),
                                        (f"${ENV_OTHER_PASSWORD}", passwords[0]),
                                        (f"--admin-user or ${ENV_ADMIN_USER}", admin_user),
                                        (f"${ENV_ADMIN_PASSWORD}", passwords[1])) if not value]
    require(not missing, f"missing {', '.join(missing)}")
    with_container = not request.config.getoption("no_container")
    run_root = RUNS / f"console-{uuid.uuid4().hex[:12]}"
    run_root.mkdir(parents=True)
    print(f"Logs: {run_root}", flush=True)
    owner, user, admin = preflight(run_root, other_user, admin_user, passwords, with_container)
    lab = Lab(run_root, owner, user, admin, with_container)
    lab.prepare()
    try:
        yield lab
    finally:
        lab.log.close()


@pytest.fixture(scope="module")
def lab(request, prepared):
    """The lab every case uses: brought up once and destroyed at the end, even when a case fails,
    unless `--keep-failed` is given and something failed."""
    keep_failed = request.config.getoption("keep_failed")
    keep = False
    try:
        try:
            prepared.up()
        except Exception:
            keep = keep_failed
            raise
        yield prepared
        keep = keep_failed and request.session.testsfailed > 0
    finally:
        if keep:
            print(f"keeping failed lab {prepared.name} ({prepared.lab_id}) in {prepared.directory}", flush=True)
        else:
            cleanup = prepared.destroy()
            if cleanup:
                raise cleanup


@pytest.fixture
def at_login(lab):
    """The owner's console on the VM, open at its login prompt; ended after the case if the case
    left it open."""
    console = open_at_login(lab, "console at the login prompt")
    try:
        yield console
    finally:
        leave(console)


@pytest.fixture
def vnc(lab):
    """The owner's `sherpa vnc` on the VM, listening; stopped after the case if the case left it
    running."""
    command = lab.vnc("sherpa vnc as the owner")
    try:
        yield command
    finally:
        stop(command)


@pytest.fixture
def held_vnc(lab, vnc):
    """`vnc` with a viewer connected that got the RFB banner, so the VNC console is held."""
    viewer, banner = viewer_with_banner(vnc)
    try:
        require(banner == RFB_BANNER, f"the holding viewer got {banner!r}, not the RFB banner; see {lab.log.name}")
        yield vnc
    finally:
        viewer.close()


# -- cases ---------------------------------------------------------------------------------------


def test_serial_console_001_the_owner_opens_a_vms_console_with_no_telnet_or_ssh(lab, at_login):
    # `at_login` ran it with no `telnet` or `ssh` on PATH, saw the login prompt and the typed text
    # echoed.
    require(connected_line(lab.name) in at_login.text(),
            f"the connect line does not name {VM}, lab {lab.name} and both escapes; see {lab.log.name}")


def test_serial_console_002_ctrl_right_bracket_ends_the_console_with_exit_code_0(lab, at_login):
    at_login.send(CTRL_RIGHT_BRACKET)
    require_closed(lab, at_login)


def test_serial_console_003_tilde_dot_at_the_start_of_a_line_ends_it_the_same_way(lab, at_login):
    at_login.send(b"\r~.")
    require_closed(lab, at_login)


def test_vnc_console_001_a_viewer_of_the_local_address_gets_the_nodes_rfb_banner(lab, vnc):
    host, _, _ = vnc.address.rpartition(":")
    require(host == VNC_LOCAL_HOST, f"sherpa vnc listens on {vnc.address}, not on {VNC_LOCAL_HOST}")
    viewer, banner = viewer_with_banner(vnc)
    viewer.close()
    require(banner == RFB_BANNER, f"the viewer got {banner!r}, not the RFB banner; see {lab.log.name}")


def test_vnc_console_002_ctrl_c_stops_the_command_with_exit_code_0_and_its_stop_line(lab, vnc):
    vnc.interrupt()
    code = vnc.finish()
    require(code == 0, f"sherpa vnc exited {code} on Ctrl+C; see {lab.log.name}")
    require(VNC_STOPPED in vnc.text(), f"sherpa vnc did not say it stopped; see {lab.log.name}")


def test_vnc_console_005_started_on_a_held_console_it_says_who_holds_it_and_keeps_listening(lab, held_vnc):
    second = lab.vnc("sherpa vnc while the console is held")
    try:
        second.expect(re.compile(re.escape(f"{VNC_IN_USE}{lab.owner} ")), VNC_TIMEOUT)
        require("--force" in second.text(), f"sherpa vnc on a held console did not offer --force; see {lab.log.name}")
        require(second.process.poll() is None, f"sherpa vnc ended on a held console; see {lab.log.name}")
    finally:
        stop(second)


def test_vnc_console_006_a_viewer_of_a_held_console_gets_nothing_and_the_command_goes_on(lab, held_vnc):
    viewer, received = held_vnc.viewer()
    viewer.close()
    require(received == b"", f"a second viewer got {received!r} while the first was connected")
    held_vnc.expect(re.compile(re.escape(f"{VNC_IN_USE}{lab.owner} ")), VNC_TIMEOUT)
    require(held_vnc.process.poll() is None, f"sherpa vnc ended after a refused viewer; see {lab.log.name}")


def test_vnc_console_007_once_a_viewer_leaves_the_next_one_gets_the_banner(lab, vnc):
    first, banner = viewer_with_banner(vnc)
    first.close()
    require(banner == RFB_BANNER, f"the first viewer got {banner!r}, not the RFB banner; see {lab.log.name}")
    vnc.expect(re.compile(re.escape(VNC_VIEWER_LEFT)), VNC_TIMEOUT)
    again, banner = viewer_with_banner(vnc)
    again.close()
    require(banner == RFB_BANNER, f"a viewer after the first left got {banner!r}; see {lab.log.name}")


def test_console_access_001_another_user_is_refused_the_console_as_not_the_owner(lab):
    label = f"console as the user {lab.user.username}"
    outcome = lab.as_account(lab.user, label, ["console", VM], CONSOLE_TIMEOUT)
    require(outcome.code not in (0, None), f"{label} succeeded")
    require(refused_to(lab.lab_id) in outcome.output, f"{label} was not refused as not the owner; see {lab.log.name}")


def test_console_access_002_another_user_and_an_admin_are_refused_vnc_with_no_address(lab):
    for account in (lab.user, lab.admin):
        label = f"vnc as the {account.role} {account.username}"
        outcome = lab.as_account(account, label, ["vnc", VM], VNC_TIMEOUT)
        require_vnc_refused(lab, label, outcome, refused_to(lab.lab_id))


def test_console_session_001_a_second_console_fails_naming_the_holder_and_offering_force(lab, at_login):
    other = lab.console("second console while one is open")
    code = other.finish()
    require(code != 0, "a second console on the node succeeded")
    require(f"{IN_USE}{lab.owner} " in other.text(), f"a second console did not name who holds it; see {lab.log.name}")
    require("--force" in other.text(), f"a second console did not offer --force; see {lab.log.name}")


def test_console_session_002_force_takes_the_console_over_and_the_old_session_says_who(lab, at_login):
    taken = open_at_login(lab, "console taking it over with --force", ("--force",))
    try:
        code = at_login.finish()
        require(code == 0, f"{at_login.label} exited {code} once taken over; see {lab.log.name}")
        require(f"{TAKEN_OVER}{lab.owner} " in at_login.text(),
                f"{at_login.label} did not say it was taken over by {lab.owner}; see {lab.log.name}")
    finally:
        leave(taken)


def test_console_refusal_001_a_container_is_refused_the_console_pointing_to_ssh(lab):
    if not lab.with_container:
        pytest.skip(NO_CONTAINER)
    outcome = lab.sherpa(f"console {CONTAINER}", ["console", CONTAINER], CONSOLE_TIMEOUT, check=False)
    require(outcome.code not in (0, None), f"sherpa console {CONTAINER} succeeded")
    require(CONTAINER_REFUSED in outcome.output,
            f"sherpa console {CONTAINER} did not say containers have no console; see {lab.log.name}")
    require("sherpa ssh" in outcome.output, f"sherpa console {CONTAINER} did not point to sherpa ssh; see {lab.log.name}")


def test_console_refusal_002_a_container_is_refused_vnc_with_exit_code_1_and_no_address(lab):
    if not lab.with_container:
        pytest.skip(NO_CONTAINER)
    label = f"vnc {CONTAINER}"
    outcome = lab.sherpa(label, ["vnc", CONTAINER], VNC_TIMEOUT, check=False)
    require_vnc_refused(lab, label, outcome, VNC_CONTAINER_REFUSED)


# -- running -------------------------------------------------------------------------------------


def main():
    parser = argparse.ArgumentParser(description=__doc__,
                                     formatter_class=argparse.RawDescriptionHelpFormatter,
                                     epilog="Other arguments, such as --other-user, --admin-user, --keep-failed, "
                                            "--no-container or -k, are passed to pytest.")
    parser.add_argument("--self-test", action="store_true", help="Run the runner's own unit tests")
    args, pytest_args = parser.parse_known_args()
    if args.self_test:
        sys.argv = sys.argv[:1]
        unittest.main()
        return 0
    return pytest.main(["-v", "-p", "no:cacheprovider", __file__, *pytest_args])


class SelfTest(unittest.TestCase):
    # The runner's own checks, run with --self-test; pytest must not collect them as lab cases.
    __test__ = False

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


if __name__ == "__main__":
    sys.exit(main())
