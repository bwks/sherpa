#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.15"
# dependencies = ["pytest==9.2.0"]
# ///
"""End-to-end test of private bridges through the sherpa CLI (Python 3.15+, pytest).

Brings up a lab with a VM and containers in one `[[bridges]]` entry, two of them also on P2p
links, addresses every bridge member and link end, and checks the private bridge cases of
tests/specs/links-and-bridges.md with ping: every bridge member reaches every other member; the
same after redeploying containers on a link, a container on no link and the VM; a node with a
bridge member and a link has both working; and destroy leaves nothing of the lab on the host.
See README.md.

A pytest module: the `prepared` fixture checks the server and writes the lab's manifest, the
`lab` fixture brings the lab up once for the module and destroys it at the end, and the
`addressed` fixture waits for every node and addresses its interfaces. The cases are the
`test_<case_id>_*` functions, in case order; the last destroys the lab. Run as a script, it runs
pytest on itself; `--self-test` runs the runner's own unit tests instead.
"""

import argparse
from pathlib import Path
import shutil
import signal
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
    COMMAND_TIMEOUT,
    Outcome,
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
# Redeployed in this order: two containers on a link (veths), one on no link (Docker networks),
# then the VM.
REDEPLOYED = ("fr01", "fr03", "fr04", "dev01")
# Bridge members are numbered from the first address of the bridge subnet; link `n` (0-based)
# uses the subnet after it.
SUBNET_PREFIX = "10.45"

UP_TIMEOUT = 900
READY_TIMEOUT = 600
POLL_INTERVAL = 10
PING_ATTEMPTS = 5

# OpenSSH options for lab nodes: no prompts, and no host key notices mixed into command output.
SSH_OPTIONS = ("-F", ".sherpa/ssh/config", "-o", "BatchMode=yes", "-o", "LogLevel=ERROR")
# Root inside the lab: the FRR image has sudo, Alpine has doas.
AS_ROOT = 'if command -v sudo >/dev/null; then as_root="sudo -n"; else as_root="doas -n"; fi; '
# A redeployed node has new MAC addresses. Its peers would wait out their stale neighbour
# entries before asking again, so every node forgets its neighbours before a ping round.
FLUSH_NEIGHBOURS = AS_ROOT + "$as_root ip neigh flush all"
# What a destroyed lab must not leave on the host, and how to list it. A leftover is a line that
# holds the lab's ID.
HOST_LISTINGS = (
    ("libvirt domains", ["virsh", "-c", "qemu:///system", "list", "--all", "--name"]),
    ("libvirt networks", ["virsh", "-c", "qemu:///system", "net-list", "--all", "--name"]),
    ("Docker networks", ["docker", "network", "ls", "--format", "{{.Name}}"]),
    ("host interfaces", ["ip", "-brief", "link"]),
)


class Address(NamedTuple):
    node: str
    interface: str
    address: str


class Plan(NamedTuple):
    """Every bridge member's and link end's address, and the pings that check them: each bridge
    member pings every other member, and each link end the other end."""

    addresses: list
    bridge_pings: list
    link_pings: list


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
    """The addresses of every bridge member and link end, and the pings that check them."""
    addresses = []
    bridge_pings = []
    link_pings = []
    for bridge_idx, bridge in enumerate(manifest.get("bridges", [])):
        members = [Address(*endpoint(member), f"{SUBNET_PREFIX}.{bridge_idx}.{host}/24")
                   for host, member in enumerate(bridge["links"], start=1)]
        addresses += members
        bridge_pings += [(source, target) for source in members for target in members if source != target]
    first_link_subnet = len(manifest.get("bridges", []))
    for link_idx, link in enumerate(manifest.get("links", [])):
        subnet = f"{SUBNET_PREFIX}.{first_link_subnet + link_idx}"
        ends = [Address(*endpoint(link["src"]), f"{subnet}.1/24"),
                Address(*endpoint(link["dst"]), f"{subnet}.2/24")]
        addresses += ends
        link_pings += [(ends[0], ends[1]), (ends[1], ends[0])]
    return Plan(addresses, bridge_pings, link_pings)


def host(address):
    return address.address.split("/")[0]


def configure_command(addresses):
    """One remote command that brings up and addresses a node's interfaces."""
    steps = [f'$as_root ip link set {item.interface} up && '
             f'$as_root ip addr replace {item.address} dev {item.interface}' for item in addresses]
    return AS_ROOT + " && ".join(steps)


def leftovers(listing, lab_id):
    """The first word of each line of a host listing that holds the lab's ID."""
    return [line.split()[0] for line in listing.splitlines() if lab_id in line]


# -- the lab -------------------------------------------------------------------------------------


class Lab:
    """The lab every case uses: its manifest written by `prepare`, brought up once by `up` and
    destroyed by `destroy`, by the last case or the `lab` fixture's teardown."""

    def __init__(self, run_root):
        self.manifest = tomllib.loads((LAB / "manifest.toml").read_text())
        self.name = f"{self.manifest['name']}-{run_root.name[-6:]}"
        self.directory = run_root / "lab"
        self.plan = address_plan(self.manifest)
        self.started = False
        self.destroyed = False
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

    # -- lifecycle ---------------------------------------------------------------------------

    def prepare(self):
        self.directory.mkdir(parents=True)
        self.log = (self.directory / "commands.log").open("w")
        (self.directory / "manifest.toml").write_text(render_manifest(self.manifest, self.name))

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

    def redeploy(self, node):
        self.sherpa(f"sherpa redeploy {node}", ["redeploy", "--node", node], UP_TIMEOUT)

    def destroy(self):
        """Destroy the lab and check `sherpa inspect` no longer finds it. Does nothing when the lab
        was never started or is already destroyed."""
        if not self.started or self.destroyed:
            return
        if self.lab_id is None:
            # `sherpa up` failed early; destroy whatever it created, found by the manifest's lab name.
            self.sherpa("sherpa destroy (after failed up)", ["destroy", "--yes"], UP_TIMEOUT, check=False)
            self.destroyed = True
            return
        self.sherpa("sherpa destroy", ["destroy", "--yes"], UP_TIMEOUT)
        self.destroyed = True
        inspect = self.sherpa("lab gone", ["inspect"], check=False)
        require(inspect.code != 0, f"lab {self.lab_id} still exists after destroy")

    def host_leftovers(self):
        """What of the lab is left on the host, by kind: libvirt domains and networks, Docker
        networks and host interfaces whose name holds the lab's ID; and the kinds that could not
        be listed here, such as libvirt off the Sherpa server."""
        found = {}
        unlisted = []
        for kind, arguments in HOST_LISTINGS:
            if not shutil.which(arguments[0]):
                unlisted.append(f"{kind} ({arguments[0]} not found)")
                continue
            listing = self.command(f"no leftover {kind}", arguments, check=False)
            if listing.code != 0:
                unlisted.append(f"{kind} (exit {listing.code})")
                continue
            left = leftovers(listing.stdout, self.lab_id)
            if left:
                found[kind] = left
        return found, unlisted

    # -- nodes -------------------------------------------------------------------------------

    def wait_for_node(self, node, timeout=READY_TIMEOUT):
        deadline = time.monotonic() + timeout
        while self.node(f"{node} reachable", node, "true", check=False).code != 0:
            require(time.monotonic() < deadline, f"{node} not reachable over SSH after {timeout}s")
            time.sleep(POLL_INTERVAL)

    def address_node(self, node):
        """Wait for the node and bring up and address its bridge member and link ends."""
        self.wait_for_node(node)
        addresses = [item for item in self.plan.addresses if item.node == node]
        outcome = self.node(f"address {node}", node, configure_command(addresses), check=False)
        require(outcome.code == 0, f"could not address {node} {[a.interface for a in addresses]}: "
                f"{outcome.output.strip()}")

    def require_pings(self, pings, stage):
        """Every ping gets a reply, sent from its source interface, after every node forgets its
        neighbours."""
        for node in self.manifest["nodes"]:
            self.node(f"flush {node['name']} neighbours", node["name"], FLUSH_NEIGHBOURS)
        failed = [f"{source.node}:{source.interface} -> {target.node}:{target.interface} ({host(target)})"
                  for source, target in pings if not self.ping(source, target)]
        require(not failed, f"{stage}, no reply on: {'; '.join(failed)}")

    def ping(self, source, target):
        for attempt in range(PING_ATTEMPTS):
            outcome = self.node(f"ping {source.node} -> {target.node} {host(target)}", source.node,
                                f"ping -c 1 -W 2 -I {source.interface} {host(target)}", check=False)
            if outcome.code == 0:
                return True
            time.sleep(1 + attempt)
        return False


# -- the fixtures --------------------------------------------------------------------------------


def preflight(run_root):
    for tool in ("sherpa", "ssh"):
        require(shutil.which(tool), f"{tool} is not on PATH")
    manifest = tomllib.loads((LAB / "manifest.toml").read_text())
    with (run_root / "preflight.log").open("w") as log:
        whoami = run_logged(log, "sherpa whoami", ["sherpa", "whoami"], run_root)
        require(whoami.code == 0, "the sherpa CLI is not logged in (run `sherpa login`)")
        require_device_password(log, run_root)
        images = sorted({(node["model"], node["version"]) for node in manifest["nodes"]})
        for model, version in images:
            image = run_logged(log, f"{model} {version} image present",
                               ["sherpa", "image", "show", model, "--version", version], run_root)
            require(image.code == 0, f"the server has no {model} {version} image")


def keep_failed_lab(request):
    """Whether the lab is to be left running: `--keep-failed` is given and a case has failed."""
    return request.config.getoption("keep_failed") and request.session.testsfailed > 0


@pytest.fixture(scope="module")
def prepared():
    """The lab before it is up: the server checked and the run's manifest written."""
    signal.signal(signal.SIGTERM, stop_on_sigterm)
    run_root = RUNS / f"private-bridge-{uuid.uuid4().hex[:12]}"
    run_root.mkdir(parents=True)
    print(f"Logs: {run_root}", flush=True)
    preflight(run_root)
    lab = Lab(run_root)
    lab.prepare()
    try:
        yield lab
    finally:
        lab.log.close()


@pytest.fixture(scope="module")
def lab(request, prepared):
    """The lab every case uses: brought up once and destroyed at the end, even when a case fails,
    unless `--keep-failed` is given and something failed. The last case destroys it itself."""
    keep = False
    try:
        try:
            prepared.up()
        except Exception:
            keep = request.config.getoption("keep_failed")
            raise
        yield prepared
        keep = keep_failed_lab(request)
    finally:
        if keep and not prepared.destroyed:
            print(f"keeping failed lab {prepared.name} ({prepared.lab_id}) in {prepared.directory}", flush=True)
        else:
            prepared.destroy()


@pytest.fixture(scope="module")
def addressed(lab):
    """The lab with every node reachable and its bridge member and link ends addressed, once for
    the module."""
    for node in lab.manifest["nodes"]:
        lab.address_node(node["name"])
    return lab


# -- cases ---------------------------------------------------------------------------------------


def test_private_bridge_001_a_vm_and_containers_on_and_off_links_in_one_bridge_reach_every_member(addressed):
    addressed.require_pings(addressed.plan.bridge_pings, "after up")


def test_private_bridge_002_members_reach_each_other_after_redeploying_containers_and_the_vm(addressed):
    # A redeploy remakes the node's link ends as well as its bridge member, so the links are
    # checked too.
    for node in REDEPLOYED:
        stage = f"after redeploying {node}"
        addressed.redeploy(node)
        addressed.address_node(node)
        addressed.require_pings(addressed.plan.bridge_pings + addressed.plan.link_pings, stage)


def test_private_bridge_003_a_node_with_a_bridge_member_and_a_link_has_both_working(addressed):
    linked = {item.node for source_target in addressed.plan.link_pings for item in source_target}
    pings = [ping for ping in addressed.plan.bridge_pings if ping[0].node in linked]
    addressed.require_pings(pings + addressed.plan.link_pings, f"on {', '.join(sorted(linked))}")


def test_link_cleanup_002_destroying_a_lab_with_a_bridge_leaves_no_bridge_or_network(lab, request):
    if keep_failed_lab(request):
        pytest.skip("--keep-failed: a case failed, so the lab is kept")
    lab.destroy()
    found, unlisted = lab.host_leftovers()
    require(not found, "left after destroy: " + "; ".join(f"{kind} {names}" for kind, names in found.items()))
    if unlisted:
        pytest.skip(f"not checked, run on the Sherpa server to check them: {'; '.join(unlisted)}")


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

    def test_every_bridge_member_pings_every_other_and_each_link_end_its_peer(self):
        manifest = {"bridges": [{"links": ["a::eth1", "b::eth1", "c::eth1"]}],
                    "links": [{"src": "a::eth2", "dst": "b::eth2"}]}
        plan = address_plan(manifest)
        self.assertEqual(len(plan.bridge_pings), 6)
        self.assertEqual({(s.node, t.node) for s, t in plan.link_pings}, {("a", "b"), ("b", "a")})
        self.assertTrue(all(s.interface == "eth2" for s, _ in plan.link_pings))
        self.assertEqual(len({item.address.rsplit(".", 1)[0] for item in plan.addresses}), 2)

    def test_a_leftover_is_a_listed_name_holding_the_lab_id(self):
        listing = "veth-4ed6360e-x UP\nlo UNKNOWN\nfr01-4ed6360e\n"
        self.assertEqual(leftovers(listing, "4ed6360e"), ["veth-4ed6360e-x", "fr01-4ed6360e"])
        self.assertEqual(leftovers("lo UNKNOWN\n", "4ed6360e"), [])

    def test_a_partly_created_lab_keeps_its_id_for_cleanup(self):
        # `sherpa up` exits 1 on a partly created lab after writing lab-info.toml; the run must
        # still know the lab's ID, so destroy checks the lab is gone instead of skipping it.
        with tempfile.TemporaryDirectory() as temp:
            lab = Lab(Path(temp) / "private-bridge-000000abcdef")
            lab.prepare()
            self.addCleanup(lab.log.close)
            (lab.directory / ".sherpa").mkdir()
            (lab.directory / ".sherpa" / "lab-info.toml").write_text(
                f'id = "4ed6360e"\nname = "{lab.name}"\n')

            def failing_up(log, label, arguments, *rest, **options):
                return Outcome(1 if arguments[:2] == ["sherpa", "up"] else 0, "", "")

            with mock.patch(f"{__name__}.run_logged", failing_up), self.assertRaises(TestFailure):
                lab.up()
            self.assertEqual(lab.lab_id, "4ed6360e")

            def still_there(log, label, arguments, *rest, **options):
                return Outcome(0, "", "")

            with mock.patch(f"{__name__}.run_logged", still_there), self.assertRaises(TestFailure):
                lab.destroy()


if __name__ == "__main__":
    sys.exit(main())
