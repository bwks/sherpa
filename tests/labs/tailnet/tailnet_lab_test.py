#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.15"
# dependencies = ["pytest==9.2.0"]
# ///
"""End-to-end test of a lab's Tailscale gateway on a real tailnet (Python 3.15+, pytest).

Brings up a lab with `[tailscale]` enabled, of an Alpine VM and an Alpine container, and checks
the real-tailnet cases of tests/specs/tailnet.md from a probe: a `tailscale/tailscale` container
on this server, in a Docker network of its own, enrolled with the same key and accepting routes,
which reaches the lab's management network only through the tailnet. Devices joining and leaving
the tailnet are read from the probe's peer list, matched by the gateway's host name and node ID.
A second lab without `[tailscale]` and a third with it show what one gateway does not open and
does not take, and the third is removed with `sherpa server clean`. Always destroys its labs, logs
the probe out and removes it, then checks with a second probe that no device of the run is left.

Runs on the Sherpa server: it uses Docker for the probe and to read the gateways, `ssh` to read a
node, and `sudo -n` to read the server's files for the key. The key is read from
`SHERPA_TEST_TAILSCALE_AUTH_KEY`; without it the run stops before anything is made. See README.md.

A pytest module: the `run` fixture brings the probe and the first lab up once and removes
everything at the end, and the cases are the `test_tailnet_*` functions taking it, in case order.
Run as a script, it runs pytest on itself; `--self-test` runs the runner's own unit tests instead.
"""

import argparse
import http.client
import ipaddress
import json
import os
from pathlib import Path
import re
import secrets
import shutil
import signal
import ssl
import subprocess
import sys
import tempfile
import time
import tomllib
from typing import NamedTuple
import unittest
from unittest import mock
import uuid

import pytest

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))

from labkit import (
    INFO_ADMIN,
    TestFailure,
    add_secret,
    mask,
    require,
    require_device_password,
    run_logged,
    stop_on_sigterm,
    value_toml,
)


LAB = Path(__file__).resolve().parent
ROOT = LAB.parent.parent.parent
RUNS = ROOT / ".tmp" / "lab-tests"

KEY_ENV = "SHERPA_TEST_TAILSCALE_AUTH_KEY"
ENV_ADMIN_USER = "SHERPA_LAB_TEST_ADMIN_USER"
ENV_ADMIN_PASSWORD = "SHERPA_LAB_TEST_ADMIN_PASSWORD"
LOGIN_USER_ENV = "SHERPA_USER_NAME"
LOGIN_PASSWORD_ENV = "SHERPA_USER_PASSWORD"
# Case 019 restarts Docker on this host, so it runs only when this is set, by `--restart-docker`.
RESTART_DOCKER_ENV = "SHERPA_TEST_RESTART_DOCKER"

VM = "vm01"
CONTAINER = "ctr01"
VM_NODE = {"name": VM, "model": "alpine_linux"}
CONTAINER_NODE = {"name": CONTAINER, "model": "generic", "kind": "container", "image": "alpine",
                  "version": "3.22"}
IMAGES = (("alpine_linux", None, None), ("generic", "container", "3.22"))

# The probe runs the gateway's image, the default of the server's `[tailscale] image`.
PROBE_IMAGE = "tailscale/tailscale:v1.102.3"
SOCKET = "/run/tailscale/tailscaled.sock"
PROBE_PREFIX = "sherpa-test-tnprobe"
GATEWAY_PREFIX = "sherpa-tailnet"
VOLUME_PREFIX = "sherpa-tailnet-state"
MANAGEMENT_PREFIX = "sherpa-management"
GATEWAY_OFFSET = 3
# What the probe may not reach but through the tailnet: every private range, and the server.
PRIVATE_IPV4 = ("10.0.0.0/8", "172.16.0.0/12", "192.168.0.0/16")
PRIVATE_IPV6 = ("fc00::/7",)
PROBE_RULES = ('own="$1"; shift; iptables-nft -A OUTPUT -d "$own" -j ACCEPT; '
               'for range in "$@"; do case $range in *:*) tool=ip6tables-nft;; *) tool=iptables-nft;; esac; '
               '$tool -A OUTPUT -d "$range" ! -o tailscale0 -j REJECT; done')
ENROLL = ('umask 077; trap "rm -f /run/enrollment-key" EXIT; cat > /run/enrollment-key; '
          'tailscale --socket="$1" up --auth-key=file:/run/enrollment-key --hostname="$2" --accept-dns=false '
          '--timeout=60s "$3"')
BANNER = 'nc -w 10 "$1" 22 </dev/null | head -c 64'
HOLD = 'sleep "$2" | nc -w "$3" "$1" 22 >/dev/null'
SSH_BANNER = "SSH-2.0-"

SHERPA_DIR = Path("/opt/sherpa")
DEFAULT_WS_PORT = 3030
AUTH_COOKIE = "sherpa_auth"
WEB_REFUSAL = "run sherpa resume in the lab folder"
LAB_NOT_FOUND = "Lab not found"

COMMAND_TIMEOUT = 180
UP_TIMEOUT = 1200
WEB_TIMEOUT = 900
PROBE_READY_TIMEOUT = 30
# How long the tailnet may take to approve routes, hand them to the probe, or drop a device.
PROPAGATION_TIMEOUT = 180
DOCKER_READY_TIMEOUT = 120
POLL_SECONDS = 3
HOLD_SECONDS = 10
CONNECTED = "Connected"
APPROVED = "Approved"


class Peer(NamedTuple):
    """A device in the probe's peer list."""

    hostname: str
    node_id: str
    online: bool
    routes: tuple


class Device(NamedTuple):
    """A tailnet device the run made, by host name and node ID."""

    hostname: str
    node_id: str


class NodeRow(NamedTuple):
    """A node's row in `sherpa inspect`."""

    state: str
    ipv4: str
    ipv6: str


class Account(NamedTuple):
    """A Sherpa account other than the labs' owner, logged in to a HOME of its own."""

    username: str
    env: dict


def render_manifest(name, nodes, tailscale):
    """The manifest of one lab of the run, with `[tailscale]` naming the key's variable when enabled."""
    lines = [f"name = {value_toml(name)}", ""]
    if tailscale:
        lines += ["[tailscale]", "enabled = true", f"auth_key_env = {value_toml(KEY_ENV)}", ""]
    for node in nodes:
        lines += ["[[nodes]]", *(f"{key} = {value_toml(item)}" for key, item in node.items()), ""]
    return "\n".join(lines)


def parse_peers(status_json):
    """The peers in `tailscale status --json` output."""
    status = json.loads(status_json)
    return [Peer(peer.get("HostName", ""), peer.get("ID", ""), bool(peer.get("Online")),
                 tuple(peer.get("PrimaryRoutes") or ()))
            for peer in (status.get("Peer") or {}).values()]


def parse_self(status_json):
    """The device of a daemon's own `tailscale status --json` output."""
    own = json.loads(status_json).get("Self") or {}
    require(own.get("ID") and own.get("HostName"), "the daemon reports no device of its own")
    return Device(own["HostName"], own["ID"])


def table_cells(line):
    """The cells of one row of a CLI table, or None for a border line."""
    if not line.startswith("│"):
        return None
    return [cell.strip() for cell in line.strip().strip("│").split("│")]


def parse_tailscale_table(output):
    """The rows of the Tailscale table in a command's output, with wrapped values joined."""
    rows = {}
    inside = False
    current = None
    for line in output.splitlines():
        cells = table_cells(line)
        if cells == ["Tailscale", "Details"]:
            inside = True
            continue
        if inside and line.startswith("└"):
            break
        if not inside or cells is None or len(cells) != 2:
            continue
        if cells[0]:
            current = cells[0]
            rows[current] = cells[1]
        elif current:
            rows[current] = f"{rows[current]} {cells[1]}".strip()
    return rows


def parse_nodes(output, lab_id):
    """Each node's state and management addresses in `sherpa inspect` output."""
    nodes = {}
    for line in output.splitlines():
        cells = table_cells(line)
        if cells and len(cells) == 9 and cells[0].endswith(f".{lab_id}"):
            nodes[cells[0].removesuffix(f".{lab_id}")] = NodeRow(cells[3], cells[4], cells[5])
    return nodes


def management_id(output):
    """The lab ID in the management network `sherpa up` names."""
    found = re.search(rf"{MANAGEMENT_PREFIX}-([0-9a-f]+)", output)
    require(found, "sherpa up did not name the lab's management network")
    return found.group(1)


def tailnet_routes(route_listing):
    """The prefixes of the routes through `tailscale0` in `ip route show table all` output."""
    prefixes = set()
    for line in route_listing.splitlines():
        fields = line.split()
        if fields and "dev" in fields and fields[fields.index("dev") + 1:][:1] == ["tailscale0"]:
            first = fields[1] if fields[0] in ("local", "broadcast", "anycast", "unicast") else fields[0]
            try:
                prefixes.add(ipaddress.ip_network(first, strict=False))
            except ValueError:
                continue
    return prefixes


def der_element(data, start):
    """The tag, content start and end of the DER element at `start`."""
    length = data[start + 1]
    content = start + 2
    if length & 0x80:
        count = length & 0x7F
        length = int.from_bytes(data[content:content + count])
        content += count
    return data[start], content, content + length


def public_key(certificate):
    """The DER of a certificate's subject public key info, which the CLI pins."""
    _, tbs, _ = der_element(certificate, 0)
    _, offset, _ = der_element(certificate, tbs)
    tag, _, end = der_element(certificate, offset)
    if tag == 0xA0:
        offset = end
    for _ in range(5):
        offset = der_element(certificate, offset)[2]
    return certificate[offset:der_element(certificate, offset)[2]]


def places_holding(secret, places):
    """The names of the `places` (name to bytes) whose contents hold `secret`."""
    needle = secret.encode()
    return [name for name, data in places.items() if needle in data]


def wait_for(condition, timeout, what):
    """Poll `condition` until it returns a true value, which is returned, or fail after `timeout` seconds."""
    deadline = time.monotonic() + timeout
    while True:
        value = condition()
        if value:
            return value
        require(time.monotonic() < deadline, f"{what} not seen within {timeout}s")
        time.sleep(POLL_SECONDS)


def server_address():
    """The server's address, its web UI's port and the trusted certificate's path, from the CLI's config."""
    config = tomllib.loads((Path.home() / ".sherpa" / "config" / "sherpa.toml").read_text())
    return config["server_ipv4"], config.get("ws_port", DEFAULT_WS_PORT), config["server_connection"].get("ca_cert_path")


def without_key(env=None):
    """The environment with the key's variable unset."""
    return {key: value for key, value in (env or os.environ).items() if key != KEY_ENV}


# -- Docker and the probe ------------------------------------------------------------------------


class Docker:
    """Docker on this server, through its CLI, logged."""

    def __init__(self, log, root):
        self.log = log
        self.root = root

    def run(self, label, arguments, timeout=COMMAND_TIMEOUT, stdin=None):
        """Run a Docker command and log it."""
        return run_logged(self.log, label, ["docker", *arguments], self.root, timeout, stdin=stdin)

    def names(self):
        """The names of every container, volume and network."""
        listings = (["ps", "-a", "--format", "{{.Names}}"], ["volume", "ls", "-q"],
                    ["network", "ls", "--format", "{{.Name}}"])
        found = set()
        for kind, arguments in zip(("container", "volume", "network"), listings):
            outcome = self.run(f"docker {kind}s", arguments)
            require(outcome.code == 0, f"could not list Docker {kind}s; see {self.log.name}")
            found |= {f"{kind} {name}" for name in outcome.stdout.split()}
        return found

    def running(self, container):
        """Whether `container` runs; None when it does not exist."""
        outcome = self.run(f"is {container} running", ["inspect", "-f", "{{.State.Running}}", container])
        if outcome.code != 0:
            return None
        return outcome.stdout.strip() == "true"

    def volume_exists(self, volume):
        """Whether the Docker volume `volume` exists."""
        return self.run(f"volume {volume}", ["volume", "inspect", volume]).code == 0

    def exec(self, label, container, arguments, timeout=COMMAND_TIMEOUT, stdin=None):
        """Run a command in `container`, with `stdin` on its standard input when given."""
        interactive = ["-i"] if stdin is not None else []
        return self.run(label, ["exec", *interactive, container, *arguments], timeout, stdin)

    def read(self, arguments):
        """The raw output of a Docker command, not logged, for searching for the key."""
        result = subprocess.run(["docker", *arguments], capture_output=True, timeout=COMMAND_TIMEOUT, check=False)
        require(result.returncode == 0, f"docker {' '.join(arguments[:2])} failed")
        return result.stdout

    def wait_ready(self):
        """Wait until the Docker daemon answers."""
        wait_for(lambda: self.run("docker info", ["info", "--format", "{{.ServerVersion}}"]).code == 0,
                 DOCKER_READY_TIMEOUT, "the Docker daemon answering")


class Probe:
    """A Tailscale device in a Docker network of its own that reaches the run's labs only through
    the tailnet."""

    def __init__(self, docker, name, server_ipv4, accept_routes=True):
        self.docker = docker
        self.name = name
        self.server_ipv4 = server_ipv4
        self.accept_routes = accept_routes
        self.device = None
        self.made = False

    def start(self, key):
        """Create the network and the container, block what it may reach directly, and enroll it."""
        self.made = True
        created = self.docker.run(f"create {self.name} network", ["network", "create", self.name])
        require(created.code == 0, f"could not create the probe's network; see {self.docker.log.name}")
        started = self.docker.run(f"start {self.name}", [
            "run", "-d", "--name", self.name, "--hostname", self.name, "--network", self.name,
            "--cap-add", "NET_ADMIN", "--cap-add", "NET_RAW", "--device", "/dev/net/tun",
            "--tmpfs", "/run:rw,size=16m,mode=0700", "--entrypoint", "tailscaled", PROBE_IMAGE,
            "--state=mem:", f"--socket={SOCKET}"])
        require(started.code == 0, f"could not start the probe; see {self.docker.log.name}")
        wait_for(lambda: self.docker.exec("probe socket", self.name, ["test", "-S", SOCKET]).code == 0,
                 PROBE_READY_TIMEOUT, "the probe's Tailscale daemon")
        subnet = self.docker.run("probe subnet", ["network", "inspect", "-f", "{{range .IPAM.Config}}{{.Subnet}}{{end}}",
                                                  self.name]).stdout.strip()
        blocked = [*PRIVATE_IPV4, *PRIVATE_IPV6, self.server_ipv4]
        rules = self.docker.exec("probe firewall", self.name, ["sh", "-ec", PROBE_RULES, "rules", subnet, *blocked])
        require(rules.code == 0, f"could not keep the probe off the server's networks; see {self.docker.log.name}")
        routes = "--accept-routes" if self.accept_routes else "--accept-routes=false"
        enrolled = self.docker.exec("probe enroll", self.name, ["sh", "-c", ENROLL, "enroll", SOCKET, self.name, routes],
                                    timeout=COMMAND_TIMEOUT, stdin=key)
        require(enrolled.code == 0, f"the probe could not join the tailnet; see {self.docker.log.name}")
        status = self.docker.exec("probe self", self.name, ["tailscale", f"--socket={SOCKET}", "status", "--json",
                                                             "--peers=false"])
        self.device = parse_self(status.stdout)

    def peers(self):
        """The probe's peer list."""
        status = self.docker.exec("probe peers", self.name, ["tailscale", f"--socket={SOCKET}", "status", "--json"])
        require(status.code == 0, f"could not read the probe's peers; see {self.docker.log.name}")
        return parse_peers(status.stdout)

    def has(self, device):
        """Whether `device` is in the peer list, by host name and node ID."""
        return any(peer.hostname == device.hostname and peer.node_id == device.node_id for peer in self.peers())

    def gone(self, device):
        """Whether the node ID of `device` has left the peer list."""
        return all(peer.node_id != device.node_id for peer in self.peers())

    def wait_routes(self, device, routes):
        """Wait until `device` serves `routes` to the probe."""
        wanted = {str(route) for route in routes}
        wait_for(lambda: any(peer.node_id == device.node_id and wanted <= set(peer.routes) for peer in self.peers()),
                 PROPAGATION_TIMEOUT, f"{device.hostname} serving {sorted(wanted)} to the probe")

    def wait_gone(self, device):
        """Wait until the node ID of `device` has left the peer list."""
        wait_for(lambda: self.gone(device), PROPAGATION_TIMEOUT, f"{device.hostname} ({device.node_id}) leaving the tailnet")

    def banner(self, address):
        """Whether `address` answers SSH: the first bytes of its SSH banner."""
        outcome = self.docker.exec(f"SSH banner of {address}", self.name,
                                   ["timeout", "15", "sh", "-c", BANNER, "banner", str(address)], timeout=30)
        return outcome.stdout.startswith(SSH_BANNER)

    def ping(self, address):
        """Whether `address` answers the probe's ping."""
        outcome = self.docker.exec(f"ping {address}", self.name, ["ping", "-c", "2", "-W", "3", str(address)], timeout=30)
        return outcome.code == 0

    def wait_banner(self, address):
        """Wait until `address` answers SSH from the probe."""
        wait_for(lambda: self.banner(address), PROPAGATION_TIMEOUT, f"SSH from {address} to the probe")

    def wait_ping(self, address):
        """Wait until `address` answers the probe's ping."""
        wait_for(lambda: self.ping(address), PROPAGATION_TIMEOUT, f"{address} answering the probe's ping")

    def hold(self, address, seconds):
        """Hold a connection to `address`'s SSH port open for `seconds`; returns the process."""
        return subprocess.Popen(["docker", "exec", self.name, "sh", "-c", HOLD, "hold", str(address), str(seconds),
                                 str(seconds + 5)], stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL,
                                stderr=subprocess.DEVNULL)

    def address(self):
        """The probe's tailnet IPv4 address."""
        outcome = self.docker.exec("probe address", self.name, ["tailscale", f"--socket={SOCKET}", "ip", "-4"])
        require(outcome.code == 0 and outcome.stdout.strip(), "the probe has no tailnet IPv4 address")
        return outcome.stdout.split()[0]

    def stop(self):
        """Log the probe out and remove it and its network. Returns a failure, or None."""
        if not self.made:
            return None
        failure = None
        if self.docker.running(self.name):
            logout = self.docker.exec("probe logout", self.name, ["tailscale", f"--socket={SOCKET}", "logout"], timeout=60)
            if logout.code != 0:
                failure = TestFailure(f"the probe {self.name} could not log out; see {self.docker.log.name}")
        for label, arguments in (("remove probe", ["rm", "-f", self.name]), ("remove probe network", ["network", "rm", self.name])):
            if self.docker.run(label, arguments).code != 0 and failure is None:
                failure = TestFailure(f"{label} failed; see {self.docker.log.name}")
        self.made = False
        return failure


# -- the labs ------------------------------------------------------------------------------------


class Lab:
    """One lab of the run, in a folder of its own under a unique name."""

    def __init__(self, run, role, nodes, tailscale):
        self.run = run
        self.name = f"tn{role}-{run.root.name[-6:]}"
        self.directory = run.root / role
        self.nodes = nodes
        self.tailscale = tailscale
        self.info = None
        self.up_outcome = None
        self.made = False
        self.removed = False

    @property
    def lab_id(self):
        """The lab's ID."""
        return self.info["id"]

    @property
    def gateway(self):
        """The name of the lab's gateway container."""
        return f"{GATEWAY_PREFIX}-{self.lab_id}"

    @property
    def volume(self):
        """The name of the lab's gateway identity volume."""
        return f"{VOLUME_PREFIX}-{self.lab_id}"

    def routes(self):
        """The lab's management subnets, the routes its gateway advertises."""
        return [ipaddress.ip_network(self.info[key]) for key in ("ipv4_network", "ipv6_network") if self.info.get(key)]

    def sherpa(self, label, arguments, timeout=COMMAND_TIMEOUT, env=None):
        """Run a `sherpa` command in the lab's folder and log it."""
        return run_logged(self.run.log, f"{self.name}: {label}", ["sherpa", *arguments], self.directory, timeout, env=env)

    def up(self, env=None):
        """Run `sherpa up` and read the lab's info; return the outcome."""
        self.directory.mkdir(parents=True)
        (self.directory / "manifest.toml").write_text(render_manifest(self.name, self.nodes, self.tailscale))
        self.made = True
        outcome = self.sherpa("sherpa up", ["up"], UP_TIMEOUT, env)
        info = self.directory / ".sherpa" / "lab-info.toml"
        if info.exists():
            self.info = tomllib.loads(info.read_text())
        return outcome

    def require_up(self, env=None):
        """Run `sherpa up` and fail unless it succeeded."""
        self.up_outcome = self.up(env)
        require(self.up_outcome.code == 0 and self.info,
                f"sherpa up of {self.name} failed (exit {self.up_outcome.code}); see {self.run.log.name}")

    def inspect(self):
        """The output of a successful `sherpa inspect`."""
        outcome = self.sherpa("sherpa inspect", ["inspect"])
        require(outcome.code == 0, f"sherpa inspect of {self.name} failed; see {self.run.log.name}")
        return outcome

    def node_rows(self):
        """Each node's row in `sherpa inspect`."""
        return parse_nodes(self.inspect().stdout, self.lab_id)

    def gateway_device(self):
        """The gateway's own tailnet device, read from its daemon."""
        status = self.run.docker.exec(f"{self.name} gateway self", self.gateway,
                                      ["tailscale", f"--socket={SOCKET}", "status", "--json", "--peers=false"])
        require(status.code == 0, f"could not read {self.gateway}'s status; see {self.run.log.name}")
        device = parse_self(status.stdout)
        self.run.devices.add(device)
        return device

    def ssh(self, label, node, command):
        """Run `command` on `node` through the lab's SSH config."""
        config = self.directory / ".sherpa" / "ssh" / "config"
        return run_logged(self.run.log, f"{self.name}: {label}",
                          ["ssh", "-F", str(config), "-o", "BatchMode=yes", "-o", "ConnectTimeout=15",
                           f"{node}.{self.lab_id}", command], self.directory)

    def destroy(self):
        """Destroy the lab if it is still there. Returns a failure, or None."""
        if not self.made or self.removed or not self.info:
            return None
        outcome = self.sherpa("sherpa destroy", ["destroy", "--yes"], UP_TIMEOUT)
        if outcome.code != 0:
            return TestFailure(f"sherpa destroy of {self.name} failed (exit {outcome.code}); see {self.run.log.name}")
        self.removed = True
        return None


class Run:
    """What every case shares: the logs, Docker, the probe, the first lab and every device made."""

    def __init__(self, root, key, admin_user, admin_password):
        self.root = root
        self.key = key
        self.admin_user = admin_user
        self.admin_password = admin_password
        self.log = (root / "commands.log").open("w")
        self.docker = Docker(self.log, root)
        self.server_ipv4, self.web_port, self.ca_cert = server_address()
        self.started = time.time()
        self.devices = set()
        self.probes = []
        self.labs = []
        self.probe = None
        self.lab = None
        self.owner = None

    def new_probe(self, accept_routes=True):
        """Start a probe of the run under a fresh name."""
        probe = Probe(self.docker, f"{PROBE_PREFIX}-{uuid.uuid4().hex[:8]}", self.server_ipv4, accept_routes)
        self.probes.append(probe)
        probe.start(self.key)
        self.devices.add(probe.device)
        return probe

    def new_lab(self, role, nodes, tailscale):
        """A lab of the run, removed at the end if a case did not remove it."""
        lab = Lab(self, role, nodes, tailscale)
        self.labs.append(lab)
        return lab

    def sudo_read(self, label, arguments):
        """What a command run as root printed, not logged, for searching for the key."""
        result = subprocess.run(["sudo", "-n", *arguments], capture_output=True, timeout=COMMAND_TIMEOUT, check=False)
        self.log.write(f"=== {label} (as root, not logged): exit {result.returncode}, {len(result.stdout)} bytes\n")
        require(result.returncode == 0, f"{label} failed as root: {mask(result.stderr.decode(errors='replace')).strip()}")
        return result.stdout

    def web_start(self, lab):
        """POST the web UI's whole-lab start with the CLI's token; return the status and the reply."""
        token = (Path.home() / ".sherpa" / "token").read_text().strip()
        add_secret(token)
        trusted = public_key(ssl.PEM_cert_to_DER_cert(Path(self.ca_cert).read_text()))
        context = ssl.create_default_context()
        context.check_hostname = False
        context.verify_mode = ssl.CERT_NONE
        connection = http.client.HTTPSConnection(self.server_ipv4, self.web_port, timeout=WEB_TIMEOUT, context=context)
        try:
            connection.connect()
            require(public_key(connection.sock.getpeercert(binary_form=True)) == trusted,
                    "the web UI's certificate is not for the key the CLI trusts")
            connection.request("POST", f"/labs/{lab.lab_id}/start", body=b"",
                               headers={"Cookie": f"{AUTH_COOKIE}={token}"})
            reply = connection.getresponse()
            status, body = reply.status, reply.read().decode(errors="replace")
        finally:
            connection.close()
        self.log.write(mask(f"=== POST /labs/{lab.lab_id}/start\n--- status {status}\n--- body\n{body}\n"))
        self.log.flush()
        return status, body

    def admin_account(self):
        """Log the admin in to a HOME of its own and check it is an admin other than the owner."""
        require(self.admin_user != self.owner, "the admin and the labs' owner must be different accounts")
        home = self.root / "admin-home"
        (home / ".sherpa").mkdir(parents=True, mode=0o700, exist_ok=True)
        source = Path.home() / ".sherpa"
        for part in ("config", "trusted_certs"):
            if (source / part).is_dir() and not (home / ".sherpa" / part).exists():
                shutil.copytree(source / part, home / ".sherpa" / part)
        env = {key: value for key, value in without_key().items() if key not in (LOGIN_USER_ENV, LOGIN_PASSWORD_ENV)}
        env["HOME"] = str(home)
        login = run_logged(self.log, "sherpa login (admin)", ["sherpa", "login"], self.root,
                           env={**env, LOGIN_USER_ENV: self.admin_user, LOGIN_PASSWORD_ENV: self.admin_password})
        require(login.code == 0, f"could not log in as {self.admin_user}; see {self.log.name}")
        token = home / ".sherpa" / "token"
        if token.exists():
            add_secret(token.read_text().strip())
        info = run_logged(self.log, "sherpa server user info (admin)", ["sherpa", "server", "user", "info"], self.root, env=env)
        found = INFO_ADMIN.search(info.stdout)
        require(info.code == 0 and found and found.group(1) == "Yes", f"{self.admin_user} is not an admin")
        return Account(self.admin_user, env)


def require_connected(output, what):
    """The Tailscale table in `output` shows the gateway connected; return its rows."""
    rows = parse_tailscale_table(output)
    require(rows.get("Gateway") == CONNECTED, f"{what} did not print the gateway connected: {rows or 'no Tailscale table'}")
    return rows


def require_same_device(lab, before, what):
    """The lab's gateway is running, connected and the same tailnet device as `before`."""
    require(lab.run.docker.running(lab.gateway), f"the gateway is not running after {what}")
    require_connected(lab.inspect().stdout, f"sherpa inspect after {what}")
    after = lab.gateway_device()
    require(after == before, f"the gateway became another device after {what}: {before} then {after}")
    require(lab.run.probe.has(after), f"the gateway's device {after} is not in the probe's peer list after {what}")


def require_reachable(run, lab):
    """The lab's routes reach the probe, and its VM answers SSH from the tailnet."""
    run.probe.wait_routes(lab.gateway_device(), lab.routes())
    run.probe.wait_banner(lab.node_rows()[VM].ipv4)


def require_running(lab, what):
    """Every node of the lab is running."""
    rows = lab.node_rows()
    stopped = {name: row.state for name, row in rows.items() if row.state != "running"}
    require(rows and not stopped, f"nodes not running after {what}: {stopped}")


def require_removed(run, lab, device, what):
    """The lab's gateway container and identity volume are gone, and its device left the tailnet."""
    require(run.docker.running(lab.gateway) is None, f"{lab.gateway} is still there after {what}")
    require(not run.docker.volume_exists(lab.volume), f"{lab.volume} is still there after {what}")
    run.probe.wait_gone(device)


def leftovers(run, before):
    """Fail if a lab, container, volume, network or tailnet device of the run is left."""
    failures = []
    after = run.docker.names()
    made = sorted(after - before)
    if made:
        failures.append(f"left on Docker: {made}")
    checker = None
    try:
        checker = run.new_probe(accept_routes=False)
        ours = {device.node_id for device in run.devices} - {checker.device.node_id}
        names = {device.hostname for device in run.devices} - {checker.device.hostname}
        try:
            wait_for(lambda: not [peer for peer in checker.peers() if peer.node_id in ours or peer.hostname in names],
                     PROPAGATION_TIMEOUT, "the run's devices leaving the tailnet")
        except TestFailure:
            left = [peer for peer in checker.peers() if peer.node_id in ours or peer.hostname in names]
            failures.append(f"left on the tailnet: {[(peer.hostname, peer.node_id) for peer in left]}")
    except TestFailure as error:
        failures.append(f"could not check the tailnet with a second probe: {error}")
    finally:
        if checker:
            stopped = checker.stop()
            if stopped:
                failures.append(str(stopped))
    return failures


# -- the fixtures --------------------------------------------------------------------------------


def preflight(run):
    """Stop unless the CLI, the server, the images and the probe image are ready."""
    for tool in ("sherpa", "docker", "ssh"):
        require(shutil.which(tool), f"{tool} is not on PATH")
    whoami = run_logged(run.log, "sherpa whoami", ["sherpa", "whoami"], run.root)
    require(whoami.code == 0, "the sherpa CLI is not logged in (run `sherpa login`)")
    require(run.ca_cert, "the CLI trusts no server certificate, so the server has no TLS, which a tailnet lab needs")
    run.owner = require_device_password(run.log, run.root)
    for model, kind, version in IMAGES:
        arguments = ["sherpa", "server", "image", "show", model]
        arguments += ["--kind", kind] if kind else []
        arguments += ["--version", version] if version else []
        image = run_logged(run.log, f"{model} image present", arguments, run.root)
        require(image.code == 0, f"the server has no {model} {version or 'default'} image")
    if run.docker.run("probe image", ["image", "inspect", PROBE_IMAGE]).code != 0:
        pulled = run.docker.run("pull probe image", ["pull", PROBE_IMAGE], timeout=600)
        require(pulled.code == 0, f"could not pull {PROBE_IMAGE}; see {run.log.name}")


@pytest.fixture(scope="module")
def run(request):
    key = os.environ.get(KEY_ENV)
    if not key:
        pytest.skip(f"{KEY_ENV} is not set: the run needs an auth key for the test tailnet")
    add_secret(key)
    admin_password = os.environ.get(ENV_ADMIN_PASSWORD)
    add_secret(admin_password)
    signal.signal(signal.SIGTERM, stop_on_sigterm)
    keep_failed = request.config.getoption("keep_failed")
    root = RUNS / f"tailnet-{uuid.uuid4().hex[:12]}"
    root.mkdir(parents=True)
    print(f"Logs: {root}", flush=True)
    run = Run(root, key, request.config.getoption("admin_user"), admin_password)
    keep = False
    before = None
    try:
        preflight(run)
        before = run.docker.names()
        try:
            run.probe = run.new_probe()
            run.lab = run.new_lab("a", (VM_NODE, CONTAINER_NODE), tailscale=True)
            run.lab.require_up()
        except Exception:
            keep = keep_failed
            raise
        yield run
        keep = keep_failed and request.session.testsfailed > 0
    finally:
        if keep:
            print(f"keeping the failed run's labs and probes; see {root}", flush=True)
            run.log.close()
        else:
            failures = [str(error) for error in (*(lab.destroy() for lab in reversed(run.labs)),
                                                 *(probe.stop() for probe in run.probes)) if error]
            if before is not None:
                failures += leftovers(run, before)
            run.log.close()
            if failures:
                raise TestFailure("; ".join(failures))


@pytest.fixture(scope="class")
def others(run):
    second = run.new_lab("b", (CONTAINER_NODE,), tailscale=False)
    third = run.new_lab("c", (CONTAINER_NODE,), tailscale=True)
    try:
        second.require_up()
        third.require_up()
        yield second, third
    finally:
        failures = [str(error) for error in (third.destroy(), second.destroy()) if error]
        if failures:
            raise TestFailure("; ".join(failures))


# -- cases ---------------------------------------------------------------------------------------


def test_tailnet_join_001_a_lab_with_tailscale_comes_up_with_its_gateway_connected_at_the_third_address(run):
    lab = run.lab
    rows = require_connected(lab.up_outcome.stdout, "sherpa up")
    addresses = [ipaddress.ip_address(address.strip()) for address in rows.get("Tailscale addresses", "").split(",")
                 if address.strip()]
    require(any(address in ipaddress.ip_network("100.64.0.0/10") for address in addresses),
            f"sherpa up printed no tailnet address: {rows}")
    network = f"{MANAGEMENT_PREFIX}-{lab.lab_id}"
    held = run.docker.run("gateway address", ["inspect", "-f", f'{{{{(index .NetworkSettings.Networks "{network}").IPAddress}}}}',
                                              lab.gateway])
    third = ipaddress.ip_network(lab.info["ipv4_network"])[GATEWAY_OFFSET]
    require(held.code == 0 and held.stdout.strip() == str(third),
            f"the gateway holds {held.stdout.strip() or 'no address'}, not the third address {third}")


def test_tailnet_join_002_a_node_answers_ssh_from_another_tailnet_device_without_ssh_jump(run):
    run.probe.wait_routes(run.lab.gateway_device(), run.lab.routes())
    run.probe.wait_banner(run.lab.node_rows()[VM].ipv4)


def test_tailnet_join_009_a_vm_and_a_container_both_answer_from_the_tailnet(run):
    rows = run.lab.node_rows()
    run.probe.wait_banner(rows[VM].ipv4)
    run.probe.wait_ping(rows[CONTAINER].ipv4)


def test_tailnet_join_010_a_node_answers_ssh_on_its_ipv4_and_its_ipv6_management_address(run):
    row = run.lab.node_rows()[VM]
    require(row.ipv6 not in ("", "-"), f"{VM} has no IPv6 management address: {row}")
    run.probe.wait_banner(row.ipv4)
    run.probe.wait_banner(row.ipv6)


def test_tailnet_routes_004_a_node_sees_the_tailnet_from_the_gateway_and_has_no_route_to_it(run):
    lab = run.lab
    row = lab.node_rows()[VM]
    run.probe.wait_banner(row.ipv4)
    probe_address = run.probe.address()
    third = str(ipaddress.ip_network(lab.info["ipv4_network"])[GATEWAY_OFFSET])
    holder = run.probe.hold(row.ipv4, HOLD_SECONDS)
    try:
        seen = wait_for(lambda: connection_from(lab, third), HOLD_SECONDS, f"a connection from the gateway {third} on {VM}")
    finally:
        holder.wait(timeout=HOLD_SECONDS + 30)
    require(probe_address not in seen, f"{VM} saw the probe's own address {probe_address}")
    route = lab.ssh("route to the probe", VM, f"ip route get {probe_address}")
    require(route.code == 0, f"could not read {VM}'s route to {probe_address}")
    require(f"via {third} " not in route.stdout, f"{VM} routes the tailnet through the gateway: {route.stdout.strip()}")


def connection_from(lab, address):
    """The VM's TCP connections when one comes from `address`, else None."""
    connections = lab.ssh("connections", VM, "netstat -tn").stdout
    return connections if f" {address}:" in connections else None


def test_tailnet_status_012_sherpa_inspect_prints_the_tailscale_table(run):
    lab = run.lab
    rows = require_connected(lab.inspect().stdout, "sherpa inspect")
    require(rows.get("Route approval") == APPROVED, f"sherpa inspect does not show the routes approved: {rows}")
    shown = {ipaddress.ip_network(route.strip()) for route in rows.get("Management routes", "").split(",") if route.strip()}
    require(shown == set(lab.routes()), f"sherpa inspect shows the routes {sorted(map(str, shown))}, "
                                         f"not {sorted(map(str, lab.routes()))}")
    require(rows.get("Tailscale addresses"), f"sherpa inspect shows no tailnet address: {rows}")


def test_tailnet_key_010_after_an_up_the_key_is_nowhere_on_the_server(run):
    if subprocess.run(["sudo", "-n", "true"], capture_output=True, check=False).returncode != 0:
        pytest.skip("reading the server's database, lab folder and log needs passwordless sudo")
    lab = run.lab
    places = {
        "the database": run.sudo_read("database", ["find", str(SHERPA_DIR / "db"), "-type", "f", "-exec", "cat", "{}", "+"]),
        "the server's lab folder": run.sudo_read("server lab folder", ["find", str(SHERPA_DIR / "labs"), "-type", "f",
                                                                     "-exec", "cat", "{}", "+"]),
        "sherpad's log files": run.sudo_read("log files", ["find", str(SHERPA_DIR / "logs"), "-type", "f",
                                                          "-exec", "cat", "{}", "+"]),
        "sherpad's journal": run.sudo_read("journal", ["journalctl", "-u", "sherpad", "--no-pager", "-o", "cat",
                                                       "--since", f"@{int(run.started)}"]),
        "the gateway's docker inspect": run.docker.read(["inspect", lab.gateway]),
        "the gateway's command lines": run.docker.read(["exec", lab.gateway, "sh", "-c", "cat /proc/[0-9]*/cmdline"]),
        "the gateway's /run": run.docker.read(["exec", lab.gateway, "sh", "-c", "find /run -type f -exec cat {} +"]),
        "the client's lab folder": b"".join(path.read_bytes() for path in lab.directory.rglob("*") if path.is_file()),
    }
    require(places["sherpad's journal"], "sherpad's journal has nothing since the run started, so the search proves nothing")
    holding = places_holding(run.key, places)
    require(not holding, f"the key is in {', '.join(holding)}")


def test_tailnet_join_004_an_up_with_a_key_the_tailnet_refuses_is_rolled_back(run):
    lab = run.new_lab("d", (CONTAINER_NODE,), tailscale=True)
    refused = f"tskey-auth-k{secrets.token_hex(6)}CNTRL-{secrets.token_hex(16)}"
    outcome = lab.up({**os.environ, KEY_ENV: refused})
    require(outcome.code != 0, "sherpa up with a key the tailnet refuses succeeded")
    lab_id = management_id(outcome.output)
    require(run.docker.running(f"{GATEWAY_PREFIX}-{lab_id}") is None, "the failed up left its gateway")
    require(not run.docker.volume_exists(f"{VOLUME_PREFIX}-{lab_id}"), "the failed up left its identity volume")
    inspect = lab.sherpa("sherpa inspect, after the failed up", ["inspect"])
    require(inspect.code != 0 and LAB_NOT_FOUND in inspect.output, f"the failed up left its lab record: {inspect.output.strip()[-300:]}")


class TestOtherLabs:
    """The cases that need a second lab without `[tailscale]` and a third with it."""

    def test_tailnet_routes_003_a_second_labs_node_and_the_servers_addresses_do_not_answer_through_the_gateway(self, run, others):
        second, _ = others
        run.probe.wait_banner(run.lab.node_rows()[VM].ipv4)
        node = second.node_rows()[CONTAINER]
        targets = [node.ipv4, second.info["ipv4_gateway"], run.server_ipv4]
        targets += [node.ipv6] if node.ipv6 not in ("", "-") else []
        answered = [target for target in targets if run.probe.ping(target) or run.probe.banner(target)]
        require(not answered, f"answered the probe through the tailnet: {answered}")

    def test_tailnet_routes_005_the_gateway_takes_no_routes_and_no_dns_from_the_tailnet(self, run, others):
        _, third = others
        run.probe.wait_routes(third.gateway_device(), third.routes())
        offered = {ipaddress.ip_network(route) for peer in run.probe.peers() for route in peer.routes}
        require(offered, "no device offers the tailnet any route, so the case proves nothing")
        listing = "".join(run.docker.exec(f"gateway routes -{family}", run.lab.gateway,
                                          ["ip", f"-{family}", "route", "show", "table", "all"]).stdout
                          for family in ("4", "6"))
        taken = sorted(str(route) for route in tailnet_routes(listing) & offered)
        require(not taken, f"the gateway took routes from the tailnet: {taken}")
        resolv = run.docker.exec("gateway resolv.conf", run.lab.gateway, ["cat", "/etc/resolv.conf"]).stdout
        require("100.100.100.100" not in resolv and "tailscale" not in resolv.lower(),
                f"the gateway took DNS settings from the tailnet: {resolv.strip()}")

    def test_tailnet_removal_011_sherpa_server_clean_removes_the_gateway_and_its_identity_volume(self, run, others):
        if not (run.admin_user and run.admin_password):
            pytest.skip(f"needs an admin: --admin-user or ${ENV_ADMIN_USER}, and ${ENV_ADMIN_PASSWORD}")
        _, third = others
        device = third.gateway_device()
        admin = run.admin_account()
        clean = run_logged(run.log, f"sherpa server clean {third.lab_id} (admin)",
                           ["sherpa", "server", "clean", third.lab_id], run.root, UP_TIMEOUT, env=admin.env)
        require(clean.code == 0, f"sherpa server clean failed (exit {clean.code}); see {run.log.name}")
        third.removed = True
        shutil.rmtree(third.directory / ".sherpa", ignore_errors=True)
        run_logged(run.log, "sherpa ssh-config clean", ["sherpa", "ssh-config", "clean"], run.root)
        require_removed(run, third, device, "sherpa server clean")
        inspect = third.sherpa("sherpa inspect, after clean", ["inspect"])
        require(inspect.code != 0 and LAB_NOT_FOUND in inspect.output, f"the cleaned lab is still there: {inspect.output.strip()[-300:]}")


def test_tailnet_suspend_011_one_nodes_down_and_resume_leave_the_gateway_connected_without_a_key(run):
    lab = run.lab
    before = lab.gateway_device()
    for verb in ("down", "resume"):
        outcome = lab.sherpa(f"sherpa {verb} --node {CONTAINER}, no key", [verb, "--node", CONTAINER], UP_TIMEOUT, without_key())
        require(outcome.code == 0, f"sherpa {verb} --node {CONTAINER} failed (exit {outcome.code}); see {run.log.name}")
        require_same_device(lab, before, f"sherpa {verb} --node {CONTAINER}")


def test_tailnet_suspend_012_a_redeploy_of_a_node_leaves_the_gateway_connected(run):
    lab = run.lab
    before = lab.gateway_device()
    outcome = lab.sherpa(f"sherpa redeploy --node {CONTAINER}", ["redeploy", "--node", CONTAINER], UP_TIMEOUT, without_key())
    require(outcome.code == 0, f"sherpa redeploy failed (exit {outcome.code}); see {run.log.name}")
    require_same_device(lab, before, "sherpa redeploy")


def test_tailnet_suspend_025_a_resume_after_every_node_was_stopped_needs_no_key_and_keeps_the_device(run):
    lab = run.lab
    before = lab.gateway_device()
    for node in (VM, CONTAINER):
        down = lab.sherpa(f"sherpa down --node {node}", ["down", "--node", node], UP_TIMEOUT, without_key())
        require(down.code == 0, f"sherpa down --node {node} failed (exit {down.code}); see {run.log.name}")
    resume = lab.sherpa("sherpa resume, no key", ["resume"], UP_TIMEOUT, without_key())
    require(resume.code == 0, f"sherpa resume without a key failed (exit {resume.code}); see {run.log.name}")
    require_running(lab, "the resume")
    require_same_device(lab, before, "the resume")


def test_tailnet_suspend_014_the_web_ui_resumes_a_lab_only_while_its_gateway_is_connected(run):
    lab = run.lab
    before = lab.gateway_device()
    for node in (VM, CONTAINER):
        down = lab.sherpa(f"sherpa down --node {node}", ["down", "--node", node], UP_TIMEOUT, without_key())
        require(down.code == 0, f"sherpa down --node {node} failed (exit {down.code}); see {run.log.name}")
    status, body = run.web_start(lab)
    require(status == 200, f"the web UI's start of a lab with its gateway connected answered {status}: {body[:300]}")
    require_running(lab, "the web UI's start")
    require_same_device(lab, before, "the web UI's start")

    down = lab.sherpa("sherpa down", ["down"], UP_TIMEOUT)
    require(down.code == 0, f"sherpa down failed (exit {down.code}); see {run.log.name}")
    try:
        status, body = run.web_start(lab)
        require(status != 200 and WEB_REFUSAL in body,
                f"the web UI's start of a lab whose gateway is stopped was not refused with {WEB_REFUSAL!r}: "
                f"{status} {body[:300]}")
        require(run.docker.running(lab.gateway) is False, "the refused start left the gateway running")
        started = {name: row.state for name, row in lab.node_rows().items() if row.state != "stopped"}
        require(not started, f"the refused start started nodes: {started}")
    finally:
        resume = lab.sherpa("sherpa resume", ["resume"], UP_TIMEOUT)
    require(resume.code == 0, f"sherpa resume failed (exit {resume.code}); see {run.log.name}")
    require_reachable(run, lab)


def test_tailnet_suspend_001_down_stops_the_gateway_and_takes_it_and_the_nodes_off_the_tailnet(run):
    lab = run.lab
    rows = lab.node_rows()
    device = lab.gateway_device()
    down = lab.sherpa("sherpa down", ["down"], UP_TIMEOUT)
    require(down.code == 0, f"sherpa down failed (exit {down.code}); see {run.log.name}")
    require(run.docker.running(lab.gateway) is False, "the gateway is still running after sherpa down")
    run.probe.wait_gone(device)
    answered = [address for address in (rows[VM].ipv4, rows[CONTAINER].ipv4) if run.probe.ping(address)]
    answered += [rows[VM].ipv4] if run.probe.banner(rows[VM].ipv4) else []
    require(not answered, f"nodes still answer from the tailnet after sherpa down: {answered}")


def test_tailnet_suspend_002_resume_with_the_key_connects_the_gateway_again(run):
    lab = run.lab
    resume = lab.sherpa("sherpa resume", ["resume"], UP_TIMEOUT)
    require(resume.code == 0, f"sherpa resume failed (exit {resume.code}); see {run.log.name}")
    require_connected(resume.stdout, "sherpa resume")
    require_reachable(run, lab)


def test_tailnet_suspend_013_a_resume_whose_enrollment_fails_starts_nothing_and_the_next_one_enrolls(run):
    lab = run.lab
    down = lab.sherpa("sherpa down", ["down"], UP_TIMEOUT)
    require(down.code == 0, f"sherpa down failed (exit {down.code}); see {run.log.name}")
    refused = f"tskey-auth-k{secrets.token_hex(6)}CNTRL-{secrets.token_hex(16)}"
    failed = lab.sherpa("sherpa resume, refused key", ["resume"], UP_TIMEOUT, {**os.environ, KEY_ENV: refused})
    require(failed.code != 0, "sherpa resume with a key the tailnet refuses succeeded")
    started = {name: row.state for name, row in lab.node_rows().items() if row.state != "stopped"}
    require(not started, f"the failed resume started nodes: {started}")
    require(run.docker.running(lab.gateway) is False, "the failed resume left the gateway running")
    resume = lab.sherpa("sherpa resume", ["resume"], UP_TIMEOUT)
    require(resume.code == 0, f"sherpa resume after the failed one failed (exit {resume.code}); see {run.log.name}")
    require_connected(resume.stdout, "sherpa resume after the failed one")
    require_reachable(run, lab)


@pytest.mark.skipif(os.environ.get(RESTART_DOCKER_ENV) != "1",
                    reason=f"restarts Docker on the server host; run with --restart-docker or {RESTART_DOCKER_ENV}=1")
def test_tailnet_suspend_019_after_docker_restarts_resume_brings_the_gateway_back_as_a_new_device(run):
    lab = run.lab
    old = lab.gateway_device()
    stopped = run.probe.stop()
    require(stopped is None, str(stopped))
    restart = run_logged(run.log, "systemctl restart docker", ["sudo", "-n", "systemctl", "restart", "docker"], run.root,
                         timeout=300)
    require(restart.code == 0, f"could not restart Docker (exit {restart.code}): {restart.output.strip()}")
    run.docker.wait_ready()
    run.probe = run.new_probe()
    require(not run.docker.running(lab.gateway), "the gateway is running after Docker restarted, before the resume")
    resume = lab.sherpa("sherpa resume", ["resume"], UP_TIMEOUT)
    require(resume.code == 0, f"sherpa resume after Docker restarted failed (exit {resume.code}); see {run.log.name}")
    require_connected(resume.stdout, "sherpa resume after Docker restarted")
    new = lab.gateway_device()
    require(new.node_id != old.node_id, f"the gateway came back as its old device {old}")
    run.probe.wait_gone(old)
    require(run.probe.has(new), f"the gateway's new device {new} is not in the probe's peer list")
    require_reachable(run, lab)


def test_tailnet_removal_001_destroy_removes_the_gateway_its_volume_and_its_device(run):
    lab = run.lab
    device = lab.gateway_device()
    destroy = lab.sherpa("sherpa destroy", ["destroy", "--yes"], UP_TIMEOUT)
    require(destroy.code == 0, f"sherpa destroy failed (exit {destroy.code}); see {run.log.name}")
    lab.removed = True
    require_removed(run, lab, device, "sherpa destroy")


# -- running -------------------------------------------------------------------------------------


def main():
    parser = argparse.ArgumentParser(description=__doc__,
                                     formatter_class=argparse.RawDescriptionHelpFormatter,
                                     epilog="Other arguments, such as --admin-user, --keep-failed or -k, are passed to pytest.")
    parser.add_argument("--self-test", action="store_true", help="Run the runner's own unit tests")
    parser.add_argument("--restart-docker", action="store_true",
                        help="Also run tailnet_suspend_019, which restarts Docker on this host with sudo")
    args, pytest_args = parser.parse_known_args()
    if args.self_test:
        sys.argv = sys.argv[:1]
        unittest.main()
        return 0
    if not os.environ.get(KEY_ENV):
        print(f"{KEY_ENV} is not set: the tailnet lab test needs an auth key for the test tailnet. "
              f"Nothing was run. See tests/README.md, Live tailnet tests.", file=sys.stderr)
        return 2
    if args.restart_docker:
        os.environ[RESTART_DOCKER_ENV] = "1"
    return pytest.main(["-v", "-p", "no:cacheprovider", "--durations=0", __file__, *pytest_args])


class SelfTest(unittest.TestCase):
    # The runner's own checks, run with --self-test; pytest must not collect them as lab cases.
    __test__ = False

    STATUS = json.dumps({"Self": {"ID": "nSelf", "HostName": "sherpa-test-tnprobe-1"},
                         "Peer": {"k1": {"HostName": "sherpa-tailnet-abc", "ID": "nGw", "Online": True,
                                         "PrimaryRoutes": ["172.28.1.0/24"]},
                                  "k2": {"HostName": "laptop", "ID": "nLap", "Online": False, "PrimaryRoutes": None}}})
    TABLE = ("┌─────────────────────┬──────────────────┐\n"
             "│ Tailscale           │ Details          │\n"
             "├─────────────────────┼──────────────────┤\n"
             "│ Gateway             │ Connected        │\n"
             "│ Management routes   │ 172.28.1.0/24,   │\n"
             "│                     │ fd00:b0d1:0:1::/ │\n"
             "│                     │ 64               │\n"
             "└─────────────────────┴──────────────────┘\n"
             "│ Gateway             │ Stopped          │\n")

    def test_peers_are_matched_by_host_name_and_node_id(self):
        peers = parse_peers(self.STATUS)
        self.assertEqual(peers[0], Peer("sherpa-tailnet-abc", "nGw", True, ("172.28.1.0/24",)))
        self.assertEqual(peers[1].routes, ())
        self.assertEqual(parse_self(self.STATUS), Device("sherpa-test-tnprobe-1", "nSelf"))
        self.assertEqual(parse_peers('{"Peer": null}'), [])

    def test_the_tailscale_table_joins_wrapped_values_and_stops_at_its_end(self):
        rows = parse_tailscale_table(f"before\n{self.TABLE}")
        self.assertEqual(rows["Gateway"], "Connected")
        self.assertEqual(rows["Management routes"], "172.28.1.0/24, fd00:b0d1:0:1::/ 64")
        self.assertEqual(parse_tailscale_table("no table"), {})

    def test_node_rows_are_read_by_their_lab_id(self):
        row = "│ vm01.ab12 │ alpine_linux │ virtual_machine │ running │ 172.28.1.21 │ fd00:b0d1:0:1::15 │ 1 │ 1024 │ - │"
        other = row.replace("ab12", "cd34")
        self.assertEqual(parse_nodes(f"{row}\n{other}", "ab12"), {"vm01": NodeRow("running", "172.28.1.21", "fd00:b0d1:0:1::15")})

    def test_the_lab_id_is_read_from_the_management_network(self):
        self.assertEqual(management_id("Creating management network: sherpa-management-fd9642fc\n"), "fd9642fc")
        with self.assertRaises(TestFailure):
            management_id("Lab creation failed")

    def test_only_routes_through_tailscale0_count_as_taken(self):
        listing = ("100.66.9.117 dev tailscale0 table 52\n10.100.52.0/23 dev tailscale0 table 52\n"
                   "default via 172.28.1.1 dev eth0\n172.28.2.0/24 dev eth0 proto kernel\n"
                   "local fd7a:115c:a1e0::1 dev tailscale0 table local\n")
        taken = tailnet_routes(listing)
        self.assertIn(ipaddress.ip_network("10.100.52.0/23"), taken)
        self.assertNotIn(ipaddress.ip_network("172.28.2.0/24"), taken)
        self.assertIn(ipaddress.ip_network("fd7a:115c:a1e0::1/128"), taken)

    def test_the_manifest_names_the_variable_and_never_holds_the_key(self):
        with mock.patch.dict(os.environ, {KEY_ENV: "tskey-auth-secret123"}):
            text = render_manifest("tna-abcdef", (VM_NODE,), tailscale=True)
        manifest = tomllib.loads(text)
        self.assertEqual(manifest["tailscale"], {"enabled": True, "auth_key_env": KEY_ENV})
        self.assertNotIn("tskey-auth-secret123", text)
        self.assertNotIn("tailscale", tomllib.loads(render_manifest("tnb-abcdef", (VM_NODE,), tailscale=False)))

    @unittest.skipUnless(shutil.which("openssl"), "needs openssl to make certificates")
    def test_a_renewed_certificate_keeps_its_key_pin_and_another_key_does_not(self):
        with tempfile.TemporaryDirectory() as temp:
            def certificate(name, key):
                if not (Path(temp) / key).exists():
                    subprocess.run(["openssl", "genpkey", "-algorithm", "EC", "-pkeyopt", "ec_paramgen_curve:P-256",
                                    "-out", str(Path(temp) / key)], check=True, capture_output=True)
                subprocess.run(["openssl", "req", "-x509", "-new", "-key", str(Path(temp) / key), "-subj", f"/CN={name}",
                                "-days", "1", "-out", str(Path(temp) / name)], check=True, capture_output=True)
                return public_key(ssl.PEM_cert_to_DER_cert((Path(temp) / name).read_text()))
            first = certificate("first", "key")
            self.assertEqual(first, certificate("renewed", "key"))
            self.assertNotEqual(first, certificate("other", "other-key"))

    def test_the_key_is_found_wherever_it_is(self):
        self.assertEqual(places_holding("tskey-auth-x1", {"a": b"..tskey-auth-x1..", "b": b"tskey-auth-x"}), ["a"])

    def test_waiting_fails_after_its_deadline(self):
        with mock.patch(f"{__name__}.POLL_SECONDS", 0), self.assertRaises(TestFailure):
            wait_for(lambda: False, 0, "nothing")

    def test_without_the_key_the_run_stops_before_anything_is_made(self):
        with mock.patch.dict(os.environ, {}, clear=True), \
                mock.patch.object(sys, "argv", ["tailnet_lab_test.py"]), \
                mock.patch("pytest.main") as pytest_main, \
                mock.patch("sys.stderr", new_callable=lambda: tempfile.TemporaryFile("w+")) as stderr:
            self.assertEqual(main(), 2)
            stderr.seek(0)
            self.assertIn(KEY_ENV, stderr.read())
        pytest_main.assert_not_called()


if __name__ == "__main__":
    sys.exit(main())
