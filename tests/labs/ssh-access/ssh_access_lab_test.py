#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.15"
# dependencies = ["pytest==9.2.0"]
# ///
"""End-to-end test of how `sherpa ssh` and SSH provisioners reach a lab's nodes (Python 3.15+,
pytest).

Brings up one Alpine VM with an SSH provisioner and checks the `ssh_access` cases of
tests/specs/client-lab-files.md: the lab's SSH config reaches the node directly; `sherpa ssh` and
`sherpa provision` connect to the node themselves; with `--ssh-jump` they connect only to the
Sherpa server, whose SSH server opens the connection to the node. Which process holds the
connection the node sees (its `$SSH_CONNECTION`) is read with `ss`, so the runner must run on a
machine with a route to the lab's management network, such as the Sherpa server. Always destroys
the lab. See README.md.

A pytest module: the `lab` fixture brings the lab up once for the module and destroys it at the
end, and the cases are the `test_ssh_access_<NNN>_*` functions taking it, in case order. Run as a
script, it runs pytest on itself; `--self-test` runs the runner's own unit tests instead.
"""

import argparse
import json
import os
from pathlib import Path
import pty
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
import urllib.parse
import uuid

import pytest


LAB = Path(__file__).resolve().parent
ROOT = LAB.parent.parent.parent
RUNS = ROOT / ".tmp" / "lab-tests"
NODE = "alp01"
# The lab, as `render_manifest` writes it for each run under a unique name. The provisioner's
# script prints the connection the node sees, then holds it open long enough for `ss` to see it.
NODES = ({"name": NODE, "model": "alpine_linux"},)
PROVISION_SCRIPT = "connection.sh"
PROVISION_MARKER = "ssh-access-connection"
HOLD_SECONDS = 5
PROVISION_SCRIPT_CONTENT = f'#!/bin/sh\necho "{PROVISION_MARKER} $SSH_CONNECTION"\nsleep {HOLD_SECONDS}\n'
# Typed into the `sherpa ssh` shell: the same marker, then a pause and a distinctive exit code.
SHELL_EXIT = 7
SHELL_COMMAND = f'echo "{PROVISION_MARKER} $SSH_CONNECTION"; sleep {HOLD_SECONDS}; exit {SHELL_EXIT}\r'
SSH_PORT = 22

COMMAND_TIMEOUT = 180
UP_TIMEOUT = 1200
SSH_TIMEOUT = 120
POLL_SECONDS = 0.1
CONNECTED = "Connected to "
SERVER_URL = re.compile(r"Connecting to server: (\S+)")
CONNECTION = re.compile(rf"{PROVISION_MARKER} (\S+) (\d+) (\S+) (\d+)")
PID = re.compile(r"pid=(\d+),")


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


class Socket(NamedTuple):
    """An established TCP connection on this machine, and the processes holding it that this user
    may see; none for another user's, such as the server's SSH server for the jump."""

    local: tuple
    peer: tuple
    pids: frozenset


class Observed(NamedTuple):
    """What a command did: its outcome, and every TCP connection to an SSH port seen while it ran."""

    pid: int
    outcome: Outcome
    sockets: frozenset


def require(condition, message):
    if not condition:
        raise TestFailure(message)


def run_logged(log, label, arguments, cwd, timeout=COMMAND_TIMEOUT):
    """Run a command, record it and its output in the log, and return its Outcome."""
    print(f"[{cwd.name}] {label}", flush=True)
    try:
        result = subprocess.run(arguments, cwd=cwd, capture_output=True, text=True, timeout=timeout,
                                stdin=subprocess.DEVNULL)
        outcome = Outcome(result.returncode, result.stdout, result.stderr)
    except subprocess.TimeoutExpired as error:
        partial = [part.decode(errors="replace") if isinstance(part, bytes) else part or ""
                   for part in (error.stdout, error.stderr)]
        outcome = Outcome(None, partial[0], f"{partial[1]}\ntimed out after {timeout}s")
    log_outcome(log, label, arguments, outcome)
    return outcome


def log_outcome(log, label, arguments, outcome):
    log.write(f"=== {label}: {shlex.join(arguments)}\n--- exit {outcome.code}\n"
              f"--- stdout\n{outcome.stdout}\n--- stderr\n{outcome.stderr}\n")
    log.flush()


def stop_on_sigterm(_signal, _frame):
    raise KeyboardInterrupt("SIGTERM")


def value_toml(value):
    if isinstance(value, str):
        return json.dumps(value)
    if isinstance(value, list):
        return "[" + ", ".join(value_toml(item) for item in value) + "]"
    if isinstance(value, dict):
        return "{ " + ", ".join(f"{key} = {value_toml(item)}" for key, item in value.items()) + " }"
    raise TypeError(f"Unsupported TOML value: {value!r}")


def render_manifest(name):
    """The lab manifest for one run, under a unique lab name: the node with its SSH provisioner."""
    lines = [f"name = {value_toml(name)}", ""]
    for node in NODES:
        lines += ["[[nodes]]", *(f"{key} = {value_toml(item)}" for key, item in node.items()), "",
                  "[[nodes.provisioners]]", 'type = "ssh"', f"scripts = {value_toml([f'./{PROVISION_SCRIPT}'])}",
                  ""]
    return "\n".join(lines)


def parse_ssh_hosts(config):
    """The `Host` blocks of an SSH config, each a dict of its settings with lower-case keys and
    its aliases under `host`."""
    blocks = []
    for line in config.splitlines():
        key, _, value = line.strip().partition(" ")
        if not key or key.startswith("#"):
            continue
        if key.lower() == "host":
            blocks.append({"host": tuple(value.split())})
        elif blocks:
            blocks[-1][key.lower()] = value.strip()
    return blocks


def endpoint(text):
    """`(address, port)` from `ss`'s `address:port`, IPv6 addresses in brackets."""
    address, _, port = text.rpartition(":")
    return address.strip("[]"), int(port)


def parse_sockets(text):
    """The connections in `ss -tnpH state established` output."""
    sockets = []
    for line in text.splitlines():
        fields = line.split()
        if len(fields) < 4:
            continue
        sockets.append(Socket(endpoint(fields[2]), endpoint(fields[3]),
                              frozenset(int(pid) for pid in PID.findall(line))))
    return sockets


def node_connection(output):
    """`(client address, client port)` of the connection the node reported in `output`."""
    found = CONNECTION.search(output)
    require(found, f"the node did not report its connection: {output.strip()[-400:]}")
    return found.group(1), int(found.group(2))


def server_host(inspect_output):
    """The Sherpa server's host, from the server URL `sherpa inspect` prints."""
    found = SERVER_URL.search(inspect_output)
    require(found, "`sherpa inspect` did not print the server URL")
    host = urllib.parse.urlsplit(found.group(1)).hostname
    require(host, f"the server URL {found.group(1)} names no host")
    return host


def ssh_sockets():
    """The established TCP connections to an SSH port on this machine."""
    listing = subprocess.run(["ss", "-tnpH", "state", "established", f"( dport = :{SSH_PORT} )"],
                             capture_output=True, text=True, check=False)
    return parse_sockets(listing.stdout)


def hop_holder(observed, node_address, client):
    """The connection to the node from `client`, an `(address, port)` the node reported, seen while
    `observed` ran."""
    for sock in observed.sockets:
        if sock.peer == (node_address, SSH_PORT) and sock.local == client:
            return sock
    raise TestFailure(f"no connection from {client[0]}:{client[1]} to {node_address}:{SSH_PORT} "
                      f"was seen while the command ran")


def check_direct(observed, node_address):
    """The command itself held the connection the node reported."""
    sock = hop_holder(observed, node_address, node_connection(observed.outcome.output))
    require(observed.pid in sock.pids,
            f"the connection to the node was held by {sorted(sock.pids) or 'another user'}, "
            f"not by sherpa (pid {observed.pid})")


def check_jump(observed, node_address, server_address):
    """The command connected only to the server, whose SSH server held the connection to the node."""
    sock = hop_holder(observed, node_address, node_connection(observed.outcome.output))
    require(observed.pid not in sock.pids,
            f"sherpa (pid {observed.pid}) held the connection to the node itself")
    to_server = [s for s in observed.sockets if observed.pid in s.pids and s.peer == (server_address, SSH_PORT)]
    require(to_server, f"sherpa (pid {observed.pid}) held no connection to the server's SSH port "
                       f"{server_address}:{SSH_PORT}")
    to_node = [s for s in observed.sockets if observed.pid in s.pids and s.peer[0] == node_address]
    require(not to_node, f"sherpa (pid {observed.pid}) also connected to the node: {to_node}")


# -- the lab -------------------------------------------------------------------------------------


class Lab:
    """The lab every case uses, brought up once by `up` and always destroyed by `destroy`."""

    def __init__(self, run_root):
        self.name = f"sshaccess-{run_root.name[-6:]}"
        self.directory = run_root / "lab"
        self.started = False
        self.lab_id = None
        self.node_address = None
        self.server_address = None
        self.log = None

    def command(self, label, arguments, timeout=COMMAND_TIMEOUT, check=True):
        outcome = run_logged(self.log, label, arguments, self.directory, timeout)
        if check and outcome.code != 0:
            raise TestFailure(f"{label} failed (exit {outcome.code}); see {self.log.name}")
        return outcome

    def sherpa(self, label, arguments, timeout=COMMAND_TIMEOUT, check=True):
        return self.command(label, ["sherpa", *arguments], timeout, check)

    def up(self):
        self.directory.mkdir(parents=True)
        self.log = (self.directory / "commands.log").open("w")
        (self.directory / "manifest.toml").write_text(render_manifest(self.name))
        script = self.directory / PROVISION_SCRIPT
        script.write_text(PROVISION_SCRIPT_CONTENT)
        script.chmod(0o755)
        self.started = True
        outcome = self.sherpa("sherpa up", ["up"], UP_TIMEOUT, check=False)
        info_path = self.directory / ".sherpa" / "lab-info.toml"
        if info_path.exists():
            info = tomllib.loads(info_path.read_text())
            require(info["name"] == self.name, f".sherpa/lab-info.toml names {info['name']}, not {self.name}")
            self.lab_id = info["id"]
        require(outcome.code == 0, f"sherpa up failed (exit {outcome.code}); see {self.log.name}")
        require(self.lab_id is not None, "sherpa up wrote no .sherpa/lab-info.toml")
        self.node_address = self.host_block()["hostname"]
        inspect = self.sherpa("sherpa inspect", ["inspect"])
        self.server_address = socket.gethostbyname(server_host(inspect.output))

    def host_block(self):
        """The node's `Host` block in the lab's SSH config."""
        alias = f"{NODE}.{self.lab_id}"
        config = (self.directory / ".sherpa" / "ssh" / "config").read_text()
        for block in parse_ssh_hosts(config):
            if alias in block["host"]:
                return block
        raise TestFailure(f"the lab SSH config has no Host block for {alias}")

    def observe(self, label, arguments, spawn):
        """Run `sherpa <arguments>` with `spawn`, which starts it and returns its pid and a function
        that reports its Outcome once it ended, recording every connection to an SSH port it was
        seen to hold while it ran."""
        full = ["sherpa", *arguments]
        print(f"[{self.directory.name}] {label}", flush=True)
        pid, finished = spawn(full, self.directory)
        sockets = set()
        deadline = time.monotonic() + SSH_TIMEOUT
        outcome = None
        while outcome is None:
            sockets.update(ssh_sockets())
            outcome = finished()
            if outcome is None and time.monotonic() > deadline:
                os.kill(pid, signal.SIGKILL)
                outcome = finished(wait=True)
                outcome = Outcome(None, outcome.stdout, f"{outcome.stderr}\ntimed out after {SSH_TIMEOUT}s")
            time.sleep(POLL_SECONDS)
        log_outcome(self.log, label, full, outcome)
        return Observed(pid, outcome, frozenset(sockets))

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
        finally:
            self.log.close()
        return None


def spawn_in_terminal(arguments, cwd):
    """Start `arguments` in a pseudo-terminal of its own, as a user's terminal runs `sherpa ssh`,
    and type `SHELL_COMMAND` once it says it is connected."""
    pid, fd = pty.fork()
    if pid == 0:
        os.chdir(cwd)
        os.execvp(arguments[0], arguments)
    output = bytearray()
    typed = False
    status = None

    def finished(wait=False):
        nonlocal typed, status
        while select.select([fd], [], [], 0)[0]:
            try:
                data = os.read(fd, 4096)
            except OSError:
                break
            if not data:
                break
            output.extend(data)
        if not typed and CONNECTED.encode() in output:
            os.write(fd, SHELL_COMMAND.encode())
            typed = True
        if status is None:
            done, raw = os.waitpid(pid, 0 if wait else os.WNOHANG)
            if done == 0:
                return None
            status = os.waitstatus_to_exitcode(raw)
            os.close(fd)
        return Outcome(status, output.decode(errors="replace"), "")

    return pid, finished


def spawn_piped(arguments, cwd):
    """Start `arguments` with its output piped, as a script runs `sherpa provision`."""
    process = subprocess.Popen(arguments, cwd=cwd, stdin=subprocess.DEVNULL, stdout=subprocess.PIPE,
                               stderr=subprocess.PIPE, text=True)

    def finished(wait=False):
        if not wait and process.poll() is None:
            return None
        stdout, stderr = process.communicate()
        return Outcome(process.returncode, stdout, stderr)

    return process.pid, finished


# -- the fixture ---------------------------------------------------------------------------------


DEVICE_PASSWORD_SET = re.compile(r"^\s*Device password:\s*set\s*$", re.MULTILINE)


def preflight(run_root):
    for tool in ("sherpa", "ss"):
        require(shutil.which(tool), f"{tool} is not on PATH")
    with (run_root / "preflight.log").open("w") as log:
        whoami = run_logged(log, "sherpa whoami", ["sherpa", "whoami"], run_root)
        require(whoami.code == 0, "the sherpa CLI is not logged in (run `sherpa login`)")
        info = run_logged(log, "sherpa server user info", ["sherpa", "server", "user", "info"], run_root)
        require(info.code == 0 and DEVICE_PASSWORD_SET.search(info.stdout),
                "the user has no device password, which `sherpa up` needs: "
                "run `sherpa server user device-password`")
        for node in NODES:
            image = run_logged(log, f"{node['model']} image present", ["sherpa", "image", "show", node["model"]],
                               run_root)
            require(image.code == 0, f"the server has no {node['model']} image")


@pytest.fixture(scope="module")
def lab(request):
    """The lab every case uses: brought up once after the preflight checks and destroyed at the
    end, even when a case fails, unless `--keep-failed` is given and something failed."""
    signal.signal(signal.SIGTERM, stop_on_sigterm)
    keep_failed = request.config.getoption("keep_failed")
    run_root = RUNS / f"ssh-access-{uuid.uuid4().hex[:12]}"
    run_root.mkdir(parents=True)
    print(f"Logs: {run_root}", flush=True)
    preflight(run_root)
    lab = Lab(run_root)
    keep = False
    try:
        try:
            lab.up()
        except Exception:
            keep = keep_failed
            raise
        yield lab
        keep = keep_failed and request.session.testsfailed > 0
    finally:
        if keep:
            print(f"keeping failed lab {lab.name} ({lab.lab_id}) in {lab.directory}", flush=True)
        else:
            cleanup = lab.destroy()
            if cleanup:
                raise cleanup


# -- cases ---------------------------------------------------------------------------------------


def test_ssh_access_001_the_lab_ssh_config_reaches_every_node_directly(lab):
    config = (lab.directory / ".sherpa" / "ssh" / "config").read_text()
    blocks = parse_ssh_hosts(config)
    require(blocks, "the lab SSH config has no Host blocks")
    for block in blocks:
        for setting in ("proxyjump", "proxycommand"):
            require(setting not in block, f"Host {' '.join(block['host'])} has {setting}: {block.get(setting)}")
    require(lab.host_block()["hostname"] in lab.host_block()["host"],
            f"{NODE}'s Host block does not name its own address: {lab.host_block()}")


def test_ssh_access_002_sherpa_ssh_reaches_a_node_directly(lab):
    observed = lab.observe(f"sherpa ssh {NODE}", ["ssh", NODE], spawn_in_terminal)
    require(observed.outcome.code == SHELL_EXIT,
            f"sherpa ssh exited {observed.outcome.code}, not the shell's {SHELL_EXIT}")
    check_direct(observed, lab.node_address)


def test_ssh_access_003_sherpa_ssh_with_ssh_jump_goes_through_the_server(lab):
    observed = lab.observe(f"sherpa ssh {NODE} --ssh-jump", ["ssh", NODE, "--ssh-jump"], spawn_in_terminal)
    require(observed.outcome.code == SHELL_EXIT,
            f"sherpa ssh --ssh-jump exited {observed.outcome.code}, not the shell's {SHELL_EXIT}")
    check_jump(observed, lab.node_address, lab.server_address)


def test_ssh_access_004_a_provisioner_reaches_its_node_directly(lab):
    observed = lab.observe("sherpa provision", ["provision"], spawn_piped)
    require(observed.outcome.code == 0, f"sherpa provision failed (exit {observed.outcome.code})")
    check_direct(observed, lab.node_address)


def test_ssh_access_005_a_provisioner_with_ssh_jump_goes_through_the_server(lab):
    observed = lab.observe("sherpa provision --ssh-jump", ["provision", "--ssh-jump"], spawn_piped)
    require(observed.outcome.code == 0, f"sherpa provision --ssh-jump failed (exit {observed.outcome.code})")
    check_jump(observed, lab.node_address, lab.server_address)


# -- running -------------------------------------------------------------------------------------


def main():
    parser = argparse.ArgumentParser(description=__doc__,
                                     formatter_class=argparse.RawDescriptionHelpFormatter,
                                     epilog="Other arguments, such as --keep-failed or -k, are passed to pytest.")
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

    SS = ("0 0 172.30.1.1:40760 172.30.1.21:22\n"
          '0 0 172.30.1.1:49566 172.30.1.21:22 users:(("sherpa",pid=228837,fd=9))\n'
          '0 0 172.31.1.21:33554 172.31.1.21:22 users:(("sherpa",pid=300,fd=11))\n')

    def observed(self, pid, sockets_text, client_port):
        output = f"{PROVISION_MARKER} 172.30.1.1 {client_port} 172.30.1.21 22\n"
        return Observed(pid, Outcome(0, output, ""), frozenset(parse_sockets(sockets_text)))

    def test_a_connection_held_by_sherpa_is_direct_and_not_a_jump(self):
        observed = self.observed(228837, self.SS, 49566)
        check_direct(observed, "172.30.1.21")
        with self.assertRaises(TestFailure):
            check_jump(observed, "172.30.1.21", "172.31.1.21")

    def test_a_connection_held_by_another_user_with_sherpa_at_the_server_is_a_jump(self):
        observed = self.observed(300, self.SS, 40760)
        check_jump(observed, "172.30.1.21", "172.31.1.21")
        with self.assertRaises(TestFailure):
            check_direct(observed, "172.30.1.21")

    def test_a_jump_without_a_connection_to_the_server_fails(self):
        observed = self.observed(999, self.SS, 40760)
        with self.assertRaises(TestFailure):
            check_jump(observed, "172.30.1.21", "172.31.1.21")

    def test_a_connection_never_seen_fails(self):
        with self.assertRaises(TestFailure):
            check_direct(self.observed(228837, self.SS, 1), "172.30.1.21")

    def test_a_config_with_a_jump_fails_the_config_case(self):
        config = "Host 172.30.1.21 alp01.x\n    HostName 172.30.1.21\n    ProxyJump a@b\n"
        with tempfile.TemporaryDirectory() as temp:
            lab = Lab(Path(temp) / "ssh-access-000000abcdef")
            lab.lab_id = "x"
            (lab.directory / ".sherpa" / "ssh").mkdir(parents=True)
            (lab.directory / ".sherpa" / "ssh" / "config").write_text(config)
            with self.assertRaisesRegex(TestFailure, "proxyjump: a@b"):
                test_ssh_access_001_the_lab_ssh_config_reaches_every_node_directly(lab)
            (lab.directory / ".sherpa" / "ssh" / "config").write_text(config.replace("    ProxyJump a@b\n", ""))
            test_ssh_access_001_the_lab_ssh_config_reaches_every_node_directly(lab)

    def test_a_proxy_jump_in_the_config_is_seen(self):
        blocks = parse_ssh_hosts("Host 172.30.1.21 alp01.x\n    HostName 172.30.1.21\n    ProxyJump a@b\n")
        self.assertEqual(blocks[0]["proxyjump"], "a@b")
        self.assertEqual(blocks[0]["host"], ("172.30.1.21", "alp01.x"))

    def test_the_server_host_is_read_from_inspect(self):
        self.assertEqual(server_host("Connecting to server: wss://172.31.1.21:3030/ws\n"), "172.31.1.21")

    def test_the_manifest_gives_the_node_its_provisioner(self):
        manifest = tomllib.loads(render_manifest("sshaccess-abcdef"))
        self.assertEqual(manifest["nodes"][0]["provisioners"][0]["scripts"], [f"./{PROVISION_SCRIPT}"])


if __name__ == "__main__":
    sys.exit(main())
