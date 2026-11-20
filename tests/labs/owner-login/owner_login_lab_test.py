#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.15"
# dependencies = []
# ///
"""End-to-end test of the lab owner's login, and of what an admin may not do, through the sherpa CLI
(Python 3.15+, stdlib only).

Brings up a lab started by the owner (the CLI's current login) with an Ubuntu VM and an SR Linux
container. Checks the owner reaches each node with `sherpa ssh` by key and with ssh by the device
password, has administrator rights on each, is refused as the old shared `sherpa` user, and has no
key file in the lab folder. Then checks a second account, an admin who does not own the lab, which
works with its own HOME: it can inspect the lab, is refused `sherpa download`,
`sherpa ssh` and `sherpa console`, cannot reach the nodes with ssh and its own key, and cannot add a
key to the owner's account; the admin then destroys the lab. Always destroys the lab. See README.md.
"""

import argparse
import fcntl
from http.server import BaseHTTPRequestHandler, HTTPServer
import io
import json
import os
from pathlib import Path
import re
import select
import shlex
import shutil
import signal
import ssl
import struct
import subprocess
import sys
import tempfile
import termios
import threading
import time
import tomllib
from typing import NamedTuple
import unittest
from unittest import mock
import urllib.error
import urllib.parse
import urllib.request
import uuid


LAB = Path(__file__).resolve().parent
ROOT = LAB.parent.parent.parent
RUNS = ROOT / ".tmp" / "lab-tests"

# The admin's name, the admin's Sherpa password and the owner's device password.
ENV_ADMIN_USER = "SHERPA_LAB_TEST_ADMIN_USER"
ENV_ADMIN_PASSWORD = "SHERPA_LAB_TEST_ADMIN_PASSWORD"
ENV_DEVICE_PASSWORD = "SHERPA_LAB_TEST_DEVICE_PASSWORD"
# What `sherpa login` reads, and what a client run reads for its server.
LOGIN_USER_ENV = "SHERPA_USER_NAME"
LOGIN_PASSWORD_ENV = "SHERPA_USER_PASSWORD"
SERVER_URL_ENV = "SHERPA_SERVER_URL"

VM = "ubu01"
CONTAINER = "srl01"
# The shared account earlier releases gave every node. It is not made now.
OLD_SHARED_USER = "sherpa"
BUILT_IN_ADMIN = "admin"

# What Sherpa says when it refuses, matched as text contained in the output. `{lab}` is the lab's ID
# and `{owner}` the owner's name.
OWNER_ONLY_SERVER = "Only the owner of lab '{lab}' can perform this operation"
OWNER_ONLY_CONNECT = "Only the owner of lab '{lab}' can connect to its nodes"
# The server's refusal of a console, serial or VNC, to anyone but the lab's owner.
OWNER_ONLY_CONSOLE = "only the owner of lab '{lab}' can open its consoles"
OWN_ACCOUNT_ONLY = "only user '{owner}' can do this for their own account"

COMMAND_TIMEOUT = 180
UP_TIMEOUT = 1800
READY_TIMEOUT = 1200
POLL_INTERVAL = 10
# How long an interactive session may take to show a command's output, and how often the command is typed again.
SESSION_TIMEOUT = 120
RESEND_SECONDS = 10
CONNECT_TIMEOUT = 60
PASSWORD_TIMEOUT = 120
LEAVE_TIMEOUT = 20
REST_TIMEOUT = 30
REFUSAL_TIMEOUT = 90

# OpenSSH options for lab nodes: no prompts, and no host key notices mixed into command output.
SSH_OPTIONS = ("-o", "BatchMode=yes", "-o", "LogLevel=ERROR", "-o", "ConnectTimeout=15")
PERMISSION_DENIED = "Permission denied"

# A command that succeeds once the owner's login works, by node.
READY_COMMANDS = {VM: "true", CONTAINER: "show version"}
# Prints who is logged in (the typed command shows `%s`, never the name) and succeeds if sudo needs
# no password: the owner's administrator rights on a Linux node.
LINUX_COMMAND = "printf 'who:%s:\\n' \"$(id -un)\"; sudo -n true && echo sudo-ok-$((40+2))"
# SR Linux's state of the owner's account (`{owner}` filled in): its roles, and its last successful
# login, which the session running the command has just made. The owner's role is `srl-admin`,
# whose `superuser` flag the generated config sets. (SR Linux 26.7's session list needs a session
# id, so it cannot be listed whole.)
SRLINUX_COMMAND = "info from state /system aaa authentication user {owner}"
PROBE_COMMANDS = {VM: LINUX_COMMAND, CONTAINER: SRLINUX_COMMAND}


def probe_command(node, owner):
    """The command that shows `owner`'s login and administrator rights on `node`."""
    return PROBE_COMMANDS[node].format(owner=owner)
# What leaves each node's shell: the shell's own `exit`, and SR Linux's `quit`.
LEAVE_COMMANDS = {VM: b"exit\r", CONTAINER: b"quit\r"}
ESCAPE = b"\r~."

PASSWORD_PROMPT = re.compile(
    r"(?P<user>[^\s@()']+)@(?P<host>[^\s')]+)'s password: |"
    r"\((?P<kuser>[^\s@()]+)@(?P<khost>[^\s)]+)\) Password: ")
DEVICE_PASSWORD_SET = re.compile(r"^\s*Device password:\s*set\s*$", re.MULTILINE)
INFO_USERNAME = re.compile(r"^\s*Username:\s*(\S+)\s*$", re.MULTILINE)
INFO_ADMIN = re.compile(r"^\s*Admin:\s*(Yes|No)\s*$", re.MULTILINE)
# A terminal control sequence, such as the colours and cursor moves of SR Linux's CLI.
ANSI_ESCAPE = re.compile(r"\x1b\[[0-9;?]*[A-Za-z]")
PRIVATE_KEY = re.compile(rb"-----BEGIN [A-Z0-9 ]*PRIVATE KEY-----|openssh-key-v1")
CELL_SEPARATOR = "│"
SECRETS = []


class TestFailure(Exception):
    """A check failed; the message says which and why."""


class Outcome(NamedTuple):
    """A finished command. Parse `stdout`; search `output` for messages, which may be on either stream."""

    code: int | None
    stdout: str
    stderr: str

    @property
    def output(self):
        return self.stdout + self.stderr


class Accounts(NamedTuple):
    """The two Sherpa accounts, as found by the preflight."""

    owner: str
    admin: str
    admin_home: Path
    admin_env: dict
    device_password: str


class SshHost(NamedTuple):
    """One `Host` block of the lab's SSH config."""

    aliases: tuple
    hostname: str
    port: int
    user: str


class PasswordPrompt(NamedTuple):
    user: str
    host: str
    end: int


class RestReply(NamedTuple):
    status: int
    body: str


def require(condition, message):
    if not condition:
        raise TestFailure(message)


def add_secret(secret):
    if secret and secret not in SECRETS:
        SECRETS.append(secret)


def mask(text):
    """`text` with every secret of the run replaced, for the logs."""
    for secret in SECRETS:
        text = text.replace(secret, "********")
    return text


def log_name(log):
    return getattr(log, "name", "the log")


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
        log.write(mask(f"=== {label}: {shlex.join(arguments)}\n--- interrupted\n"))
        log.flush()
        raise
    log.write(mask(f"=== {label}: {shlex.join(arguments)}\n--- exit {outcome.code}\n"
                   f"--- stdout\n{outcome.stdout}\n--- stderr\n{outcome.stderr}\n"))
    log.flush()
    return outcome


def stop_on_sigterm(_signal, _frame):
    raise KeyboardInterrupt("SIGTERM")


def value_toml(value):
    if isinstance(value, bool):
        return "true" if value else "false"
    if isinstance(value, int):
        return str(value)
    if isinstance(value, str):
        return json.dumps(value)
    if isinstance(value, dict):
        return "{ " + ", ".join(f"{key} = {value_toml(item)}" for key, item in value.items()) + " }"
    raise TypeError(f"Unsupported TOML value: {value!r}")


def render_manifest(manifest, name):
    """The lab manifest for one run, under a unique lab name."""
    lines = [f"name = {value_toml(name)}", "", "nodes = ["]
    lines += [f"  {value_toml(node)}," for node in manifest["nodes"]]
    return "\n".join([*lines, "]"]) + "\n"


# -- parsing -------------------------------------------------------------------------------------


class UserInfo(NamedTuple):
    username: str
    admin: bool
    device_password_set: bool


def parse_user_info(text):
    """The user in `sherpa server user info`'s text output."""
    username = INFO_USERNAME.search(text)
    admin = INFO_ADMIN.search(text)
    require(username and admin, "`sherpa server user info` did not name the user and say if they are an admin")
    return UserInfo(username.group(1), admin.group(1) == "Yes", bool(DEVICE_PASSWORD_SET.search(text)))


def parse_ssh_hosts(config):
    """The `Host` blocks of a lab SSH config."""
    blocks = []
    for line in config.splitlines():
        key, _, value = line.strip().partition(" ")
        value = value.strip()
        if key == "Host":
            blocks.append({"aliases": tuple(value.split())})
        elif blocks and key in ("HostName", "Port", "User"):
            blocks[-1][key] = value
    hosts = []
    for block in blocks:
        require("HostName" in block and "User" in block,
                f"the lab SSH config has a Host block without a HostName or User: {block}")
        hosts.append(SshHost(block["aliases"], block["HostName"], int(block.get("Port", "22")), block["User"]))
    return hosts


def find_host(hosts, alias):
    for host in hosts:
        if alias in host.aliases:
            return host
    raise TestFailure(f"the lab SSH config has no host {alias}")


def parse_devices_table(text):
    """The rows of `sherpa inspect`'s Active Nodes table, each a dict by column header. A cell that
    wraps onto more lines, as a node's disks do, adds no row."""
    headers = None
    rows = []
    for line in text.splitlines():
        line = line.strip()
        if line.startswith("└") and headers is not None:
            break
        if not (line.startswith(CELL_SEPARATOR) and line.endswith(CELL_SEPARATOR)):
            continue
        cells = [cell.strip() for cell in line.split(CELL_SEPARATOR)[1:-1]]
        if headers is None:
            if cells and cells[0] == "Device":
                headers = cells
            continue
        if cells[0]:
            rows.append(dict(zip(headers, cells)))
    return rows


def password_prompts(text):
    """Every password prompt OpenSSH printed: for `user@host's password: `, and for a
    keyboard-interactive `(user@host) Password: `."""
    prompts = []
    for found in PASSWORD_PROMPT.finditer(text):
        prompts.append(PasswordPrompt(found.group("user") or found.group("kuser"),
                                      found.group("host") or found.group("khost"), found.end()))
    return prompts


def probe_problems(node, owner, text):
    """What the output of the node's probe command lacks, or an empty list when it shows the owner's
    administrator rights."""
    if node == VM:
        wanted = {f"the login `{owner}`": rf"who:{re.escape(owner)}:", "passwordless sudo": r"sudo-ok-42"}
    else:
        wanted = {f"a login of `{owner}`": r'last-successful-login\s+"[^"]*\(\d+ (?:seconds?|minutes?) ago\)"',
                  "the `srl-admin` role": r"role\s*\[\s*srl-admin\s*\]"}
    text = ANSI_ESCAPE.sub("", text)
    return [what for what, pattern in wanted.items() if not re.search(pattern, text)]


def private_key_files(folder):
    """The files under `folder` that hold a private key."""
    found = []
    for path in sorted(folder.rglob("*")):
        if path.is_file() and not path.is_symlink():
            with path.open("rb") as handle:
                if PRIVATE_KEY.search(handle.read(1 << 20)):
                    found.append(path)
    return found


def password_ssh_config(lab_config, aliases):
    """An SSH config that gives the nodes `aliases` password logins only, then the lab's own config."""
    blocks = [f"Host {alias}\n    PubkeyAuthentication no\n    PreferredAuthentications "
              "password,keyboard-interactive\n    NumberOfPasswordPrompts 3\n" for alias in aliases]
    return "\n".join(blocks) + "\n" + lab_config


def admin_ssh_config(key):
    """An SSH config for the admin's own key and nothing else: no agent, no other identity, no prompt."""
    return (f"Host *\n    IdentityFile {key}\n    IdentitiesOnly yes\n    IdentityAgent none\n"
            "    BatchMode yes\n    PreferredAuthentications publickey\n    PasswordAuthentication no\n"
            "    KbdInteractiveAuthentication no\n    StrictHostKeyChecking no\n"
            "    UserKnownHostsFile /dev/null\n    LogLevel ERROR\n    ConnectTimeout 15\n")


def direct_ssh_arguments(config, key, host, login, command):
    """`ssh` straight to a node's management address, as `login`, with the admin's key."""
    return ["ssh", "-F", str(config), "-o", "BatchMode=yes", "-o", "IdentitiesOnly=yes", "-i", str(key),
            "-p", str(host.port), "-l", login, host.hostname, command]


# -- REST ----------------------------------------------------------------------------------------


def server_base_url(config, from_env):
    """The server's REST address, `https://host:port`: the URL named in the environment or the client
    config, else its address and port, on the WebSocket's port."""
    connection = config.get("server_connection", {})
    url = from_env or connection.get("url")
    if not url:
        scheme = "wss" if config.get("tls", {}).get("enabled", True) else "ws"
        url = f"{scheme}://{config.get('server_ipv4', '127.0.0.1')}:{config.get('ws_port', 3030)}/ws"
    parts = urllib.parse.urlsplit(url)
    scheme = {"wss": "https", "ws": "http", "https": "https", "http": "http"}.get(parts.scheme)
    require(scheme, f"cannot make a REST address from the server URL {url}")
    return f"{scheme}://{parts.netloc}"


def trusted_certificate(trust_dir, base_url):
    """The certificate the client trusted for the server, as the client's trust store names it."""
    parts = urllib.parse.urlsplit(base_url)
    host = (parts.hostname or "unknown").replace(":", "_").replace("/", "_")
    return trust_dir / f"{host}_{parts.port}.pem"


def tls_context(config, base_url, trust_dir):
    """How to check the server's certificate as the client config does: not at all when it allows
    insecure connections, against its CA file when it names one, against the certificate the client
    trusted on first use when there is one, and otherwise against the system's authorities."""
    if base_url.startswith("http://"):
        return None
    connection = config.get("server_connection", {})
    if connection.get("insecure") or connection.get("validate_certs") is False:
        context = ssl.create_default_context()
        context.check_hostname = False
        context.verify_mode = ssl.CERT_NONE
        return context
    if connection.get("ca_cert_path"):
        context = ssl.create_default_context(cafile=connection["ca_cert_path"])
    elif trusted_certificate(trust_dir, base_url).exists():
        context = ssl.create_default_context(cafile=str(trusted_certificate(trust_dir, base_url)))
        # The pinned certificate is the one trusted, whatever name it was issued for.
        context.check_hostname = False
    else:
        context = ssl.create_default_context()
    context.verify_flags &= ~ssl.VERIFY_X509_STRICT
    return context


def rest_post(url, token, body, context):
    """POST `body` as JSON with the caller's token; a refusal is a reply, not an error."""
    request = urllib.request.Request(
        url, data=json.dumps(body).encode(), method="POST",
        headers={"Authorization": f"Bearer {token}", "Content-Type": "application/json",
                 "Accept": "application/json"})
    try:
        with urllib.request.urlopen(request, timeout=REST_TIMEOUT, context=context) as reply:
            return RestReply(reply.status, reply.read().decode(errors="replace"))
    except urllib.error.HTTPError as error:
        with error:
            return RestReply(error.code, error.read().decode(errors="replace"))
    except (urllib.error.URLError, OSError) as error:
        raise TestFailure(f"POST {url} failed: {error}") from error


# -- terminal ------------------------------------------------------------------------------------


# The "device status report" a program sends to ask for the cursor position, and a terminal's
# answer: row 1, column 1.
CURSOR_POSITION_QUERY = b"\x1b[6n"
CURSOR_POSITION_REPORT = b"\x1b[1;1R"


class Terminal:
    """A command in a pseudo-terminal of its own, as a user's terminal runs it: it is the command's
    controlling terminal, so `ssh` and `sherpa ssh` ask for passwords on it."""

    def __init__(self, log, label, arguments, cwd, env):
        self.log = log
        self.label = label
        self.transcript = bytearray()
        master, slave = os.openpty()
        # A terminal of a real size: a full-screen CLI lays its prompt out by it.
        fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 24, 120, 0, 0))
        self.master = master
        self.process = subprocess.Popen(arguments, cwd=cwd, env=env, stdin=slave, stdout=slave,
                                        stderr=slave, preexec_fn=lambda: os.login_tty(slave))
        os.close(slave)
        print(f"[{Path(cwd).name}] {label}", flush=True)

    def read(self, timeout):
        """Read what the command printed within `timeout` seconds; False once it has closed."""
        ready, _, _ = select.select([self.master], [], [], timeout)
        if not ready:
            return True
        try:
            data = os.read(self.master, 4096)
        except OSError:
            return False
        self.transcript += data
        # A full-screen CLI such as SR Linux's asks the terminal where its cursor is and waits for
        # the answer; a real terminal replies, so this one does too.
        for _ in range(data.count(CURSOR_POSITION_QUERY)):
            os.write(self.master, CURSOR_POSITION_REPORT)
        return bool(data)

    def text(self):
        return self.transcript.decode(errors="replace").replace("\r\n", "\n")

    def send(self, data):
        os.write(self.master, data)

    def running(self):
        return self.process.poll() is None

    def wait_for(self, condition, timeout, what):
        """Wait until `condition(text)` holds; return the text."""
        deadline = time.monotonic() + timeout
        while True:
            text = self.text()
            if condition(text):
                return text
            remaining = deadline - time.monotonic()
            require(remaining > 0, f"{self.label}: {what} not seen within {timeout}s; see {log_name(self.log)}")
            open_ = self.read(min(remaining, 1))
            require(open_ or self.running(), f"{self.label} ended before {what}; see {log_name(self.log)}")

    def wait_for_end(self, timeout):
        """Read until the command ends; whether it did within `timeout` seconds."""
        deadline = time.monotonic() + timeout
        while self.running() and time.monotonic() < deadline:
            self.read(0.5)
        return not self.running()

    def finish(self, timeout=CONNECT_TIMEOUT):
        """Wait for the command to end and return its exit code, keeping what it printed."""
        ended = self.wait_for_end(timeout)
        while self.read(0.1):
            pass
        code = self.process.poll()
        if code is None:
            self.process.kill()
            self.process.wait()
        os.close(self.master)
        self.log.write(mask(f"=== {self.label}\n--- exit {code}\n--- terminal\n{self.text()}\n"))
        self.log.flush()
        require(ended, f"{self.label} did not end within {timeout}s; see {log_name(self.log)}")
        return code


def answer_passwords(terminal, address, password, timeout):
    """Type `password` at each password prompt for the host `address` until the command ends, and
    return the number of prompts answered. A prompt for another host, such as the Sherpa server, or a
    question about a host key fails the check: neither is what is under test."""
    deadline = time.monotonic() + timeout
    answered_to = 0
    answered = 0
    while True:
        text = terminal.text()
        for prompt in password_prompts(text):
            if prompt.end <= answered_to:
                continue
            require(prompt.host == address,
                    f"{terminal.label}: asked for the password of {prompt.user}@{prompt.host}, not of "
                    f"the node {address}; see {log_name(terminal.log)}")
            terminal.send(password.encode() + b"\r")
            answered_to = prompt.end
            answered += 1
        if "(yes/no" in text:
            terminal.process.kill()
            terminal.finish()
            raise TestFailure(f"{terminal.label}: ssh asked about a host key; see {log_name(terminal.log)}")
        if not terminal.running():
            return answered
        require(time.monotonic() < deadline, f"{terminal.label}: still running after {timeout}s")
        terminal.read(0.5)


# -- lab -----------------------------------------------------------------------------------------


class LabRun:
    def __init__(self, run_root, keep_failed, accounts):
        self.accounts = accounts
        self.owner = accounts.owner
        self.manifest = tomllib.loads((LAB / "manifest.toml").read_text())
        self.name = f"{self.manifest['name']}-{run_root.name[-6:]}"
        self.run_root = run_root
        self.directory = run_root / "lab"
        # The admin's folder holds a copy of the owner's lab-info.toml and nothing else; it is how
        # an admin names a lab they do not own. `sherpa download` needs a folder with no lab files.
        self.admin_directory = run_root / "admin"
        self.download_directory = run_root / "admin-download"
        self.admin_key = run_root / "admin-key" / "id_ed25519"
        self.keep_failed = keep_failed
        self.started = False
        self.admin_destroyed = False
        self.lab_id = None
        self.hosts = []
        self.log = None
        self.sherpa_path = shutil.which("sherpa") or "sherpa"

    # -- commands ----------------------------------------------------------------------------

    def command(self, label, arguments, timeout=COMMAND_TIMEOUT, check=True, cwd=None, env=None):
        outcome = run_logged(self.log, label, arguments, cwd or self.directory, timeout, env)
        if check and outcome.code != 0:
            raise TestFailure(f"{label} failed (exit {outcome.code}); see {self.log.name}")
        return outcome

    def sherpa(self, label, arguments, timeout=COMMAND_TIMEOUT, check=True):
        """A `sherpa` command as the owner, in the lab folder."""
        return self.command(label, [self.sherpa_path, *arguments], timeout, check)

    def admin(self, label, arguments, timeout=COMMAND_TIMEOUT, check=True, cwd=None):
        """A `sherpa` command as the admin, with the admin's own HOME, in the admin's folder."""
        return self.command(f"{label} (admin)", [self.sherpa_path, *arguments], timeout, check,
                            cwd or self.admin_directory, self.accounts.admin_env)

    def alias(self, node):
        return f"{node}.{self.lab_id}"

    def host(self, node):
        return find_host(self.hosts, self.alias(node))

    def ssh(self, label, node, remote_command, config=None, login=None, check=False):
        """`ssh` to a node as the lab's SSH config does (the owner, by key), or as `login`."""
        user = ["-l", login] if login else []
        return self.command(label, ["ssh", "-F", str(config or ".sherpa/ssh/config"), *SSH_OPTIONS, *user,
                                    self.alias(node), remote_command], check=check)

    # -- run ---------------------------------------------------------------------------------

    def run(self):
        self.directory.mkdir(parents=True)
        with (self.directory / "commands.log").open("w") as self.log:
            (self.directory / "manifest.toml").write_text(render_manifest(self.manifest, self.name))
            failure = None
            try:
                self.up()
                self.owner_checks()
                self.admin_checks()
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
        # A partly created lab exits non-zero after writing lab-info.toml: record its ID first, so
        # destroy can check what it leaves behind.
        info_path = self.directory / ".sherpa" / "lab-info.toml"
        if info_path.exists():
            info = tomllib.loads(info_path.read_text())
            require(info["name"] == self.name, f".sherpa/lab-info.toml names {info['name']}, not {self.name}")
            self.lab_id = info["id"]
        require(outcome.code == 0, f"sherpa up failed (exit {outcome.code}); see {self.log.name}")
        require(self.lab_id is not None, "sherpa up wrote no .sherpa/lab-info.toml")
        config = (self.directory / ".sherpa" / "ssh" / "config").read_text()
        require("IdentityFile" not in config, "the lab's SSH config names an identity file")
        self.hosts = parse_ssh_hosts(config)
        for node in (VM, CONTAINER):
            require(self.host(node).user == self.owner,
                    f"the lab's SSH config does not log in to {node} as {self.owner}")

    # -- owner -------------------------------------------------------------------------------

    def owner_checks(self):
        for node in (VM, CONTAINER):
            self.wait_for_login(node)
        self.check_no_key_in_lab_folder()
        self.check_owner_inspect()
        for node in (VM, CONTAINER):
            self.check_key_login(node)
        password_config = self.run_root / "password_ssh_config"
        password_config.write_text(password_ssh_config((self.directory / ".sherpa" / "ssh" / "config").read_text(),
                                             [self.alias(VM), self.alias(CONTAINER)]))
        for node in (VM, CONTAINER):
            self.check_password_login(node, password_config)
        for node in (VM, CONTAINER):
            self.check_old_shared_login_refused(node, password_config)

    def wait_for_login(self, node, timeout=READY_TIMEOUT):
        """The node takes the owner's SSH login by key, retrying while it boots."""
        deadline = time.monotonic() + timeout
        while self.ssh(f"{node} reachable", node, READY_COMMANDS[node]).code != 0:
            require(time.monotonic() < deadline, f"no SSH login as {self.owner} on {node} within {timeout}s")
            time.sleep(POLL_INTERVAL)

    def check_no_key_in_lab_folder(self):
        """`sherpa up` leaves no SSH key in the lab folder: nodes take the owner's own keys."""
        key = self.directory / ".sherpa" / "ssh" / "key"
        require(not key.exists(), f"the lab folder has a key file, {key}")
        found = private_key_files(self.directory / ".sherpa")
        require(not found, f"the lab folder has private key files: {[str(path) for path in found]}")

    def check_owner_inspect(self):
        """The owner's inspect lists both nodes, with no VNC port: VNC goes through `sherpa vnc`."""
        rows = {row["Device"]: row for row in parse_devices_table(self.sherpa("sherpa inspect", ["inspect"]).stdout)}
        require(self.alias(VM) in rows and self.alias(CONTAINER) in rows,
                f"sherpa inspect does not list both nodes: {sorted(rows)}")
        require("VNC Port" not in rows[self.alias(VM)], "the owner's inspect has a VNC Port column")

    def check_key_login(self, node):
        """`sherpa ssh` logs in by key, runs the node's probe command and leaves."""
        label = f"sherpa ssh {node} (key)"
        terminal = Terminal(self.log, label, [self.sherpa_path, "ssh", node], self.directory, os.environ.copy())
        try:
            terminal.wait_for(lambda text: f"Connected to {self.alias(node)}." in text, CONNECT_TIMEOUT,
                              "the connect line")
            deadline = time.monotonic() + SESSION_TIMEOUT
            while True:
                terminal.send(probe_command(node, self.owner).encode() + b"\r")
                resend = time.monotonic() + RESEND_SECONDS
                while time.monotonic() < resend:
                    text = terminal.text()
                    require(not password_prompts(text),
                            f"{label} asked for a password, so the key login failed; see {self.log.name}")
                    if not probe_problems(node, self.owner, text):
                        break
                    require(terminal.running(), f"{label} ended early; see {self.log.name}")
                    terminal.read(0.5)
                else:
                    require(time.monotonic() < deadline, f"{label}: no output of the probe command within "
                            f"{SESSION_TIMEOUT}s; see {self.log.name}")
                    continue
                break
            text = terminal.text()
            terminal.send(LEAVE_COMMANDS[node])
            if not terminal.wait_for_end(LEAVE_TIMEOUT):
                terminal.send(ESCAPE)
        except BaseException:
            terminal.process.kill()
            terminal.finish()
            raise
        terminal.finish()
        problems = probe_problems(node, self.owner, text)
        require(not problems, f"{label}: the session lacks {', '.join(problems)}; see {self.log.name}")
        require(f"Connection to {self.alias(node)} closed." in terminal.text(),
                f"{label} did not end with its close line; see {self.log.name}")

    def password_login(self, label, node, config, login, password):
        """`ssh` as `login` to a node with no key offered, answering its password prompts from here.
        Returns the exit code, the number of prompts answered and the text printed."""
        address = self.host(node).hostname
        user = ["-l", login] if login else []
        arguments = ["ssh", "-F", str(config), "-o", "LogLevel=ERROR", "-o", "ConnectTimeout=15", *user,
                     self.alias(node), probe_command(node, self.owner)]
        terminal = Terminal(self.log, label, arguments, self.directory, os.environ.copy())
        try:
            answered = answer_passwords(terminal, address, password, PASSWORD_TIMEOUT)
        except BaseException:
            terminal.process.kill()
            terminal.finish()
            raise
        code = terminal.finish()
        return code, answered, terminal.text()

    def check_password_login(self, node, config):
        """The node takes the owner's device password, with no key offered, and gives the same rights."""
        label = f"ssh {node} (device password)"
        code, answered, text = self.password_login(label, node, config, None, self.accounts.device_password)
        require(code == 0, f"{label} failed (exit {code}): {text.strip()}")
        require(answered >= 1, f"{label} was never asked for a password; see {self.log.name}")
        problems = probe_problems(node, self.owner, text)
        require(not problems, f"{label}: the session lacks {', '.join(problems)}; see {self.log.name}")

    def check_old_shared_login_refused(self, node, config):
        """The shared `sherpa` account is not on the node: neither the owner's key nor the
        owner's device password, the one password the node has, logs in as it."""
        by_key = self.ssh(f"ssh {OLD_SHARED_USER}@{node} (key)", node, "true", login=OLD_SHARED_USER)
        require(by_key.code != 0, f"{node} took an SSH login as {OLD_SHARED_USER} with the owner's key")
        require(PERMISSION_DENIED in by_key.output,
                f"the key login as {OLD_SHARED_USER} to {node} was not refused: {by_key.output.strip()}")
        label = f"ssh {OLD_SHARED_USER}@{node} (device password)"
        code, _, text = self.password_login(label, node, config, OLD_SHARED_USER, self.accounts.device_password)
        require(code != 0, f"{node} took an SSH login as {OLD_SHARED_USER} with the device password")
        require(PERMISSION_DENIED in text, f"{label} was not refused: {text.strip()}")

    # -- admin -------------------------------------------------------------------------------

    def admin_checks(self):
        self.admin_directory.mkdir()
        (self.admin_directory / ".sherpa").mkdir(mode=0o700)
        shutil.copy(self.directory / ".sherpa" / "lab-info.toml", self.admin_directory / ".sherpa" / "lab-info.toml")
        self.download_directory.mkdir()
        self.check_admin_inspect()
        self.check_admin_download_refused()
        self.check_admin_connect_refused()
        self.make_admin_key()
        for login in (self.accounts.admin, self.owner):
            for node in (VM, CONTAINER):
                self.check_admin_direct_ssh_refused(node, login)
        self.check_admin_key_add_refused()
        self.admin_destroy()

    def check_admin_inspect(self):
        """An admin sees the lab and its nodes."""
        outcome = self.admin("sherpa inspect", ["inspect"])
        rows = {row["Device"]: row for row in parse_devices_table(outcome.stdout)}
        for node in (VM, CONTAINER):
            require(self.alias(node) in rows, f"the admin's inspect does not list {node}: {sorted(rows)}")

    def check_admin_download_refused(self):
        outcome = self.admin("sherpa download", ["download", self.lab_id], cwd=self.download_directory,
                             check=False)
        require(outcome.code != 0, "the admin downloaded a lab they do not own")
        require(OWNER_ONLY_SERVER.format(lab=self.lab_id) in outcome.output,
                f"sherpa download was not refused for the owner only: {outcome.output.strip()}")
        files = [str(path) for path in (self.download_directory / ".sherpa").rglob("*") if path.is_file()] \
            if (self.download_directory / ".sherpa").exists() else []
        require(not files, f"the refused download wrote files: {files}")

    def check_admin_connect_refused(self):
        for arguments, refusal in ((["ssh", VM], OWNER_ONLY_CONNECT), (["ssh", CONTAINER], OWNER_ONLY_CONNECT),
                                   (["console", VM], OWNER_ONLY_CONSOLE)):
            outcome = self.admin(f"sherpa {' '.join(arguments)}", arguments, REFUSAL_TIMEOUT, check=False)
            command = f"sherpa {' '.join(arguments)}"
            require(outcome.code not in (0, None), f"{command} did not fail for the admin")
            require(refusal.format(lab=self.lab_id) in outcome.output,
                    f"{command} was not refused for the owner only: {outcome.output.strip()}")
            require("Connected to" not in outcome.output, f"{command} connected for the admin")

    def make_admin_key(self):
        self.admin_key.parent.mkdir(mode=0o700)
        self.command("ssh-keygen (admin key)", ["ssh-keygen", "-q", "-t", "ed25519", "-N", "", "-C",
                                                "owner-login-lab-test", "-f", str(self.admin_key)])
        (self.admin_key.parent / "ssh_config").write_text(admin_ssh_config(self.admin_key))

    def check_admin_direct_ssh_refused(self, node, login):
        """The admin's own key does not log in to the node, as the admin or as the owner."""
        marker = f"reached-{uuid.uuid4().hex[:8]}"
        label = f"ssh {login}@{node} with the admin's key"
        outcome = self.command(label, direct_ssh_arguments(self.admin_key.parent / "ssh_config", self.admin_key,
                                                           self.host(node), login, f"echo {marker}"),
                               REFUSAL_TIMEOUT, check=False)
        require(marker not in outcome.stdout and outcome.code != 0,
                f"{label} logged in: exit {outcome.code}, {outcome.output.strip()}")
        require(PERMISSION_DENIED in outcome.output,
                f"{label} was not refused with {PERMISSION_DENIED!r}: {outcome.output.strip()}")

    def owner_keys(self, label):
        outcome = self.sherpa(label, ["server", "--output", "json", "user", "ssh-key", "list"])
        try:
            return json.loads(outcome.stdout[outcome.stdout.index("{"):])["ssh_keys"]
        except (ValueError, KeyError) as error:
            raise TestFailure(f"{label} printed no list of keys: {outcome.stdout.strip()}") from error

    def check_admin_key_add_refused(self):
        """No admin can add a key to the owner's account: the server refuses with 403, and the
        owner's keys are as they were."""
        before = self.owner_keys("owner's keys before")
        home = self.accounts.admin_home / ".sherpa"
        config_file = home / "config" / "sherpa.toml"
        config = tomllib.loads(config_file.read_text()) if config_file.exists() else {}
        base_url = server_base_url(config, os.environ.get(SERVER_URL_ENV))
        context = tls_context(config, base_url, home / "trusted_certs")
        token = (home / "token").read_text().strip()
        add_secret(token)
        public_key = self.admin_key.with_name("id_ed25519.pub").read_text().strip()
        url = f"{base_url}/api/v1/admin/users/{self.owner}/ssh-keys"
        reply = rest_post(url, token, {"username": self.owner, "ssh_key": public_key, "token": ""}, context)
        self.log.write(mask(f"=== POST {url}\n--- status {reply.status}\n--- body\n{reply.body}\n"))
        self.log.flush()
        require(reply.status == 403, f"adding a key to {self.owner}'s account as the admin gave "
                f"HTTP {reply.status}, not 403: {reply.body.strip()}")
        require(OWN_ACCOUNT_ONLY.format(owner=self.owner) in reply.body,
                f"the refusal does not say the account's own user only may: {reply.body.strip()}")
        after = self.owner_keys("owner's keys after")
        require(after == before, f"the owner's keys changed: {before} -> {after}")
        require(public_key.split()[1] not in " ".join(after), "the admin's key is on the owner's account")

    def admin_destroy(self):
        """An admin may destroy a lab they do not own."""
        self.admin("sherpa destroy", ["destroy", "--yes"], UP_TIMEOUT)
        self.admin_destroyed = True

    # -- destroy -----------------------------------------------------------------------------

    def destroy(self):
        """Destroy the lab and check nothing is left. Returns a failure, or None."""
        if not self.started:
            return None
        if not (self.directory / ".sherpa" / "lab-info.toml").exists():
            # `sherpa up` failed early; destroy whatever it created, found by the manifest's lab name.
            self.sherpa("sherpa destroy (after failed up)", ["destroy", "--yes"], UP_TIMEOUT, check=False)
            return None
        try:
            if self.admin_destroyed:
                # The admin's destroy removed the admin's files; the owner's include is still in
                # ~/.ssh/sherpa_lab_hosts, and is stale now.
                self.sherpa("sherpa ssh-config clean", ["ssh-config", "clean"], check=False)
                listing = self.sherpa("sherpa ssh-config inspect", ["ssh-config", "inspect"], check=False)
                require(str(self.directory) not in listing.output,
                        f"~/.ssh/sherpa_lab_hosts still includes {self.directory}")
            else:
                self.sherpa("sherpa destroy", ["destroy", "--yes"], UP_TIMEOUT)
            inspect = self.sherpa("lab gone", ["inspect"], check=False)
            require(inspect.code != 0, f"lab {self.lab_id} still exists after destroy")
            self.no_host_leftovers()
        except TestFailure as error:
            return error
        return None

    def no_host_leftovers(self):
        """When run on the Sherpa server, check its libvirt domains, containers and host interfaces."""
        checks = (
            ("libvirt domains", ["virsh", "-c", "qemu:///system", "list", "--all", "--name"]),
            ("Docker containers", ["docker", "ps", "--all", "--format", "{{.Names}}"]),
            ("Docker networks", ["docker", "network", "ls", "--format", "{{.Name}}"]),
            ("host interfaces", ["ip", "-brief", "link"]),
        )
        for kind, arguments in checks:
            if not shutil.which(arguments[0]):
                self.log.write(f"=== {kind} leftover check skipped: {arguments[0]} not found\n")
                continue
            listing = self.command(f"no leftover {kind}", arguments, check=False)
            if listing.code != 0:
                self.log.write(f"=== {kind} leftover check skipped: exit {listing.code}\n")
                continue
            leftovers = [line.split()[0] for line in listing.stdout.splitlines() if self.lab_id in line]
            require(not leftovers, f"{kind} left after destroy: {leftovers}")


# -- preflight -----------------------------------------------------------------------------------


def user_info(log, label, run_root, env, username=None):
    arguments = ["sherpa", "server", "user", "info", *([username] if username else [])]
    info = run_logged(log, label, arguments, run_root, env=env)
    require(info.code == 0, f"could not read the user's details ({label})")
    return parse_user_info(info.stdout)


def prepare_admin_home(run_root):
    """A HOME for the admin: the owner's client config and trusted server certificate, and no token."""
    home = run_root / "admin-home"
    (home / ".sherpa").mkdir(parents=True, mode=0o700)
    source = Path.home() / ".sherpa"
    if (source / "config").is_dir():
        shutil.copytree(source / "config", home / ".sherpa" / "config")
    if (source / "trusted_certs").is_dir():
        shutil.copytree(source / "trusted_certs", home / ".sherpa" / "trusted_certs")
    env = {key: value for key, value in os.environ.items() if key not in (LOGIN_USER_ENV, LOGIN_PASSWORD_ENV)}
    env["HOME"] = str(home)
    return home, env


def preflight(run_root, admin_user, admin_password, device_password):
    for tool in ("sherpa", "ssh", "ssh-keygen"):
        require(shutil.which(tool), f"{tool} is not on PATH")
    manifest = tomllib.loads((LAB / "manifest.toml").read_text())
    for secret in (admin_password, device_password):
        add_secret(secret)
    with (run_root / "preflight.log").open("w") as log:
        whoami = run_logged(log, "sherpa whoami", ["sherpa", "whoami"], run_root)
        require(whoami.code == 0, "the sherpa CLI is not logged in (run `sherpa login`)")
        owner = user_info(log, "sherpa server user info (owner)", run_root, None)
        require(owner.username != BUILT_IN_ADMIN, "the CLI is logged in as the built-in `admin`, which cannot start labs")
        require(not owner.admin, f"the CLI's user {owner.username} is an admin: the owner must be a normal user")
        require(owner.device_password_set,
                f"{owner.username} has no device password, which `sherpa up` needs: "
                "run `sherpa server user device-password`")
        require(admin_user != BUILT_IN_ADMIN, f"--admin-user must be an admin other than the built-in `{BUILT_IN_ADMIN}`")
        require(admin_user != owner.username, "the admin and the owner must be different accounts")

        home, env = prepare_admin_home(run_root)
        login_env = {**env, LOGIN_USER_ENV: admin_user, LOGIN_PASSWORD_ENV: admin_password}
        login = run_logged(log, "sherpa login (admin)", ["sherpa", "login"], run_root, env=login_env)
        require(login.code == 0, f"could not log in as {admin_user}; see {log.name}")
        admin = user_info(log, "sherpa server user info (admin)", run_root, env, admin_user)
        require(admin.admin, f"{admin_user} is not an admin")
        require(admin.device_password_set,
                f"{admin_user} has no device password: log in as them and run `sherpa server user device-password`")

        for model, version in sorted({(node["model"], node["version"]) for node in manifest["nodes"]}):
            image = run_logged(log, f"{model} {version} image present",
                               ["sherpa", "image", "show", model, "--version", version], run_root)
            require(image.code == 0, f"the server has no {model} {version} image")
    return Accounts(owner.username, admin_user, home, env, device_password)


def main():
    parser = argparse.ArgumentParser(description=__doc__,
                                     formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--admin-user", default=os.environ.get(ENV_ADMIN_USER),
                        help=f"An admin account other than `admin` (default: ${ENV_ADMIN_USER}). Its password "
                             f"is read from ${ENV_ADMIN_PASSWORD}")
    parser.add_argument("--keep-failed", action="store_true", help="Leave a failed lab running for debugging")
    parser.add_argument("--self-test", action="store_true", help="Run the runner's own unit tests")
    args = parser.parse_args()
    if args.self_test:
        sys.argv = sys.argv[:1]
        unittest.main()
        return 0
    admin_password = os.environ.get(ENV_ADMIN_PASSWORD)
    device_password = os.environ.get(ENV_DEVICE_PASSWORD)
    missing = [name for name, value in ((f"--admin-user or ${ENV_ADMIN_USER}", args.admin_user),
                                        (f"${ENV_ADMIN_PASSWORD}", admin_password),
                                        (f"${ENV_DEVICE_PASSWORD}", device_password)) if not value]
    if missing:
        print(f"FAIL: missing {', '.join(missing)}", file=sys.stderr)
        return 2
    signal.signal(signal.SIGTERM, stop_on_sigterm)
    run_root = RUNS / f"owner-login-{uuid.uuid4().hex[:12]}"
    run_root.mkdir(parents=True)
    print(f"Logs: {run_root}", flush=True)
    try:
        accounts = preflight(run_root, args.admin_user, admin_password, device_password)
        LabRun(run_root, args.keep_failed, accounts).run()
    except TestFailure as error:
        print(f"FAIL {error}. Logs: {run_root}", file=sys.stderr)
        return 1
    except KeyboardInterrupt:
        print("interrupted", file=sys.stderr)
        return 130
    print(f"PASS. Logs: {run_root}")
    return 0


class SelfTest(unittest.TestCase):
    def manifest(self):
        return tomllib.loads((LAB / "manifest.toml").read_text())

    def test_user_info_is_read_from_the_info_output(self):
        ready = "\nUser Information:\n  Username: alice\n  Admin: No\n  SSH Keys: 1 key(s)\n  Device password: set\n"
        self.assertEqual(parse_user_info(ready), UserInfo("alice", False, True))
        self.assertEqual(parse_user_info(ready.replace("Admin: No", "Admin: Yes").replace(": set", ": not set")),
                         UserInfo("alice", True, False))
        with self.assertRaises(TestFailure):
            parse_user_info("  Device password: set\n")

    def test_preflight_stops_for_an_owner_who_is_an_admin_or_the_built_in_admin_or_has_no_device_password(self):
        info = "  Username: {}\n  Admin: {}\n  Device password: {}\n"
        for owner in (info.format("alice", "Yes", "set"), info.format("admin", "No", "set"),
                      info.format("alice", "No", "not set")):
            with tempfile.TemporaryDirectory() as temp:
                run_root = Path(temp)

                def fake(log, label, arguments, *rest, owner=owner, **kwargs):
                    return Outcome(0, owner if arguments[:2] == ["sherpa", "server"] else "ok", "")

                with (mock.patch(f"{__name__}.run_logged", fake),
                      mock.patch("shutil.which", return_value="/usr/bin/true"),
                      self.assertRaises(TestFailure)):
                    preflight(run_root, "bob", "pw", "dp")

    def test_a_secret_is_masked_in_the_log(self):
        SECRETS.clear()
        add_secret("hunter2")
        add_secret("")
        self.addCleanup(SECRETS.clear)
        self.assertEqual(mask("pw hunter2, again hunter2"), "pw ********, again ********")
        with tempfile.TemporaryDirectory() as temp:
            path = Path(temp)
            with (path / "log").open("w+") as log:
                run_logged(log, "echo", ["echo", "hunter2"], path)
                log.seek(0)
                text = log.read()
        self.assertNotIn("hunter2", text)
        self.assertIn("********", text)

    def test_the_lab_ssh_config_is_read_into_hosts(self):
        config = ("\nHost 172.31.0.21 ubu01.abcd1234 ubu01.abcd1234.sherpa.lab.local\n    HostName 172.31.0.21\n"
                  "    Port 22\n    User alice\n    StrictHostKeyChecking yes\n"
                  "Host 172.31.0.22 srl01.abcd1234 srl01.abcd1234.sherpa.lab.local\n    HostName 172.31.0.22\n"
                  "    Port 2222\n    User alice\n")
        hosts = parse_ssh_hosts(config)
        self.assertEqual(find_host(hosts, "srl01.abcd1234"),
                         SshHost(("172.31.0.22", "srl01.abcd1234", "srl01.abcd1234.sherpa.lab.local"),
                                 "172.31.0.22", 2222, "alice"))
        with self.assertRaises(TestFailure):
            find_host(hosts, "nope.abcd1234")
        with self.assertRaises(TestFailure):
            parse_ssh_hosts("Host a\n    Port 22\n")

    def test_the_devices_table_is_read_from_inspect_output(self):
        text = ("Sherpa Environment - x-abcd1234\nConnecting to server: wss://s\n"
                "┌──────────┬───────┐\n│ Property │ Value │\n└──────────┴───────┘\n\n"
                "┌──────────────────────────────────┐\n│           Active Nodes           │\n"
                "├────────────────┬─────────┬───────┤\n"
                "│ Device         │ Model   │ Memory (MB) │ Disks │\n"
                "├────────────────┼─────────┼───────┤\n"
                "│ ubu01.abcd1234 │ ubuntu  │ -           │ a.qcow2 │\n"
                "│                │         │             │ b.qcow2 │\n"
                "├────────────────┼─────────┼───────┤\n"
                "│ srl01.abcd1234 │ srlinux │ 2048        │ -       │\n"
                "└────────────────┴─────────┴───────┘\n\n│ Node A │ Node B │\n")
        rows = parse_devices_table(text)
        self.assertEqual([row["Device"] for row in rows], ["ubu01.abcd1234", "srl01.abcd1234"])
        self.assertEqual([row["Memory (MB)"] for row in rows], ["-", "2048"])
        self.assertEqual(parse_devices_table("nothing here"), [])

    def test_password_prompts_name_the_host_that_asks(self):
        text = ("alice@172.31.0.21's password: \nPermission denied, please try again.\n"
                "(alice@172.31.0.22) Password: ")
        prompts = password_prompts(text)
        self.assertEqual([(prompt.user, prompt.host) for prompt in prompts],
                         [("alice", "172.31.0.21"), ("alice", "172.31.0.22")])
        self.assertEqual(password_prompts("Connected to ubu01.abcd1234. Escape: ~. at the start of a line."), [])

    def test_the_probe_output_must_show_the_owner_and_admin_rights(self):
        linux = "alice@ubu01:~$ printf 'who:%s:\\n'\nwho:alice:\nsudo-ok-42\n"
        self.assertEqual(probe_problems(VM, "alice", linux), [])
        self.assertEqual(len(probe_problems(VM, "alice", "who:alice:\n")), 1)
        self.assertEqual(len(probe_problems(VM, "alice", "who:bob:\nsudo-ok-42\n")), 1)
        # Typing the command shows its text, which holds neither the name nor the result.
        self.assertEqual(len(probe_problems(VM, "alice", "printf 'who:%s:\\n'; echo sudo-ok-$((40+2))")), 2)
        srlinux = ('    superuser false\n    last-successful-login "2026-11-17T07:29:15.326Z (8 seconds ago)"\n'
                   '    role [\n        \x1b[0msrl-admin\n    ]\n')
        self.assertEqual(probe_problems(CONTAINER, "alice", srlinux), [])
        self.assertEqual(len(probe_problems(CONTAINER, "alice", srlinux.replace("srl-admin", "srl-viewer"))), 1)
        self.assertEqual(len(probe_problems(CONTAINER, "alice", srlinux.replace("8 seconds ago", "never"))), 1)
        self.assertEqual(probe_command(CONTAINER, "alice"),
                         "info from state /system aaa authentication user alice")

    def test_private_keys_are_found_in_the_lab_folder(self):
        with tempfile.TemporaryDirectory() as temp:
            folder = Path(temp)
            (folder / "ssh").mkdir()
            (folder / "ssh" / "config").write_text("Host a\n")
            (folder / "lab-info.toml").write_text("id = 'x'\n")
            (folder / "ca").mkdir()
            (folder / "ca" / "lab-ca.crt").write_text("-----BEGIN CERTIFICATE-----\nabc\n")
            self.assertEqual(private_key_files(folder), [])
            (folder / "ssh" / "other").write_text("-----BEGIN OPENSSH PRIVATE KEY-----\nabc\n")
            (folder / "k.pem").write_text("-----BEGIN RSA PRIVATE KEY-----\nabc\n")
            self.assertEqual([path.name for path in private_key_files(folder)], ["k.pem", "other"])

    def test_the_password_config_takes_keys_away_from_the_nodes_only(self):
        text = password_ssh_config("Host x.1\n    User alice\n", ["a.1", "b.1"])
        self.assertTrue(text.startswith("Host a.1\n    PubkeyAuthentication no\n"))
        self.assertEqual(text.count("PubkeyAuthentication no"), 2)
        self.assertEqual(text.count("Host "), 3)
        self.assertTrue(text.endswith("Host x.1\n    User alice\n"))
        self.assertNotIn("Host *", text)

    def test_the_admin_ssh_uses_only_the_admins_key(self):
        host = SshHost(("172.31.0.21",), "172.31.0.21", 22, "alice")
        arguments = direct_ssh_arguments(Path("/run/ssh_config"), Path("/run/key"), host, "dana", "true")
        self.assertEqual(arguments[:2], ["ssh", "-F"])
        for option in ("BatchMode=yes", "IdentitiesOnly=yes"):
            self.assertIn(option, arguments)
        self.assertEqual(arguments[arguments.index("-i") + 1], "/run/key")
        self.assertEqual(arguments[arguments.index("-l") + 1], "dana")
        self.assertEqual(arguments[-2:], ["172.31.0.21", "true"])
        config = admin_ssh_config(Path("/run/key"))
        for line in ("IdentityFile /run/key", "IdentitiesOnly yes", "IdentityAgent none", "BatchMode yes"):
            self.assertIn(line, config)

    def test_the_rest_address_comes_from_the_environment_the_config_or_its_defaults(self):
        config = {"server_connection": {"url": "wss://10.0.0.1:3030/ws"}}
        self.assertEqual(server_base_url(config, None), "https://10.0.0.1:3030")
        self.assertEqual(server_base_url(config, "ws://localhost:4030/ws"), "http://localhost:4030")
        self.assertEqual(server_base_url({"server_ipv4": "10.0.0.2", "ws_port": 3040}, None), "https://10.0.0.2:3040")
        self.assertEqual(server_base_url({"tls": {"enabled": False}}, None), "http://127.0.0.1:3030")
        with self.assertRaises(TestFailure):
            server_base_url({}, "ftp://x/ws")

    def test_the_tls_check_follows_the_client_config(self):
        with tempfile.TemporaryDirectory() as temp:
            trust = Path(temp)
            self.assertIsNone(tls_context({}, "http://127.0.0.1:3030", trust))
            self.assertEqual(trusted_certificate(trust, "https://10.0.0.1:3030"), trust / "10.0.0.1_3030.pem")
            insecure = tls_context({"server_connection": {"insecure": True}}, "https://10.0.0.1:3030", trust)
            self.assertEqual(insecure.verify_mode, ssl.CERT_NONE)
            default = tls_context({}, "https://10.0.0.1:3030", trust)
            self.assertEqual(default.verify_mode, ssl.CERT_REQUIRED)
            self.assertTrue(default.check_hostname)

    def test_a_refusal_is_a_reply_with_its_status_and_body_and_carries_the_token(self):
        seen = {}

        class Handler(BaseHTTPRequestHandler):
            def do_POST(self):
                seen["auth"] = self.headers["Authorization"]
                seen["path"] = self.path
                seen["body"] = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
                body = json.dumps({"error": {"code": "FORBIDDEN", "message": "Access denied",
                                             "details": "only user 'alice' can do this for their own account"}})
                self.send_response(403)
                self.send_header("Content-Type", "application/json")
                self.send_header("Content-Length", str(len(body)))
                self.end_headers()
                self.wfile.write(body.encode())

            def log_message(self, *args):
                pass

        server = HTTPServer(("127.0.0.1", 0), Handler)
        thread = threading.Thread(target=server.serve_forever)
        thread.start()
        self.addCleanup(thread.join)
        self.addCleanup(server.server_close)
        self.addCleanup(server.shutdown)
        url = f"http://127.0.0.1:{server.server_port}/api/v1/admin/users/alice/ssh-keys"
        reply = rest_post(url, "tok", {"username": "alice", "ssh_key": "ssh-ed25519 AAAA", "token": ""}, None)
        self.assertEqual(reply.status, 403)
        self.assertIn(OWN_ACCOUNT_ONLY.format(owner="alice"), reply.body)
        self.assertEqual(seen["auth"], "Bearer tok")
        self.assertEqual(seen["path"], "/api/v1/admin/users/alice/ssh-keys")
        self.assertEqual(seen["body"], {"username": "alice", "ssh_key": "ssh-ed25519 AAAA", "token": ""})
        with self.assertRaises(TestFailure):
            rest_post("http://127.0.0.1:1/x", "tok", {}, None)

    def test_a_terminal_is_the_commands_controlling_terminal_and_takes_typed_input(self):
        script = ("import sys\nopen('/dev/tty', 'w').write('tty-ok\\n')\n"
                  "sys.stdout.write('pw: '); sys.stdout.flush()\n"
                  "print('got-' + sys.stdin.readline().strip())\n")
        with tempfile.TemporaryDirectory() as temp:
            log = io.StringIO()
            terminal = Terminal(log, "tty check", [sys.executable, "-I", "-c", script], Path(temp), os.environ.copy())
            terminal.wait_for(lambda text: "pw: " in text, 20, "the prompt")
            terminal.send(b"secret\r")
            self.assertEqual(terminal.finish(), 0)
        self.assertIn("tty-ok", terminal.text())
        self.assertIn("got-secret", terminal.text())
        self.assertIn("=== tty check", log.getvalue())

    def test_a_password_is_typed_at_the_nodes_prompt_and_not_at_another_hosts(self):
        script = ("import sys\nfor _ in range(2):\n    sys.stdout.write(\"{}@{}'s password: \")\n"
                  "    sys.stdout.flush()\n    sys.stdin.readline()\n    print('Permission denied, please try again.')\n")
        for host, passes in (("10.0.0.5", True), ("10.0.0.1", False)):
            with tempfile.TemporaryDirectory() as temp:
                terminal = Terminal(io.StringIO(), "prompts", [sys.executable, "-I", "-c",
                                                              script.format("alice", host)],
                                    Path(temp), os.environ.copy())
                if passes:
                    self.assertEqual(answer_passwords(terminal, "10.0.0.5", "pw", 20), 2)
                    terminal.finish()
                else:
                    with self.assertRaises(TestFailure):
                        answer_passwords(terminal, "10.0.0.5", "pw", 20)
                    terminal.process.kill()
                    terminal.finish()

    def test_the_manifest_has_a_pinned_linux_vm_and_a_network_os_container(self):
        nodes = self.manifest()["nodes"]
        self.assertEqual([(node["name"], node["model"]) for node in nodes],
                         [(VM, "ubuntu_linux"), (CONTAINER, "nokia_srlinux")])
        self.assertTrue(all(node.get("version") for node in nodes))
        self.assertEqual(set(READY_COMMANDS), {VM, CONTAINER})
        self.assertEqual(set(PROBE_COMMANDS), {VM, CONTAINER})
        self.assertEqual(set(LEAVE_COMMANDS), {VM, CONTAINER})

    def test_the_rendered_manifest_round_trips_with_a_new_name(self):
        manifest = self.manifest()
        rendered = tomllib.loads(render_manifest(manifest, "renamed"))
        self.assertEqual(rendered["name"], "renamed")
        self.assertEqual(rendered["nodes"], manifest["nodes"])

    def test_a_partly_created_lab_keeps_its_id_for_cleanup(self):
        # `sherpa up` exits 1 on a partly created lab after writing lab-info.toml; the run must
        # still know the lab's ID, so destroy checks for leftovers instead of crashing.
        with tempfile.TemporaryDirectory() as temp:
            accounts = Accounts("alice", "dana", Path(temp) / "home", {}, "dp")
            run = LabRun(Path(temp) / "owner-login-000000abcdef", keep_failed=False, accounts=accounts)
            (run.directory / ".sherpa").mkdir(parents=True)
            (run.directory / ".sherpa" / "lab-info.toml").write_text(f'id = "4ed6360e"\nname = "{run.name}"\n')
            run.log = (run.directory / "commands.log").open("w")
            self.addCleanup(run.log.close)

            def failing_up(log, label, arguments, *rest, **kwargs):
                return Outcome(1 if arguments[1:2] == ["up"] else 0, "", "")

            with mock.patch(f"{__name__}.run_logged", failing_up), self.assertRaises(TestFailure):
                run.up()
            self.assertEqual(run.lab_id, "4ed6360e")

            def leftover(log, label, arguments, *rest, **kwargs):
                return Outcome(1 if "inspect" in arguments and "ssh-config" not in arguments else 0,
                               "node-4ed6360e-x\n", "")

            with (mock.patch(f"{__name__}.run_logged", leftover),
                  mock.patch("shutil.which", return_value="/usr/bin/true")):
                self.assertIsInstance(run.destroy(), TestFailure)
                # After the admin destroyed the lab, the owner's destroy is not run again.
                calls = []

                def recording(log, label, arguments, *rest, **kwargs):
                    calls.append(arguments[1:3])
                    return Outcome(1 if arguments[1:2] == ["inspect"] else 0, "", "")

                run.admin_destroyed = True
                with mock.patch(f"{__name__}.run_logged", recording):
                    self.assertIsNone(run.destroy())
                self.assertNotIn(["destroy", "--yes"], calls)
                self.assertIn(["ssh-config", "clean"], calls)


if __name__ == "__main__":
    sys.exit(main())
