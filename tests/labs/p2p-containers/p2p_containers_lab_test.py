#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.15"
# dependencies = ["pytest==9.2.0"]
# ///
"""End-to-end test of P2p links on container nodes through the sherpa CLI (Python 3.15+, pytest).

Brings up three routers (cEOS, SR Linux, FRR) on a triangle of routed P2p links, with cEOS also
the access switch for application and OS containers on P2p links. Some of those images have no
`ip` command or no shell. Checks the P2p container cases of tests/specs/links-and-bridges.md:
every host's link interface is named, up, promiscuous, jumbo and has carrier; every host reaches
the gateway, the other hosts and the far routers, and each router its link peers; the same after
redeploying a host without a shell, a host without `ip` and a router; an unused interface has no
carrier; and destroy leaves nothing of the lab on the host. Runs on the Sherpa server; see
README.md.

A pytest module: the `prepared` fixture checks the server and writes the lab's manifest, the
`lab` fixture brings the lab up once for the module and destroys it at the end, and the
`addressed` fixture configures the routers and addresses the hosts for the cases that send
traffic. The cases are the `test_<case_id>_*` functions, in case order; the last destroys the
lab. Run as a script, it runs pytest on itself; `--self-test` runs the runner's own unit tests
instead.
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
ROUTERS = ("sw01", "srl01", "frr01")
# Redeployed in this order: a host without a shell, a host without `ip`, then a router.
REDEPLOYED = ("surreal01", "mongo01", "frr01")
# The image of the throwaway container that joins a node's network namespace to check and
# address its interfaces, so the node's own image needs neither a shell nor `ip`.
SIDECAR_IMAGE = "alpine:3.22"

GATEWAY = "10.46.10.1"
HOST_SUBNET = "10.46.10"
CORE_SUBNET = "10.46.0.0/24"
# Router addresses on the routed links, and the pings across each link.
SRL01_TO_SW01 = "10.46.0.1"
FRR01_TO_SW01 = "10.46.0.3"
FRR01_TO_SRL01 = "10.46.0.5"
ROUTER_PINGS = (("sw01", SRL01_TO_SW01), ("sw01", FRR01_TO_SW01), ("srl01", FRR01_TO_SRL01))
# Each router's link peers. A redeployed router has new MAC addresses, and SR Linux keeps a
# neighbour entry for hours, so a redeployed router first pings its peers: their ARP requests
# replace the stale entries, as a gratuitous ARP would.
ROUTER_PEERS = {
    "sw01": (SRL01_TO_SW01, FRR01_TO_SW01),
    "srl01": ("10.46.0.0", FRR01_TO_SRL01),
    "frr01": ("10.46.0.2", "10.46.0.4"),
}
# Host interfaces and an unused sw01 interface, which must have no carrier.
HOST_INTERFACE = "eth1"
UNUSED_SW01_INTERFACE = "eth16"
JUMBO_MTU = 9600
IFF_UP = 0x1
IFF_PROMISC = 0x100
AFTER_UP = "after up"

UP_TIMEOUT = 1200
READY_TIMEOUT = 900
POLL_INTERVAL = 10
PING_ROUNDS = 6

SW01_CONFIG = """configure
ip routing
interface Ethernet1
no switchport
ip address 10.46.0.0/31
interface Ethernet2
no switchport
ip address 10.46.0.2/31
vlan 10
interface Ethernet3-15
switchport access vlan 10
interface Vlan10
ip address 10.46.10.1/24
end"""
SRL01_CONFIG = """set / interface ethernet-1/1 admin-state enable subinterface 0 ipv4 admin-state enable address 10.46.0.1/31
set / interface ethernet-1/2 admin-state enable subinterface 0 ipv4 admin-state enable address 10.46.0.4/31
set / network-instance default interface ethernet-1/1.0
set / network-instance default interface ethernet-1/2.0
set / network-instance default next-hop-groups group to-sw01 nexthop 1 ip-address 10.46.0.0
set / network-instance default static-routes route 10.46.10.0/24 next-hop-group to-sw01
"""
FRR01_CONFIG = ("configure terminal", "interface eth1", "ip address 10.46.0.3/31", "exit",
                "interface eth2", "ip address 10.46.0.5/31", "exit",
                "ip route 10.46.10.0/24 10.46.0.2", "end")
# What a destroyed lab must not leave on the host, and how to list it. A leftover is a line that
# holds the lab's ID.
HOST_LISTINGS = (
    ("Docker containers", ["docker", "ps", "--all", "--format", "{{.Names}}"]),
    ("Docker networks", ["docker", "network", "ls", "--format", "{{.Name}}"]),
    ("host interfaces", ["ip", "-brief", "link"]),
)


class Interface(NamedTuple):
    """An interface's state, from sysfs in the node's network namespace."""

    flags: int
    carrier: bool
    mtu: int


def render_manifest(manifest, name):
    """The lab manifest for one run, under a unique lab name."""
    lines = [f"name = {value_toml(name)}"]
    for section in ("nodes", "links"):
        lines.append("")
        lines.append(f"{section} = [")
        for entry in manifest.get(section, []):
            lines.append(f"  {value_toml(entry)},")
        lines.append("]")
    return "\n".join(lines) + "\n"


def host_addresses(manifest):
    """Each host's address in the host VLAN, numbered in manifest order."""
    hosts = [node["name"] for node in manifest["nodes"] if node["name"] not in ROUTERS]
    return {host: f"{HOST_SUBNET}.{10 + index}" for index, host in enumerate(hosts, start=1)}


def interface_script(interface):
    """A sidecar command printing an interface's flags, carrier and MTU, or `missing`."""
    path = f"/sys/class/net/{interface}"
    return (f"if [ -d {path} ]; then echo $(cat {path}/flags) $(cat {path}/carrier 2>/dev/null "
            f"|| echo 0) $(cat {path}/mtu); else echo missing; fi")


def parse_interface(text):
    """The Interface from `interface_script` output, or None when the interface is missing."""
    fields = text.split()
    if fields == ["missing"]:
        return None
    require(len(fields) == 3, f"unexpected interface state: {text.strip()!r}")
    return Interface(int(fields[0], 16), fields[1] == "1", int(fields[2]))


def active_interface_problems(state):
    """What is wrong with an interface that should be up and passing traffic, if anything."""
    if state is None:
        return ["missing"]
    problems = []
    if not state.flags & IFF_UP:
        problems.append("down")
    if not state.flags & IFF_PROMISC:
        problems.append("not promiscuous")
    if not state.carrier:
        problems.append("no carrier")
    if state.mtu != JUMBO_MTU:
        problems.append(f"MTU {state.mtu}")
    return problems


def address_script(address):
    """A sidecar command that addresses a host and routes the router links through the gateway."""
    return (f"ip addr replace {address}/24 dev {HOST_INTERFACE} && "
            f"ip route replace {CORE_SUBNET} via {GATEWAY} && ip neigh flush all")


def ping_script(targets):
    """A sidecar command pinging each target once, printing `ok <target>` for each reply."""
    return "; ".join(f"ping -c 1 -W 2 -I {HOST_INTERFACE} {target} >/dev/null 2>&1 && "
                     f"echo ok {target}" for target in targets) + "; true"


def replied(output):
    return {line.split()[1] for line in output.splitlines() if line.startswith("ok ")}


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
        self.hosts = host_addresses(self.manifest)
        self.started = False
        self.destroyed = False
        self.lab_id = None
        self.log = None

    # -- commands ----------------------------------------------------------------------------

    def command(self, label, arguments, timeout=COMMAND_TIMEOUT, check=True, stdin=None):
        outcome = run_logged(self.log, label, arguments, self.directory, timeout, stdin=stdin)
        if check and outcome.code != 0:
            raise TestFailure(f"{label} failed (exit {outcome.code}); see {self.log.name}")
        return outcome

    def sherpa(self, label, arguments, timeout=COMMAND_TIMEOUT, check=True):
        return self.command(label, ["sherpa", *arguments], timeout, check)

    def container(self, node):
        return f"{node}-{self.lab_id}"

    def node_exec(self, label, node, arguments, check=True, stdin=None):
        """Run a command in a router's own container."""
        interactive = ["-i"] if stdin is not None else []
        return self.command(label, ["docker", "exec", *interactive, self.container(node), *arguments],
                            check=check, stdin=stdin)

    def sidecar(self, label, node, script, check=True):
        """Run a shell script as root in a throwaway container sharing the node's network namespace."""
        return self.command(label, ["docker", "run", "--rm", "--network", f"container:{self.container(node)}",
                                    "--cap-add", "NET_ADMIN", "--cap-add", "NET_RAW", SIDECAR_IMAGE,
                                    "sh", "-c", script], check=check)

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
        """What of the lab is left on the host, by kind: containers, Docker networks and host
        interfaces whose name holds the lab's ID."""
        found = {}
        for kind, arguments in HOST_LISTINGS:
            left = leftovers(self.command(f"no leftover {kind}", arguments).stdout, self.lab_id)
            if left:
                found[kind] = left
        return found

    # -- routers and hosts -------------------------------------------------------------------

    def configure_router(self, router, timeout=READY_TIMEOUT):
        """Apply a router's config, retrying while its NOS is still booting."""
        deadline = time.monotonic() + timeout
        while not self.apply_router_config(router):
            require(time.monotonic() < deadline, f"could not configure {router} within {timeout}s")
            time.sleep(POLL_INTERVAL)

    def apply_router_config(self, router):
        if router == "sw01":
            outcome = self.node_exec("configure sw01", router, ["Cli", "-p", "15", "-c", SW01_CONFIG],
                                     check=False)
            return outcome.code == 0 and "% " not in outcome.output.replace("! IP configuration", "")
        if router == "srl01":
            outcome = self.node_exec("configure srl01", router, ["sr_cli", "-ec"], check=False,
                                     stdin=SRL01_CONFIG)
            return outcome.code == 0 and "committed" in outcome.output
        arguments = ["vtysh"]
        for line in FRR01_CONFIG:
            arguments += ["-c", line]
        return self.node_exec("configure frr01", router, arguments, check=False).code == 0

    def interface(self, node, interface):
        """The state of one of the node's interfaces, or None when it is missing."""
        outcome = self.sidecar(f"{node} {interface} state", node, interface_script(interface))
        return parse_interface(outcome.stdout)

    def require_active_interface(self, host, stage):
        """The host's link interface is named, up, promiscuous, jumbo and has carrier."""
        problems = active_interface_problems(self.interface(host, HOST_INTERFACE))
        require(not problems, f"{stage}, {host} {HOST_INTERFACE}: {', '.join(problems)}")

    def address_host(self, host):
        self.sidecar(f"address {host}", host, address_script(self.hosts[host]))

    def host_targets(self, host):
        others = [address for other, address in self.hosts.items() if other != host]
        return [GATEWAY, SRL01_TO_SW01, FRR01_TO_SW01, *others]

    def router_ping(self, router, target):
        if router == "sw01":
            outcome = self.node_exec(f"ping sw01 -> {target}", router,
                                     ["Cli", "-p", "15", "-c", f"ping {target} repeat 1"], check=False)
            return " 1 received" in outcome.output
        if router == "srl01":
            command = ["ip", "netns", "exec", "srbase-default", "ping", "-c", "1", "-W", "2", target]
        else:
            command = ["ping", "-c", "1", "-W", "2", target]
        return self.node_exec(f"ping {router} -> {target}", router, command, check=False).code == 0

    def require_pings(self, stage):
        """Every host reaches each of its targets and each router its link peers, retrying while
        neighbours learn the addresses of redeployed nodes."""
        pending = {host: set(self.host_targets(host)) for host in self.hosts}
        pending_routers = set(ROUTER_PINGS)
        for round_number in range(PING_ROUNDS):
            for host, targets in pending.items():
                if targets:
                    outcome = self.sidecar(f"pings from {host}", host, ping_script(sorted(targets)),
                                           check=False)
                    targets -= replied(outcome.stdout)
            for router, target in sorted(pending_routers):
                if self.router_ping(router, target):
                    pending_routers.discard((router, target))
            if not pending_routers and not any(pending.values()):
                return
            time.sleep(POLL_INTERVAL * (round_number + 1) // 2)
        failed = [f"{host} -> {target}" for host, targets in pending.items() for target in sorted(targets)]
        failed += [f"{router} -> {target}" for router, target in sorted(pending_routers)]
        raise TestFailure(f"{stage}, no reply on: {'; '.join(failed)}")


# -- the fixtures --------------------------------------------------------------------------------


def preflight(run_root):
    for tool in ("sherpa", "docker", "ip"):
        require(shutil.which(tool), f"{tool} is not on PATH")
    manifest = tomllib.loads((LAB / "manifest.toml").read_text())
    with (run_root / "preflight.log").open("w") as log:
        whoami = run_logged(log, "sherpa whoami", ["sherpa", "whoami"], run_root)
        require(whoami.code == 0, "the sherpa CLI is not logged in (run `sherpa login`)")
        require_device_password(log, run_root)
        docker = run_logged(log, f"{SIDECAR_IMAGE} present", ["docker", "image", "inspect", SIDECAR_IMAGE],
                            run_root)
        require(docker.code == 0, f"Docker has no {SIDECAR_IMAGE} image, or this user cannot use Docker; "
                f"this test runs on the Sherpa server")
        images = sorted({(node["model"], node.get("kind", ""), node["version"]) for node in manifest["nodes"]})
        for model, kind, version in images:
            arguments = ["sherpa", "image", "show", model, "--version", version]
            if kind:
                arguments += ["--kind", kind]
            image = run_logged(log, f"{model} {kind} {version} image present", arguments, run_root)
            require(image.code == 0, f"the server has no {model} {kind} {version} image")


def keep_failed_lab(request):
    """Whether the lab is to be left running: `--keep-failed` is given and a case has failed."""
    return request.config.getoption("keep_failed") and request.session.testsfailed > 0


@pytest.fixture(scope="module")
def prepared():
    """The lab before it is up: the server checked and the run's manifest written."""
    signal.signal(signal.SIGTERM, stop_on_sigterm)
    run_root = RUNS / f"p2p-containers-{uuid.uuid4().hex[:12]}"
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
    """The lab with every router configured and every host addressed, once for the module."""
    for router in ROUTERS:
        lab.configure_router(router)
    for host in lab.hosts:
        lab.address_host(host)
    return lab


# -- cases ---------------------------------------------------------------------------------------


def test_p2p_link_006_a_container_link_interface_is_named_up_promiscuous_with_carrier_and_mtu_9600(lab):
    problems = []
    for host in lab.hosts:
        state = active_interface_problems(lab.interface(host, HOST_INTERFACE))
        if state:
            problems.append(f"{host} {HOST_INTERFACE}: {', '.join(state)}")
    require(not problems, f"{AFTER_UP}, {'; '.join(problems)}")


def test_p2p_link_007_nos_application_and_os_containers_reach_each_other_directly_and_routed(addressed):
    addressed.require_pings(AFTER_UP)


def test_link_reattach_005_redeployed_containers_with_no_shell_or_no_ip_and_a_router_reach_every_peer(addressed):
    for node in REDEPLOYED:
        stage = f"after redeploying {node}"
        addressed.redeploy(node)
        if node in ROUTERS:
            addressed.configure_router(node)
            for peer in ROUTER_PEERS[node]:
                addressed.router_ping(node, peer)
        else:
            addressed.require_active_interface(node, stage)
            addressed.address_host(node)
        addressed.require_pings(stage)


def test_node_interfaces_004_an_unused_container_interface_is_there_with_no_carrier(lab):
    # The interface's admin state is the NOS's: cEOS brings all its interfaces up.
    state = lab.interface("sw01", UNUSED_SW01_INTERFACE)
    require(state is not None, f"sw01 {UNUSED_SW01_INTERFACE} is missing")
    require(not state.carrier, f"sw01 {UNUSED_SW01_INTERFACE} should have no carrier: {state}")


def test_link_cleanup_008_destroying_a_container_lab_leaves_no_host_interface_or_docker_network(lab, request):
    if keep_failed_lab(request):
        pytest.skip("--keep-failed: a case failed, so the lab is kept")
    lab.destroy()
    found = lab.host_leftovers()
    require(not found, "left after destroy: " + "; ".join(f"{kind} {names}" for kind, names in found.items()))


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

    def test_interface_state_is_parsed_from_sysfs(self):
        self.assertIsNone(parse_interface("missing\n"))
        state = parse_interface("0x1103 1 9600\n")
        self.assertEqual(active_interface_problems(state), [])
        self.assertEqual(active_interface_problems(parse_interface("0x1002 0 1500\n")),
                         ["down", "not promiscuous", "no carrier", "MTU 1500"])

    def test_only_addresses_that_replied_are_counted(self):
        self.assertEqual(replied("ok 10.46.10.1\nnoise\n"), {"10.46.10.1"})

    def test_a_leftover_is_a_listed_name_holding_the_lab_id(self):
        listing = "veth-4ed6360e-x UP\nlo UNKNOWN\nsw01-4ed6360e\n"
        self.assertEqual(leftovers(listing, "4ed6360e"), ["veth-4ed6360e-x", "sw01-4ed6360e"])
        self.assertEqual(leftovers("lo UNKNOWN\n", "4ed6360e"), [])

    def test_a_partly_created_lab_keeps_its_id_for_cleanup(self):
        # `sherpa up` exits 1 on a partly created lab after writing lab-info.toml; the run must
        # still know the lab's ID, so destroy checks the lab is gone instead of skipping it.
        with tempfile.TemporaryDirectory() as temp:
            lab = Lab(Path(temp) / "p2p-containers-000000abcdef")
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
