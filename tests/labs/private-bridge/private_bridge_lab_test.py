#!/usr/bin/env python3
"""End-to-end test of private bridges through the sherpa CLI (Python 3.11+, stdlib only).

Brings up a lab with a VM and containers in one `[[bridges]]` entry, addresses every data
connection, and checks each one with ping: after `sherpa up`, and after redeploying a
container, a P2p container and the VM. Always destroys the lab. See README.md.
"""

import argparse
import json
from pathlib import Path
import shlex
import shutil
import signal
import subprocess
import sys
import time
import tomllib
from typing import NamedTuple
import unittest
import uuid


LAB = Path(__file__).resolve().parent
ROOT = LAB.parent.parent.parent
RUNS = ROOT / ".tmp" / "lab-tests"
# Redeployed in this order: a Docker-networked container, a P2p container, then the VM.
REDEPLOYED = ("fr01", "fr03", "dev01")
# Bridge members are numbered from the first address of the bridge subnet; link `n` (0-based)
# uses the subnet after it.
SUBNET_PREFIX = "10.45"

COMMAND_TIMEOUT = 180
UP_TIMEOUT = 900
READY_TIMEOUT = 600
POLL_INTERVAL = 10
PING_ATTEMPTS = 5

# OpenSSH options for lab nodes: no prompts, and no host key notices mixed into command output.
SSH_OPTIONS = ("-F", "sherpa_ssh_config", "-o", "BatchMode=yes", "-o", "LogLevel=ERROR")
# Root inside the lab: the FRR image has sudo, Alpine has doas.
AS_ROOT = 'if command -v sudo >/dev/null; then as_root="sudo -n"; else as_root="doas -n"; fi; '
# A redeployed node has new MAC addresses. Its peers would wait out their stale neighbour
# entries before asking again, so every node forgets its neighbours before a ping round.
FLUSH_NEIGHBOURS = AS_ROOT + "$as_root ip neigh flush all"


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


class Address(NamedTuple):
    node: str
    interface: str
    address: str


def require(condition, message):
    if not condition:
        raise TestFailure(message)


def run_logged(log, label, arguments, cwd, timeout=COMMAND_TIMEOUT):
    """Run a command, record it and its output in the log, and return its Outcome."""
    print(f"[{cwd.name}] {label}", flush=True)
    try:
        result = subprocess.run(arguments, cwd=cwd, capture_output=True, text=True, timeout=timeout)
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
    if isinstance(value, bool):
        return "true" if value else "false"
    if isinstance(value, int):
        return str(value)
    if isinstance(value, str):
        return json.dumps(value)
    if isinstance(value, list):
        return "[" + ", ".join(value_toml(item) for item in value) + "]"
    if isinstance(value, dict):
        return "{ " + ", ".join(f"{key} = {value_toml(item)}" for key, item in value.items()) + " }"
    raise TypeError(f"Unsupported TOML value: {value!r}")


def render_manifest(manifest, name):
    """The lab manifest for one run, under a unique lab name."""
    lines = [f"name = {value_toml(name)}"]
    for section in ("nodes", "links", "bridges"):
        lines.append("")
        lines.append(f"{section} = [")
        for entry in manifest.get(section, []):
            lines.append(f"  {value_toml(entry)},")
        lines.append("]")
    return "\n".join(lines) + "\n"


def endpoint(text):
    node, interface = text.split("::")
    return node, interface


def address_plan(manifest):
    """Addresses for every bridge member and link end, and the pings that check them.

    Every bridge member pings every other member, and each link end pings the other end.
    """
    addresses = []
    pings = []
    for bridge_idx, bridge in enumerate(manifest.get("bridges", [])):
        members = [Address(*endpoint(member), f"{SUBNET_PREFIX}.{bridge_idx}.{host}/24")
                   for host, member in enumerate(bridge["links"], start=1)]
        addresses += members
        pings += [(source, target) for source in members for target in members if source != target]
    first_link_subnet = len(manifest.get("bridges", []))
    for link_idx, link in enumerate(manifest.get("links", [])):
        subnet = f"{SUBNET_PREFIX}.{first_link_subnet + link_idx}"
        ends = [Address(*endpoint(link["src"]), f"{subnet}.1/24"),
                Address(*endpoint(link["dst"]), f"{subnet}.2/24")]
        addresses += ends
        pings += [(ends[0], ends[1]), (ends[1], ends[0])]
    return addresses, pings


def host(address):
    return address.address.split("/")[0]


def configure_command(addresses):
    """One remote command that brings up and addresses a node's interfaces."""
    steps = [f'$as_root ip link set {item.interface} up && '
             f'$as_root ip addr replace {item.address} dev {item.interface}' for item in addresses]
    return AS_ROOT + " && ".join(steps)


class LabRun:
    def __init__(self, run_root, keep_failed):
        self.manifest = tomllib.loads((LAB / "manifest.toml").read_text())
        self.name = f"{self.manifest['name']}-{run_root.name[-6:]}"
        self.directory = run_root / "lab"
        self.keep_failed = keep_failed
        self.addresses, self.pings = address_plan(self.manifest)
        self.started = False
        self.lab_id = None
        self.log = None

    # -- commands ----------------------------------------------------------------------------

    def command(self, label, arguments, timeout=COMMAND_TIMEOUT, check=True):
        outcome = run_logged(self.log, label, arguments, self.directory, timeout)
        if check and outcome.code != 0:
            raise TestFailure(f"{label} failed (exit {outcome.code}); see {self.log.name}")
        return outcome

    def sherpa(self, label, arguments, timeout=COMMAND_TIMEOUT, check=True):
        return self.command(label, ["sherpa", *arguments], timeout, check)

    def node(self, label, node, remote_command, timeout=COMMAND_TIMEOUT, check=True):
        return self.command(label, ["ssh", *SSH_OPTIONS, f"{node}.{self.lab_id}", remote_command],
                            timeout, check)

    # -- lab ---------------------------------------------------------------------------------

    def run(self):
        self.directory.mkdir(parents=True)
        with (self.directory / "commands.log").open("w") as self.log:
            (self.directory / "manifest.toml").write_text(render_manifest(self.manifest, self.name))
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
        self.sherpa("sherpa up", ["up"], UP_TIMEOUT)
        info = tomllib.loads((self.directory / "lab-info.toml").read_text())
        require(info["name"] == self.name, f"lab-info.toml names {info['name']}, not {self.name}")
        self.lab_id = info["id"]

    def checks(self):
        for node in self.manifest["nodes"]:
            self.wait_for_node(node["name"])
            self.configure(node["name"])
        self.check_pings("after up")
        for node in REDEPLOYED:
            self.sherpa(f"sherpa redeploy {node}", ["redeploy", "--node", node], UP_TIMEOUT)
            self.wait_for_node(node)
            self.configure(node)
            self.check_pings(f"after redeploying {node}")

    def wait_for_node(self, node, timeout=READY_TIMEOUT):
        deadline = time.monotonic() + timeout
        while self.node(f"{node} reachable", node, "true", check=False).code != 0:
            require(time.monotonic() < deadline, f"{node} not reachable over SSH after {timeout}s")
            time.sleep(POLL_INTERVAL)

    def configure(self, node):
        addresses = [item for item in self.addresses if item.node == node]
        outcome = self.node(f"address {node}", node, configure_command(addresses), check=False)
        require(outcome.code == 0, f"could not address {node} {[a.interface for a in addresses]}: "
                f"{outcome.output.strip()}")

    def check_pings(self, stage):
        for node in self.manifest["nodes"]:
            self.node(f"flush {node['name']} neighbours", node["name"], FLUSH_NEIGHBOURS)
        failed = []
        for source, target in self.pings:
            if not self.ping(source, target):
                failed.append(f"{source.node}:{source.interface} -> "
                              f"{target.node}:{target.interface} ({host(target)})")
        require(not failed, f"{stage}, no reply on: {'; '.join(failed)}")

    def ping(self, source, target):
        for attempt in range(PING_ATTEMPTS):
            outcome = self.node(f"ping {source.node} -> {target.node} {host(target)}", source.node,
                                f"ping -c 1 -W 2 -I {source.interface} {host(target)}", check=False)
            if outcome.code == 0:
                return True
            time.sleep(1 + attempt)
        return False

    def destroy(self):
        """Destroy the lab and check nothing is left. Returns a failure, or None."""
        if not self.started:
            return None
        if not (self.directory / "lab-info.toml").exists():
            # `sherpa up` failed early; destroy whatever it created, found by the manifest's lab name.
            self.sherpa("sherpa destroy (after failed up)", ["destroy", "--yes"], UP_TIMEOUT, check=False)
            return None
        try:
            destroy = self.sherpa("sherpa destroy", ["destroy", "--yes"], UP_TIMEOUT)
            require("partially destroyed" not in destroy.output,
                    f"sherpa destroy did not clean up everything; see {self.log.name}")
            inspect = self.sherpa("lab gone", ["inspect"], check=False)
            require(inspect.code != 0, f"lab {self.lab_id} still exists after destroy")
            self.no_host_leftovers()
        except TestFailure as error:
            return error
        return None

    def no_host_leftovers(self):
        """When run on the Sherpa server, check its libvirt, Docker and host interfaces."""
        checks = (
            ("libvirt domains", ["virsh", "-c", "qemu:///system", "list", "--all", "--name"]),
            ("libvirt networks", ["virsh", "-c", "qemu:///system", "net-list", "--all", "--name"]),
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
            leftovers = [line.split()[0] for line in listing.stdout.splitlines()
                         if self.lab_id in line]
            require(not leftovers, f"{kind} left after destroy: {leftovers}")


def preflight(run_root):
    for tool in ("sherpa", "ssh"):
        require(shutil.which(tool), f"{tool} is not on PATH")
    manifest = tomllib.loads((LAB / "manifest.toml").read_text())
    with (run_root / "preflight.log").open("w") as log:
        whoami = run_logged(log, "sherpa whoami", ["sherpa", "whoami"], run_root)
        require(whoami.code == 0, "the sherpa CLI is not logged in (run `sherpa login`)")
        images = sorted({(node["model"], node["version"]) for node in manifest["nodes"]})
        for model, version in images:
            image = run_logged(log, f"{model} {version} image present",
                               ["sherpa", "image", "show", model, "--version", version], run_root)
            require(image.code == 0, f"the server has no {model} {version} image")


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
    run_root = RUNS / f"private-bridge-{uuid.uuid4().hex[:12]}"
    run_root.mkdir(parents=True)
    print(f"Logs: {run_root}", flush=True)
    try:
        preflight(run_root)
        LabRun(run_root, args.keep_failed).run()
    except TestFailure as error:
        print(f"FAIL {error}. Logs: {run_root}", file=sys.stderr)
        return 1
    print(f"PASS. Logs: {run_root}")
    return 0


class SelfTest(unittest.TestCase):
    def manifest(self):
        return tomllib.loads((LAB / "manifest.toml").read_text())

    def test_rendered_manifest_round_trips_with_a_new_name(self):
        manifest = self.manifest()
        rendered = tomllib.loads(render_manifest(manifest, "renamed"))
        self.assertEqual(rendered["name"], "renamed")
        for section in ("nodes", "links", "bridges"):
            self.assertEqual(rendered[section], manifest[section])

    def test_every_bridge_member_and_link_end_gets_a_unique_address(self):
        addresses, _ = address_plan(self.manifest())
        self.assertEqual(len(addresses), 8)
        self.assertEqual(len({item.address for item in addresses}), len(addresses))
        self.assertEqual(len({(item.node, item.interface) for item in addresses}), len(addresses))

    def test_bridge_members_ping_each_other_and_link_ends_ping_their_peer(self):
        _, pings = address_plan(self.manifest())
        bridge_pings = [p for p in pings if p[0].address.startswith(f"{SUBNET_PREFIX}.0.")]
        self.assertEqual(len(bridge_pings), 4 * 3)
        self.assertEqual(len(pings), 4 * 3 + 2 + 2)
        self.assertIn((Address("fr01", "eth2", "10.45.1.1/24"), Address("fr02", "eth2", "10.45.1.2/24")),
                      pings)

    def test_every_redeployed_node_is_in_the_lab(self):
        names = {node["name"] for node in self.manifest()["nodes"]}
        self.assertTrue(set(REDEPLOYED) <= names)

    def test_configure_command_addresses_each_interface(self):
        command = configure_command([Address("fr01", "eth1", "10.45.0.2/24"),
                                     Address("fr01", "eth2", "10.45.1.1/24")])
        self.assertTrue(command.startswith(AS_ROOT))
        self.assertIn("ip addr replace 10.45.0.2/24 dev eth1", command)
        self.assertIn("ip addr replace 10.45.1.1/24 dev eth2", command)

    def test_outcome_keeps_streams_apart_for_parsing(self):
        outcome = Outcome(0, "x", "Warning\n")
        self.assertEqual(outcome.stdout, "x")
        self.assertIn("Warning", outcome.output)


if __name__ == "__main__":
    sys.exit(main())
