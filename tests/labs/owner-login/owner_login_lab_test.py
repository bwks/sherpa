#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.15"
# dependencies = ["pytest==9.2.0"]
# ///
"""End-to-end test of the lab owner's login on a lab's nodes, of what an admin who does not own the
lab may do to it, and of down, resume and redeploy with the owner's keys (Python 3.15+, pytest).

With the runner's own CLI login, which must be an admin, the run creates a throwaway admin, and as
that admin, which works with a HOME of its own, a throwaway owner: a normal Sherpa user. Each has a
unique name and a login password made for the run, and the owner a device password and SSH key
too. The owner's CLI works with a HOME of its own as well. As the owner it brings up a lab with an
Ubuntu VM and an SR Linux container, and checks the `owner_login` and `access_control` cases of
tests/specs/accounts-and-auth.md, the `lab_down_resume` and `lab_redeploy` cases of
tests/specs/lab-lifecycle.md, and the `ssh_config_hosts` and `stale_includes` cases of
tests/specs/client-lab-files.md. The lab's SSH config reaches the nodes directly, so the runner
must have a route to the lab's management network, such as the Sherpa server itself. At the end,
even when a case fails, the lab is destroyed and both accounts deleted. See README.md.

A pytest module: the `session` fixture makes the accounts and the `lab` fixture brings the lab up
once for the module; the cases are the test functions taking them, in case order. Run as a script,
it runs pytest on itself; `--self-test` runs the runner's own unit tests instead.
"""

import argparse
from http.server import BaseHTTPRequestHandler, HTTPServer
import io
import json
import os
from pathlib import Path
import re
import shutil
import signal
import ssl
import sys
import tempfile
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

import pytest

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))

from labkit import (
    COMMAND_TIMEOUT,
    DEVICE_PASSWORD_SET,
    INFO_ADMIN,
    INFO_USERNAME,
    Terminal,
    TestFailure,
    add_secret,
    answer_prompts,
    generate_password,
    log_name,
    mask,
    parse_key_list,
    require,
    run_logged,
    stop_on_sigterm,
    value_toml,
)


LAB = Path(__file__).resolve().parent
ROOT = LAB.parent.parent.parent
RUNS = ROOT / ".tmp" / "lab-tests"

# What `sherpa login` reads, and what a client run reads for its server. The runner's own login
# variables are never passed on to a child.
LOGIN_USER_ENV = "SHERPA_USER_NAME"
LOGIN_PASSWORD_ENV = "SHERPA_USER_PASSWORD"
SERVER_URL_ENV = "SHERPA_SERVER_URL"
PRIVATE_ENV = (LOGIN_USER_ENV, LOGIN_PASSWORD_ENV, "SHERPA_ADMIN_PASSWORD", "SHERPA_USER_SSH_KEY",
               "SHERPA_USER_IS_ADMIN", "SSH_AUTH_SOCK")

VM = "ubu01"
CONTAINER = "srl01"

# What Sherpa says when it refuses, matched as text contained in the output. `{lab}` is the lab's ID
# and `{owner}` the owner's name.
OWNER_ONLY_SERVER = "Only the owner of lab '{lab}' can perform this operation"
OWNER_ONLY_CONNECT = "Only the owner of lab '{lab}' can connect to its nodes"
# The server's refusal of a console, serial or VNC, to anyone but the lab's owner.
OWNER_ONLY_CONSOLE = "only the owner of lab '{lab}' can open its consoles"
OWN_ACCOUNT_ONLY = "only user '{owner}' can do this for their own account"
NO_SSH_KEY = "has no SSH public key registered"

# The prompts of the commands that take a password, which read it from the terminal.
CREATE_PROMPTS = ("Password for {user}: ", "Confirm password: ")
DEVICE_PASSWORD_PROMPTS = ("Sherpa login password for {owner}: ", "New device password: ",
                           "Confirm device password: ")

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
BOOT_ID_COMMAND = "cat /proc/sys/kernel/random/boot_id"
# Prints who is logged in, their home and login shell (the typed command shows `%s`, never the
# values), and succeeds if the home is theirs and sudo needs no password: the owner's account and
# administrator rights on a Linux node.
LINUX_COMMAND = ("printf 'who:%s:home:%s:shell:%s:\\n' \"$(id -un)\" \"$HOME\" "
                 "\"$(getent passwd \"$(id -un)\" | cut -d: -f7)\"; "
                 "test -d \"$HOME\" -a -O \"$HOME\" && echo home-ok-$((40+2)); "
                 "sudo -n true && echo sudo-ok-$((40+2))")
# SR Linux's state of the owner's account (`{owner}` filled in): its roles, and its last successful
# login, which the session running the command has just made. The owner's role is `srl-admin`,
# whose `superuser` flag the generated config sets. (SR Linux 26.7's session list needs a session
# id, so it cannot be listed whole.)
SRLINUX_COMMAND = "info from state /system aaa authentication user {owner}"
PROBE_COMMANDS = {VM: LINUX_COMMAND, CONTAINER: SRLINUX_COMMAND}
# What leaves each node's shell: the shell's own `exit`, and SR Linux's `quit`.
LEAVE_COMMANDS = {VM: b"exit\r", CONTAINER: b"quit\r"}
ESCAPE = b"\r~."

PASSWORD_PROMPT = re.compile(
    r"(?P<user>[^\s@()']+)@(?P<host>[^\s')]+)'s password: |"
    r"\((?P<kuser>[^\s@()]+)@(?P<khost>[^\s)]+)\) Password: ")
# A terminal control sequence, such as the colours and cursor moves of SR Linux's CLI.
ANSI_ESCAPE = re.compile(r"\x1b\[[0-9;?]*[A-Za-z]")
CELL_SEPARATOR = "│"


class SshHost(NamedTuple):
    """One `Host` block of the lab's SSH config."""

    aliases: tuple
    hostname: str
    port: int
    user: str
    identity_files: tuple = ()


class PasswordPrompt(NamedTuple):
    user: str
    host: str
    end: int


class RestReply(NamedTuple):
    status: int
    body: str


class UserInfo(NamedTuple):
    username: str
    admin: bool
    device_password_set: bool


def render_manifest(manifest, name):
    """The lab manifest for one run, under a unique lab name."""
    lines = [f"name = {value_toml(name)}", "", "nodes = ["]
    lines += [f"  {value_toml(node)}," for node in manifest["nodes"]]
    return "\n".join([*lines, "]"]) + "\n"


def child_env(home):
    """The environment of a `sherpa` or `ssh` run with `home` as its HOME: the runner's, without its
    own Sherpa login, its passwords or its SSH agent."""
    env = {key: value for key, value in os.environ.items() if key not in PRIVATE_ENV}
    env["HOME"] = str(home)
    return env


# -- parsing -------------------------------------------------------------------------------------


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
            blocks.append({"aliases": tuple(value.split()), "IdentityFile": ()})
        elif blocks and key.lower() == "identityfile":
            blocks[-1]["IdentityFile"] += (value,)
        elif blocks and key in ("HostName", "Port", "User"):
            blocks[-1][key] = value
    hosts = []
    for block in blocks:
        require("HostName" in block and "User" in block,
                f"the lab SSH config has a Host block without a HostName or User: {block}")
        hosts.append(SshHost(block["aliases"], block["HostName"], int(block.get("Port", "22")), block["User"],
                             block["IdentityFile"]))
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


def key_blob(public_key):
    """The base64 body of an OpenSSH public key line, which identifies the key whatever its comment."""
    parts = public_key.split()
    require(len(parts) >= 2, f"not an OpenSSH public key: {public_key!r}")
    return parts[1]


def scanned_host_keys(text):
    """The `(type, key)` pairs in `ssh-keyscan` output."""
    keys = set()
    for line in text.splitlines():
        parts = line.split()
        if len(parts) >= 3 and not line.startswith("#"):
            keys.add((parts[1], parts[2]))
    return keys


def password_prompts(text):
    """Every password prompt OpenSSH printed: for `user@host's password: `, and for a
    keyboard-interactive `(user@host) Password: `."""
    prompts = []
    for found in PASSWORD_PROMPT.finditer(text):
        prompts.append(PasswordPrompt(found.group("user") or found.group("kuser"),
                                      found.group("host") or found.group("khost"), found.end()))
    return prompts


def probe_command(node, owner):
    """The command that shows `owner`'s login and administrator rights on `node`."""
    return PROBE_COMMANDS[node].format(owner=owner)


def probe_problems(node, owner, text):
    """What the output of the node's probe command lacks, or an empty list when it shows the owner's
    account and administrator rights."""
    if node == VM:
        wanted = {f"the login `{owner}`": rf"who:{re.escape(owner)}:",
                  f"the home /home/{owner}": rf":home:/home/{re.escape(owner)}:",
                  "bash as the login shell": r":shell:(?:/usr)?/bin/bash:",
                  "a home of the owner's own": r"home-ok-42",
                  "passwordless sudo": r"sudo-ok-42"}
    else:
        wanted = {f"a login of `{owner}`": r'last-successful-login\s+"[^"]*\((?:\d+|an?) (?:seconds?|minutes?) ago\)"',
                  "the `srl-admin` role": r"role\s*\[\s*srl-admin\s*\]"}
    text = ANSI_ESCAPE.sub("", text)
    return [what for what, pattern in wanted.items() if not re.search(pattern, text)]


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
            "    UserKnownHostsFile /dev/null\n    ConnectTimeout 15\n")


def direct_ssh_arguments(config, key, host, login, command):
    """`ssh` straight to a node's management address, as `login`, with the admin's key, verbose so
    the output shows which host answered and refused."""
    return ["ssh", "-v", "-F", str(config), "-o", "BatchMode=yes", "-o", "IdentitiesOnly=yes", "-i", str(key),
            "-p", str(host.port), "-l", login, host.hostname, command]


def refused_at_node(output, host):
    """Why `ssh -v` output does not show the node at `host` itself refusing the login, or None: the
    connection went to its address, the key exchange with it finished, and it denied the login."""
    if f"Connecting to {host.hostname} [{host.hostname}] port {host.port}" not in output:
        return f"ssh did not connect to the node's address {host.hostname}:{host.port}"
    if "Server host key:" not in output:
        return "ssh did not finish a key exchange with the node"
    if not re.search(r"Permission denied \(publickey", output):
        return f"the node did not refuse the key with {PERMISSION_DENIED!r}"
    return None


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
            terminal.kill()
            terminal.finish()
            raise TestFailure(f"{terminal.label}: ssh asked about a host key; see {log_name(terminal.log)}")
        if not terminal.running():
            return answered
        require(time.monotonic() < deadline, f"{terminal.label}: still running after {timeout}s")
        terminal.read(0.5)


# -- accounts ------------------------------------------------------------------------------------


def prepare_home(home):
    """A HOME for one account's CLI: the runner's client config and trusted server certificates, and
    no token."""
    (home / ".sherpa").mkdir(parents=True, mode=0o700)
    source = Path.home() / ".sherpa"
    for part in ("config", "trusted_certs"):
        if (source / part).is_dir():
            shutil.copytree(source / part, home / ".sherpa" / part)
    return child_env(home)


def make_key(log, path, cwd):
    """A new ed25519 key pair at `path`, with no passphrase; returns the public key line."""
    path.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
    outcome = run_logged(log, f"ssh-keygen {path.name}", ["ssh-keygen", "-q", "-t", "ed25519", "-N", "", "-C",
                                                          "owner-login-lab-test", "-f", str(path)], cwd)
    require(outcome.code == 0, f"ssh-keygen failed: {outcome.output.strip()}")
    return path.with_name(f"{path.name}.pub").read_text().strip()


class Session:
    """The run's two throwaway accounts: the admin, which the runner's own login creates, and the
    owner, which the admin creates. Each works with its own HOME; their passwords live only in this
    object and in the environment of the child that needs one."""

    def __init__(self, run_root, private_root):
        self.run_root = run_root
        self.private = private_root
        self.log = (run_root / "commands.log").open("w")
        # The runner's own login, with its own HOME, which only creates and deletes the admin.
        self.runner_env = child_env(Path.home())
        self.admin_user = f"labadmin{uuid.uuid4().hex[:8]}"
        self.admin_password = generate_password(self.admin_user)
        self.admin_created = False
        self.owner = f"owner{uuid.uuid4().hex[:8]}"
        self.owner_password = generate_password(self.owner)
        self.device_password = generate_password(self.owner)
        self.admin_home = private_root / "admin-home"
        self.owner_home = private_root / "owner-home"
        self.admin_env = None
        self.owner_env = None
        self.owner_created = False
        # The owner's key that `sherpa ssh` uses, in the owner's HOME, and their keys by name.
        self.keys = {}
        self.sherpa_path = shutil.which("sherpa") or "sherpa"

    def __repr__(self):
        return f"Session(admin={self.admin_user!r}, owner={self.owner!r})"

    def key_path(self, name):
        return self.private / "keys" / name

    def admin(self, label, arguments, cwd=None, timeout=COMMAND_TIMEOUT, check=True):
        """A `sherpa` command as the admin, with the admin's own HOME."""
        return self.command(f"{label} (admin)", [self.sherpa_path, *arguments], cwd, timeout, check, self.admin_env)

    def as_owner(self, label, arguments, cwd=None, timeout=COMMAND_TIMEOUT, check=True):
        """A `sherpa` command as the owner, with the owner's own HOME."""
        return self.command(label, [self.sherpa_path, *arguments], cwd, timeout, check, self.owner_env)

    def command(self, label, arguments, cwd=None, timeout=COMMAND_TIMEOUT, check=True, env=None):
        outcome = run_logged(self.log, label, arguments, cwd or self.run_root, timeout, env)
        if check and outcome.code != 0:
            raise TestFailure(mask(f"{label} failed (exit {outcome.code}): {outcome.output.strip()[-600:]}"))
        return outcome

    def login(self, label, home_env, username, password):
        login_env = {**home_env, LOGIN_USER_ENV: username, LOGIN_PASSWORD_ENV: password}
        outcome = run_logged(self.log, label, [self.sherpa_path, "login"], self.run_root, env=login_env)
        require(outcome.code == 0, f"could not log in as {username}; see {self.log.name}")

    def user_info(self, label, env, username=None):
        arguments = [self.sherpa_path, "server", "user", "info", *([username] if username else [])]
        info = run_logged(self.log, label, arguments, self.run_root, env=env)
        require(info.code == 0, f"could not read the user's details ({label})")
        return parse_user_info(info.stdout)

    def preflight(self, models):
        """Check the tools and that the runner's login is an admin, create the admin and log it in,
        and check the lab's images."""
        for tool in ("sherpa", "ssh", "ssh-keygen", "ssh-keyscan"):
            require(shutil.which(tool), f"{tool} is not on PATH")
        runner = self.user_info("sherpa server user info (runner)", self.runner_env)
        require(runner.admin, f"the runner's login, {runner.username}, is not an admin, so it cannot create the run's admin")
        self.admin_created = True
        self.create_user("runner", self.runner_env, self.admin_user, self.admin_password, admin=True)
        self.admin_env = prepare_home(self.admin_home)
        self.login("sherpa login (admin)", self.admin_env, self.admin_user, self.admin_password)
        token = self.admin_home / ".sherpa" / "token"
        if token.exists():
            add_secret(token.read_text().strip())
        admin = self.user_info("sherpa server user info (admin)", self.admin_env, self.admin_user)
        require(admin.admin, f"{self.admin_user} is not an admin")
        for model, version in models:
            image = self.admin(f"{model} {version} image present", ["image", "show", model, "--version", version],
                               check=False)
            require(image.code == 0, f"the server has no {model} {version} image")

    def create_user(self, creator, env, username, password, admin=False):
        """Create `username` as `creator`, whose CLI runs with `env`, typing `password` at the prompts."""
        label = f"sherpa server user create {username} ({creator})"
        arguments = [self.sherpa_path, "server", "user", "create", username, *(["--admin"] if admin else [])]
        terminal = Terminal(self.log, label, arguments, self.run_root, env)
        prompts = [prompt.format(user=username) for prompt in CREATE_PROMPTS]
        code, text = answer_prompts(terminal, [(prompts[0], password), (prompts[1], password)])
        require(code == 0, f"{label} failed (exit {code}): {text.strip()[-600:]}")

    def create_owner(self):
        """Create the owner, a normal user with a login and a device password and no SSH key, and
        log their CLI in."""
        self.owner_created = True
        self.create_user("admin", self.admin_env, self.owner, self.owner_password)
        self.owner_env = prepare_home(self.owner_home)
        self.login("sherpa login (owner)", self.owner_env, self.owner, self.owner_password)
        token = self.owner_home / ".sherpa" / "token"
        if token.exists():
            add_secret(token.read_text().strip())
        label = "sherpa server user device-password"
        terminal = Terminal(self.log, label, [self.sherpa_path, "server", "user", "device-password"],
                            self.run_root, self.owner_env)
        prompts = [prompt.format(owner=self.owner) for prompt in DEVICE_PASSWORD_PROMPTS]
        code, text = answer_prompts(terminal, [(prompts[0], self.owner_password), (prompts[1], self.device_password),
                                               (prompts[2], self.device_password)])
        require(code == 0, f"{label} failed (exit {code}): {text.strip()[-600:]}")
        info = self.user_info("sherpa server user info (owner)", self.owner_env)
        require(info == UserInfo(self.owner, False, True),
                f"the new owner is not a normal user with a device password: {info}")
        require(not self.owner_keys("owner's keys at creation"), "the new owner has SSH keys")

    def owner_keys(self, label):
        return parse_key_list(self.as_owner(label, ["server", "--output", "json", "user", "ssh-key", "list"]).stdout)

    def new_key(self, name):
        """A new key pair for the owner, kept by `name`; returns its public key file."""
        public = make_key(self.log, self.key_path(name), self.run_root)
        self.keys[name] = self.key_path(name)
        return self.key_path(name).with_name(f"{name}.pub"), public

    def add_key(self, name):
        public_file = self.key_path(name).with_name(f"{name}.pub")
        self.as_owner(f"ssh-key add {name}", ["server", "user", "ssh-key", "add", str(public_file)])

    def remove_key(self, name):
        public_file = self.key_path(name).with_name(f"{name}.pub")
        self.as_owner(f"ssh-key remove {name}", ["server", "user", "ssh-key", "remove", str(public_file)])

    def profile_key_names(self):
        """The names of the run's keys now on the owner's account."""
        on_account = {key_blob(key) for key in self.owner_keys("owner's keys")}
        return [name for name, path in self.keys.items()
                if key_blob(path.with_name(f"{name}.pub").read_text()) in on_account]

    def delete_user(self, deleter, env, username):
        """Delete `username` as `deleter`, whose CLI runs with `env`, and check it is gone. Returns a
        failure, or None."""
        try:
            self.command(f"sherpa server user delete {username} ({deleter})",
                         [self.sherpa_path, "server", "user", "delete", username, "--force"], env=env)
            listing = self.command(f"sherpa server user list ({deleter})", [self.sherpa_path, "server", "user", "list"],
                                   env=env)
            require(not re.search(rf"\b{re.escape(username)}\b", listing.stdout),
                    f"user {username} still exists after delete")
        except TestFailure as error:
            return error
        return None

    def delete_accounts(self):
        """Delete the owner as the admin, then the admin as the runner. Returns the first failure, or
        None."""
        failures = []
        if self.owner_created:
            failures.append(self.delete_user("admin", self.admin_env, self.owner))
        if self.admin_created:
            failures.append(self.delete_user("runner", self.runner_env, self.admin_user))
        return next((failure for failure in failures if failure), None)


# -- lab -----------------------------------------------------------------------------------------


class Lab:
    """The lab every case uses, brought up by the owner once by `up` and destroyed by `destroy`."""

    def __init__(self, session):
        self.session = session
        self.owner = session.owner
        self.manifest = tomllib.loads((LAB / "manifest.toml").read_text())
        self.name = f"{self.manifest['name']}-{session.run_root.name[-6:]}"
        self.directory = session.run_root / "lab"
        # The admin's folder holds a copy of the owner's lab-info.toml and nothing else; it is how
        # an admin names a lab they do not own. `sherpa download` needs a folder with no lab files.
        self.admin_directory = session.run_root / "admin"
        self.download_directory = session.run_root / "admin-download"
        self.started = False
        self.admin_destroyed = False
        self.lab_id = None
        self.hosts = []
        # The run's key each node was built with, by node.
        self.node_keys = {}
        self.log = session.log

    def sherpa(self, label, arguments, timeout=COMMAND_TIMEOUT, check=True):
        """A `sherpa` command as the owner, in the lab folder."""
        return self.session.as_owner(label, arguments, self.directory, timeout, check)

    def admin(self, label, arguments, timeout=COMMAND_TIMEOUT, check=True, cwd=None):
        """A `sherpa` command as the admin, in the admin's folder."""
        return self.session.admin(label, arguments, cwd or self.admin_directory, timeout, check)

    def alias(self, node):
        return f"{node}.{self.lab_id}"

    def host(self, node):
        return find_host(self.hosts, self.alias(node))

    def ssh(self, label, node, remote_command, key, login=None):
        """`ssh` to a node as the lab's SSH config does, with the owner's key `key` and no other."""
        user = ["-l", login] if login else []
        return self.session.command(label, ["ssh", "-F", ".sherpa/ssh/config", *SSH_OPTIONS, "-i", str(key),
                                            "-o", "IdentitiesOnly=yes", "-o", "IdentityAgent=none", *user,
                                            self.alias(node), remote_command],
                                    self.directory, check=False, env=self.session.owner_env)

    def read_lab_files(self):
        config = (self.directory / ".sherpa" / "ssh" / "config").read_text()
        self.hosts = parse_ssh_hosts(config)

    def up(self, key_name):
        self.directory.mkdir(parents=True)
        (self.directory / "manifest.toml").write_text(render_manifest(self.manifest, self.name))
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
        self.read_lab_files()
        for node in (VM, CONTAINER):
            self.node_keys[node] = self.session.keys[key_name]
            self.wait_for_login(node)
        self.admin_directory.mkdir()
        (self.admin_directory / ".sherpa").mkdir(mode=0o700)
        shutil.copy(info_path, self.admin_directory / ".sherpa" / "lab-info.toml")
        self.download_directory.mkdir()

    def wait_for_login(self, node, timeout=READY_TIMEOUT):
        """The node takes the owner's SSH login with the key it was built with, retrying while it boots."""
        deadline = time.monotonic() + timeout
        while self.ssh(f"{node} reachable", node, READY_COMMANDS[node], self.node_keys[node]).code != 0:
            require(time.monotonic() < deadline, f"no SSH login as {self.owner} on {node} within {timeout}s")
            time.sleep(POLL_INTERVAL)

    def boot_id(self, label):
        outcome = self.ssh(label, VM, BOOT_ID_COMMAND, self.node_keys[VM])
        require(outcome.code == 0, f"{label}: no login to {VM}: {outcome.output.strip()}")
        return outcome.stdout.strip()

    def host_keys(self, label, node):
        host = self.host(node)
        outcome = self.session.command(label, ["ssh-keyscan", "-T", "15", "-p", str(host.port), host.hostname],
                                       self.directory, check=False)
        keys = scanned_host_keys(outcome.stdout)
        require(keys, f"{label}: {node} offered no host key: {outcome.output.strip()}")
        return keys

    def down_and_resume(self):
        """Take the lab down and resume it; returns the resume's Outcome."""
        self.sherpa("sherpa down", ["down"], UP_TIMEOUT)
        return self.sherpa("sherpa resume", ["resume"], UP_TIMEOUT)

    def destroy(self):
        """Destroy the lab, unless the admin did, and check nothing is left. Returns a failure, or None."""
        if not self.started:
            return None
        if not (self.directory / ".sherpa" / "lab-info.toml").exists():
            # `sherpa up` failed early; destroy whatever it created, found by the manifest's lab name.
            self.sherpa("sherpa destroy (after failed up)", ["destroy", "--yes"], UP_TIMEOUT, check=False)
            return None
        try:
            if not self.admin_destroyed:
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
            listing = self.session.command(f"no leftover {kind}", arguments, check=False)
            if listing.code != 0:
                self.log.write(f"=== {kind} leftover check skipped: exit {listing.code}\n")
                continue
            leftovers = [line.split()[0] for line in listing.stdout.splitlines() if self.lab_id in line]
            require(not leftovers, f"{kind} left after destroy: {leftovers}")

    # -- checks the cases share --------------------------------------------------------------

    def key_login(self, node):
        """`sherpa ssh` logs in by the owner's key, runs the node's probe command and leaves; returns
        the session's text."""
        label = f"sherpa ssh {node} (key)"
        terminal = Terminal(self.log, label, [self.session.sherpa_path, "ssh", node], self.directory,
                            self.session.owner_env)
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
            terminal.kill()
            terminal.finish()
            raise
        terminal.finish()
        problems = probe_problems(node, self.owner, text)
        require(not problems, f"{label}: the session lacks {', '.join(problems)}; see {self.log.name}")
        require(f"Connection to {self.alias(node)} closed." in terminal.text(),
                f"{label} did not end with its close line; see {self.log.name}")

    def password_login(self, label, node, config):
        """`ssh` to a node with no key offered, answering its password prompts with the owner's device
        password. Returns the exit code, the number of prompts answered and the text printed."""
        address = self.host(node).hostname
        arguments = ["ssh", "-F", str(config), "-o", "LogLevel=ERROR", "-o", "ConnectTimeout=15",
                     "-o", "IdentityAgent=none", self.alias(node), probe_command(node, self.owner)]
        terminal = Terminal(self.log, label, arguments, self.directory, self.session.owner_env)
        try:
            answered = answer_passwords(terminal, address, self.session.device_password, PASSWORD_TIMEOUT)
        except BaseException:
            terminal.kill()
            terminal.finish()
            raise
        code = terminal.finish()
        return code, answered, terminal.text()

    def redeploy(self, label, node, check=True):
        return self.sherpa(label, ["redeploy", "--node", node], UP_TIMEOUT, check)


# -- fixtures ------------------------------------------------------------------------------------


@pytest.fixture(scope="module")
def session(request):
    """The throwaway admin the runner's login creates, logged in, and the throwaway owner it creates,
    with a login and a device password and no SSH key yet. Both are deleted at the end, even when a
    case fails, unless `--keep-failed` is given and something failed."""
    signal.signal(signal.SIGTERM, stop_on_sigterm)
    keep_failed = request.config.getoption("keep_failed")
    run_root = RUNS / f"owner-login-{uuid.uuid4().hex[:12]}"
    run_root.mkdir(parents=True)
    # The accounts' HOMEs, holding their tokens, and the owner's private keys.
    private_root = Path(tempfile.mkdtemp(prefix="owner-login-"))
    print(f"Logs: {run_root}", flush=True)
    manifest = tomllib.loads((LAB / "manifest.toml").read_text())
    models = sorted({(node["model"], node["version"]) for node in manifest["nodes"]})
    session = Session(run_root, private_root)
    keep = False
    try:
        try:
            session.preflight(models)
            session.create_owner()
        except Exception:
            keep = keep_failed
            raise
        yield session
        keep = keep_failed and request.session.testsfailed > 0
    finally:
        if keep:
            print(f"keeping the admin {session.admin_user}, the owner {session.owner} and their HOMEs and keys "
                  f"in {private_root}", flush=True)
            session.log.close()
        else:
            failure = session.delete_accounts()
            session.log.close()
            shutil.rmtree(private_root, ignore_errors=True)
            if failure:
                raise failure


@pytest.fixture(scope="module")
def lab(session, request):
    """The lab every case uses: the owner's first key added, then brought up once as the owner and
    destroyed at the end, even when a case fails, unless `--keep-failed` is given and something
    failed."""
    keep_failed = request.config.getoption("keep_failed")
    lab = Lab(session)
    keep = False
    try:
        try:
            _, public = session.new_key("first")
            home_key = session.owner_home / ".ssh" / "id_ed25519"
            home_key.parent.mkdir(mode=0o700, exist_ok=True)
            shutil.copy(session.keys["first"], home_key)
            home_key.with_name("id_ed25519.pub").write_text(public + "\n")
            session.add_key("first")
            lab.up("first")
        except Exception:
            keep = keep_failed
            raise
        yield lab
        keep = keep_failed and request.session.testsfailed > 0
    finally:
        if keep:
            print(f"keeping failed lab {lab.name} ({lab.lab_id}) in {lab.directory}", flush=True)
        else:
            failure = lab.destroy()
            if failure:
                raise failure


# -- cases ---------------------------------------------------------------------------------------


def test_owner_login_001_an_owner_with_no_ssh_keys_is_refused_at_up(session):
    require(not session.owner_keys("owner's keys before up"), "the owner already has SSH keys")
    folder = session.run_root / "refused-up"
    folder.mkdir()
    manifest = tomllib.loads((LAB / "manifest.toml").read_text())
    (folder / "manifest.toml").write_text(render_manifest(manifest, f"{manifest['name']}-refused"))
    outcome = session.as_owner("sherpa up (no SSH key)", ["up"], folder, UP_TIMEOUT, check=False)
    require(outcome.code not in (0, None), f"sherpa up with no SSH key did not fail (exit {outcome.code})")
    require(NO_SSH_KEY in outcome.output, f"sherpa up was not refused for the missing key: {outcome.output.strip()}")
    require(not (folder / ".sherpa" / "lab-info.toml").exists(), "the refused up wrote .sherpa/lab-info.toml")


def test_owner_login_002_the_owner_logs_in_to_a_vm_by_key_with_a_home_bash_and_passwordless_sudo(lab):
    lab.key_login(VM)


def test_owner_login_003_the_owner_logs_in_to_sr_linux_by_key_with_the_superuser_role(lab):
    lab.key_login(CONTAINER)


def test_owner_login_004_the_owner_logs_in_to_each_node_with_their_device_password(lab):
    config = lab.session.run_root / "password_ssh_config"
    config.write_text(password_ssh_config((lab.directory / ".sherpa" / "ssh" / "config").read_text(),
                                          [lab.alias(VM), lab.alias(CONTAINER)]))
    for node in (VM, CONTAINER):
        label = f"ssh {node} (device password)"
        code, answered, text = lab.password_login(label, node, config)
        require(code == 0, f"{label} failed (exit {code}): {text.strip()}")
        require(answered >= 1, f"{label} was never asked for a password; see {lab.log.name}")
        problems = probe_problems(node, lab.owner, text)
        require(not problems, f"{label}: the session lacks {', '.join(problems)}; see {lab.log.name}")


def test_owner_login_005_a_key_not_on_the_owners_account_is_refused_at_the_node(lab):
    key = lab.session.private / "admin-key" / "id_ed25519"
    make_key(lab.log, key, lab.session.run_root)
    config = key.parent / "ssh_config"
    config.write_text(admin_ssh_config(key))
    for login in (lab.session.admin_user, lab.owner):
        for node in (VM, CONTAINER):
            marker = f"reached-{uuid.uuid4().hex[:8]}"
            label = f"ssh {login}@{node} with the admin's key"
            outcome = lab.session.command(label, direct_ssh_arguments(config, key, lab.host(node), login,
                                                                      f"echo {marker}"),
                                          lab.directory, REFUSAL_TIMEOUT, check=False, env=lab.session.admin_env)
            require(marker not in outcome.stdout and outcome.code != 0,
                    f"{label} logged in: exit {outcome.code}, {outcome.output.strip()}")
            why = refused_at_node(outcome.stderr, lab.host(node))
            require(why is None, f"{label}: {why}; see {lab.log.name}")


def test_ssh_config_hosts_001_each_host_logs_in_as_the_owner_and_names_no_identity_file(lab):
    hosts = parse_ssh_hosts((lab.directory / ".sherpa" / "ssh" / "config").read_text())
    for node in (VM, CONTAINER):
        host = find_host(hosts, lab.alias(node))
        require(host.user == lab.owner, f"the lab's SSH config logs in to {node} as {host.user}, not {lab.owner}")
    for host in hosts:
        require(not host.identity_files,
                f"the lab's SSH config names an identity file for {host.aliases[0]}: {host.identity_files}")


def test_access_control_004_an_admin_can_inspect_a_lab_they_do_not_own(lab):
    rows = {row["Device"]: row for row in parse_devices_table(lab.admin("sherpa inspect", ["inspect"]).stdout)}
    for node in (VM, CONTAINER):
        require(lab.alias(node) in rows, f"the admin's inspect does not list {node}: {sorted(rows)}")


def test_access_control_005_an_admin_is_refused_the_download_of_a_lab_they_do_not_own(lab):
    outcome = lab.admin("sherpa download", ["download", lab.lab_id], cwd=lab.download_directory, check=False)
    require(outcome.code != 0, "the admin downloaded a lab they do not own")
    require(OWNER_ONLY_SERVER.format(lab=lab.lab_id) in outcome.output,
            f"sherpa download was not refused for the owner only: {outcome.output.strip()}")
    files = [str(path) for path in (lab.download_directory / ".sherpa").rglob("*") if path.is_file()] \
        if (lab.download_directory / ".sherpa").exists() else []
    require(not files, f"the refused download wrote files: {files}")


def test_access_control_006_an_admin_is_refused_sherpa_ssh_to_the_nodes_of_a_lab_they_do_not_own(lab):
    for node in (VM, CONTAINER):
        command = f"sherpa ssh {node}"
        outcome = lab.admin(command, ["ssh", node], REFUSAL_TIMEOUT, check=False)
        require(outcome.code not in (0, None), f"{command} did not fail for the admin")
        require(OWNER_ONLY_CONNECT.format(lab=lab.lab_id) in outcome.output,
                f"{command} was not refused for the owner only: {outcome.output.strip()}")
        require("Connected to" not in outcome.output, f"{command} connected for the admin")


def test_access_control_007_an_admin_is_refused_the_console_of_a_lab_they_do_not_own(lab):
    command = f"sherpa console {VM}"
    outcome = lab.admin(command, ["console", VM], REFUSAL_TIMEOUT, check=False)
    require(outcome.code not in (0, None), f"{command} did not fail for the admin")
    require(OWNER_ONLY_CONSOLE.format(lab=lab.lab_id) in outcome.output,
            f"{command} was not refused for the owner only: {outcome.output.strip()}")
    require("Connected to" not in outcome.output, f"{command} connected for the admin")


def test_account_keys_001_an_admin_cannot_add_a_key_to_another_users_account(lab):
    session = lab.session
    before = session.owner_keys("owner's keys before")
    home = session.admin_home / ".sherpa"
    config_file = home / "config" / "sherpa.toml"
    config = tomllib.loads(config_file.read_text()) if config_file.exists() else {}
    base_url = server_base_url(config, os.environ.get(SERVER_URL_ENV))
    context = tls_context(config, base_url, home / "trusted_certs")
    token = (home / "token").read_text().strip()
    add_secret(token)
    _, public_key = session.new_key("admin-offered")
    url = f"{base_url}/api/v1/admin/users/{session.owner}/ssh-keys"
    reply = rest_post(url, token, {"username": session.owner, "ssh_key": public_key, "token": ""}, context)
    lab.log.write(mask(f"=== POST {url}\n--- status {reply.status}\n--- body\n{reply.body}\n"))
    lab.log.flush()
    require(reply.status == 403, f"adding a key to {session.owner}'s account as the admin gave "
            f"HTTP {reply.status}, not 403: {reply.body.strip()}")
    require(OWN_ACCOUNT_ONLY.format(owner=session.owner) in reply.body,
            f"the refusal does not say the account's own user only may: {reply.body.strip()}")
    after = session.owner_keys("owner's keys after")
    require(after == before, f"the owner's keys changed: {before} -> {after}")


def test_lab_down_resume_002_resume_waits_for_a_vm_to_be_ready_and_streams_its_progress(lab):
    resume = lab.down_and_resume()
    output = resume.stdout
    progress = f"Node {VM} - Ready"
    result = f"{VM}: Started and ready"
    require(progress in output, f"sherpa resume streamed no {progress!r}: {output.strip()}")
    require(result in output, f"sherpa resume did not report {result!r}: {output.strip()}")
    require(output.index(progress) < output.index(result),
            "sherpa resume printed the VM's readiness only with its result, not while it waited")
    first = lab.ssh(f"{VM} login right after resume", VM, READY_COMMANDS[VM], lab.node_keys[VM])
    require(first.code == 0, f"{VM} refused the owner's login right after resume returned: {first.output.strip()}")


def test_lab_down_resume_003_a_resumed_vm_keeps_its_host_keys_and_the_owner_can_still_log_in(lab):
    before = lab.host_keys("host keys before down", VM)
    lab.down_and_resume()
    after = lab.host_keys("host keys after resume", VM)
    require(after == before, f"{VM}'s host keys changed across down and resume: {sorted(before)} -> {sorted(after)}")
    # The lab's SSH config checks the host key strictly against the lab's known_hosts file.
    login = lab.ssh(f"{VM} login after resume", VM, READY_COMMANDS[VM], lab.node_keys[VM])
    require(login.code == 0, f"the owner could not log in to {VM} after resume: {login.output.strip()}")


def test_owner_login_006_a_key_rotated_in_the_profile_is_not_pushed_to_a_running_labs_nodes_on_resume(lab):
    session = lab.session
    old = session.profile_key_names()
    require(old, "the owner has none of the run's keys on their account")
    name = f"rotated-{len(session.keys)}"
    session.new_key(name)
    session.add_key(name)
    for key in old:
        session.remove_key(key)
    require(session.profile_key_names() == [name], f"the owner's account does not hold only the key {name}")
    lab.down_and_resume()
    for node in (VM, CONTAINER):
        built = lab.ssh(f"{node} login with the key it was built with", node, READY_COMMANDS[node],
                        lab.node_keys[node])
        require(built.code == 0, f"{node} refused the key it was built with after resume: {built.output.strip()}")
        rotated = lab.ssh(f"{node} login with the rotated key", node, READY_COMMANDS[node], session.keys[name])
        require(rotated.code != 0, f"{node} took the key added to the profile after it was built")
        require(PERMISSION_DENIED in rotated.output, f"the rotated key was not refused: {rotated.output.strip()}")


def test_owner_login_007_an_owner_with_no_ssh_keys_is_refused_at_redeploy(lab):
    session = lab.session
    keys = session.profile_key_names()
    require(keys, "the owner has none of the run's keys on their account")
    for key in keys:
        session.remove_key(key)
    try:
        require(not session.owner_keys("owner's keys before redeploy"), "the owner still has SSH keys")
        outcome = lab.redeploy(f"sherpa redeploy {VM} (no SSH key)", VM, check=False)
        require(outcome.code not in (0, None), f"redeploy with no SSH key did not fail (exit {outcome.code})")
        require(NO_SSH_KEY in outcome.output, f"redeploy was not refused for the missing key: {outcome.output.strip()}")
    finally:
        for key in keys:
            session.add_key(key)


def test_lab_redeploy_001_a_redeployed_vm_comes_back_and_the_owner_can_log_in(lab):
    session = lab.session
    keys = session.profile_key_names()
    require(keys, "the owner has none of the run's keys on their account")
    before = lab.boot_id("boot id before redeploy")
    lab.redeploy(f"sherpa redeploy {VM}", VM)
    lab.read_lab_files()
    lab.node_keys[VM] = session.keys[keys[0]]
    lab.wait_for_login(VM)
    require(lab.boot_id("boot id after redeploy") != before, f"{VM} was not rebuilt: its boot id is unchanged")


def test_lab_redeploy_002_a_refused_redeploy_leaves_the_running_node_untouched(lab):
    session = lab.session
    keys = session.profile_key_names()
    require(keys, "the owner has none of the run's keys on their account")
    before = lab.boot_id("boot id before the refused redeploy")
    for key in keys:
        session.remove_key(key)
    try:
        outcome = lab.redeploy(f"sherpa redeploy {VM} (refused)", VM, check=False)
        require(outcome.code not in (0, None), f"redeploy with no SSH key did not fail (exit {outcome.code})")
    finally:
        for key in keys:
            session.add_key(key)
    require(lab.boot_id("boot id after the refused redeploy") == before,
            f"{VM} restarted or was rebuilt by a refused redeploy")


def test_access_control_008_an_admin_can_destroy_a_lab_they_do_not_own(lab):
    lab.admin("sherpa destroy", ["destroy", "--yes"], UP_TIMEOUT)
    lab.admin_destroyed = True
    inspect = lab.sherpa("lab gone (owner)", ["inspect"], check=False)
    require(inspect.code != 0, f"lab {lab.lab_id} still exists after the admin's destroy")


def test_stale_includes_001_clean_removes_the_line_of_a_lab_destroyed_elsewhere(lab):
    require(lab.admin_destroyed, "the lab was not destroyed by the admin, so its line is not stale")
    before = lab.sherpa("sherpa ssh-config inspect (before clean)", ["ssh-config", "inspect"])
    require(str(lab.directory) in before.output,
            f"~/.ssh/sherpa_lab_hosts does not list {lab.directory} before the clean")
    lab.sherpa("sherpa ssh-config clean", ["ssh-config", "clean"])
    after = lab.sherpa("sherpa ssh-config inspect", ["ssh-config", "inspect"])
    require(str(lab.directory) not in after.output, f"~/.ssh/sherpa_lab_hosts still lists {lab.directory}")


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
    return pytest.main(["-v", "--tb=short", "-p", "no:cacheprovider", __file__, *pytest_args])


class SelfTest(unittest.TestCase):
    # The runner's own checks, run with --self-test; pytest must not collect them as lab cases.
    __test__ = False

    def manifest(self):
        return tomllib.loads((LAB / "manifest.toml").read_text())

    def test_user_info_is_read_from_the_info_output(self):
        ready = "\nUser Information:\n  Username: alice\n  Admin: No\n  SSH Keys: 1 key(s)\n  Device password: set\n"
        self.assertEqual(parse_user_info(ready), UserInfo("alice", False, True))
        self.assertEqual(parse_user_info(ready.replace("Admin: No", "Admin: Yes").replace(": set", ": not set")),
                         UserInfo("alice", True, False))
        with self.assertRaises(TestFailure):
            parse_user_info("  Device password: set\n")

    def test_a_childs_environment_has_its_own_home_and_none_of_the_runners_secrets(self):
        with mock.patch.dict(os.environ, {LOGIN_PASSWORD_ENV: "pw", "SSH_AUTH_SOCK": "/s", "PATH": "/bin"}):
            env = child_env(Path("/h"))
        self.assertEqual(env["HOME"], "/h")
        self.assertEqual(env["PATH"], "/bin")
        self.assertNotIn(LOGIN_PASSWORD_ENV, env)
        self.assertNotIn("SSH_AUTH_SOCK", env)

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
        keyed = parse_ssh_hosts("Host a\n    HostName a\n    User alice\n    IdentityFile ~/.ssh/k\n")
        self.assertEqual(keyed[0].identity_files, ("~/.ssh/k",))
        self.assertEqual(hosts[0].identity_files, ())

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

    def test_keys_are_compared_by_their_body(self):
        self.assertEqual(key_blob("ssh-ed25519 AAAA comment here"), "AAAA")
        scan = "# 10.0.0.1:22 SSH-2.0-OpenSSH\n10.0.0.1 ssh-ed25519 AAAA\n10.0.0.1 ssh-rsa BBBB\n"
        self.assertEqual(scanned_host_keys(scan), {("ssh-ed25519", "AAAA"), ("ssh-rsa", "BBBB")})

    def test_password_prompts_name_the_host_that_asks(self):
        text = ("alice@172.31.0.21's password: \nPermission denied, please try again.\n"
                "(alice@172.31.0.22) Password: ")
        prompts = password_prompts(text)
        self.assertEqual([(prompt.user, prompt.host) for prompt in prompts],
                         [("alice", "172.31.0.21"), ("alice", "172.31.0.22")])
        self.assertEqual(password_prompts("Connected to ubu01.abcd1234. Escape: ~. at the start of a line."), [])

    def test_the_probe_output_must_show_the_owners_account_and_admin_rights(self):
        linux = ("alice@ubu01:~$ printf 'who:%s:home:%s:shell:%s:\\n'\nwho:alice:home:/home/alice:shell:/bin/bash:\n"
                 "home-ok-42\nsudo-ok-42\n")
        self.assertEqual(probe_problems(VM, "alice", linux), [])
        self.assertEqual(len(probe_problems(VM, "alice", linux.replace("sudo-ok-42", ""))), 1)
        self.assertEqual(len(probe_problems(VM, "alice", linux.replace("/bin/bash", "/bin/sh"))), 1)
        self.assertEqual(len(probe_problems(VM, "alice", linux.replace("/home/alice", "/"))), 1)
        self.assertEqual(len(probe_problems(VM, "alice", linux.replace("who:alice", "who:bob"))), 1)
        # Typing the command shows its text, which holds none of the values or results.
        self.assertEqual(len(probe_problems(VM, "alice", LINUX_COMMAND)), 5)
        srlinux = ('    superuser false\n    last-successful-login "2026-11-17T07:29:15.326Z (8 seconds ago)"\n'
                   '    role [\n        \x1b[0msrl-admin\n    ]\n')
        self.assertEqual(probe_problems(CONTAINER, "alice", srlinux), [])
        self.assertEqual(len(probe_problems(CONTAINER, "alice", srlinux.replace("srl-admin", "srl-viewer"))), 1)
        self.assertEqual(len(probe_problems(CONTAINER, "alice", srlinux.replace("8 seconds ago", "never"))), 1)
        self.assertEqual(probe_problems(CONTAINER, "alice", srlinux.replace("8 seconds ago", "a second ago")), [])
        self.assertEqual(probe_command(CONTAINER, "alice"),
                         "info from state /system aaa authentication user alice")

    def test_the_password_config_takes_keys_away_from_the_nodes_only(self):
        text = password_ssh_config("Host x.1\n    User alice\n", ["a.1", "b.1"])
        self.assertTrue(text.startswith("Host a.1\n    PubkeyAuthentication no\n"))
        self.assertEqual(text.count("PubkeyAuthentication no"), 2)
        self.assertEqual(text.count("Host "), 3)
        self.assertTrue(text.endswith("Host x.1\n    User alice\n"))
        self.assertNotIn("Host *", text)

    def test_the_admin_ssh_uses_only_the_admins_key_and_is_refused_at_the_node(self):
        host = SshHost(("172.31.0.21",), "172.31.0.21", 22, "alice")
        arguments = direct_ssh_arguments(Path("/run/ssh_config"), Path("/run/key"), host, "dana", "true")
        self.assertEqual(arguments[:3], ["ssh", "-v", "-F"])
        for option in ("BatchMode=yes", "IdentitiesOnly=yes"):
            self.assertIn(option, arguments)
        self.assertEqual(arguments[arguments.index("-i") + 1], "/run/key")
        self.assertEqual(arguments[arguments.index("-l") + 1], "dana")
        self.assertEqual(arguments[-2:], ["172.31.0.21", "true"])
        config = admin_ssh_config(Path("/run/key"))
        for line in ("IdentityFile /run/key", "IdentitiesOnly yes", "IdentityAgent none", "BatchMode yes"):
            self.assertIn(line, config)
        at_node = ("debug1: Connecting to 172.31.0.21 [172.31.0.21] port 22.\n"
                   "debug1: Server host key: ssh-ed25519 SHA256:x\n"
                   "dana@172.31.0.21: Permission denied (publickey,password).\n")
        self.assertIsNone(refused_at_node(at_node, host))
        jumped = at_node.replace("172.31.0.21 [172.31.0.21]", "172.31.1.21 [172.31.1.21]")
        self.assertIsNotNone(refused_at_node(jumped, host))
        self.assertIsNotNone(refused_at_node(at_node.replace("Server host key", "Connection closed"), host))
        self.assertIsNotNone(refused_at_node(at_node.replace("Permission denied", "Timeout"), host))

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
                    terminal.kill()
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


if __name__ == "__main__":
    sys.exit(main())
