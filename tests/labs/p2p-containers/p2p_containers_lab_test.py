#!/usr/bin/env python3
"""End-to-end test of P2p links on container nodes through the sherpa CLI (Python 3.11+, stdlib only).

Brings up three routers (cEOS, SR Linux, FRR) on a triangle of routed P2p links, with cEOS also
the access switch for application containers on P2p links. Some of those images have no `ip`
command or no shell. Checks every container interface is named, up, promiscuous and has carrier,
that unused interfaces have no carrier, and that every host reaches the gateway, the other hosts and
the far routers: after `sherpa up`, and after redeploying a host without a shell, a host without
`ip` and a router. Always destroys the lab. Runs on the Sherpa server; see README.md.
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

COMMAND_TIMEOUT = 180
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


class Interface(NamedTuple):
    """An interface's state, from sysfs in the node's network namespace."""

    flags: int
    carrier: bool
    mtu: int


def require(condition, message):
    if not condition:
        raise TestFailure(message)


def run_logged(log, label, arguments, cwd, timeout=COMMAND_TIMEOUT, stdin=None):
    """Run a command, record it and its output in the log, and return its Outcome."""
    print(f"[{cwd.name}] {label}", flush=True)
    try:
        result = subprocess.run(arguments, cwd=cwd, capture_output=True, text=True, timeout=timeout,
                                input=stdin)
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


class LabRun:
    def __init__(self, run_root, keep_failed):
        self.manifest = tomllib.loads((LAB / "manifest.toml").read_text())
        self.name = f"{self.manifest['name']}-{run_root.name[-6:]}"
        self.directory = run_root / "lab"
        self.keep_failed = keep_failed
        self.hosts = host_addresses(self.manifest)
        self.started = False
        self.lab_id = None
        self.log = None

    # -- commands ----------------------------------------------------------------------------

    def command(self, label, arguments, timeout=COMMAND_TIMEOUT, check=True, stdin=None):
        outcome = run_logged(self.log, label, arguments, self.directory, timeout, stdin)
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
        for router in ROUTERS:
            self.configure_router(router)
        for host in self.hosts:
            self.check_host_interface(host)
            self.address_host(host)
        self.check_unused_interface_down()
        self.check_pings("after up")
        for node in REDEPLOYED:
            self.sherpa(f"sherpa redeploy {node}", ["redeploy", "--node", node], UP_TIMEOUT)
            if node in ROUTERS:
                self.configure_router(node)
                for peer in ROUTER_PEERS[node]:
                    self.router_ping(node, peer)
            else:
                self.check_host_interface(node)
                self.address_host(node)
            self.check_pings(f"after redeploying {node}")

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

    def check_host_interface(self, host):
        """The host's link interface is named, up, promiscuous, jumbo and has carrier."""
        outcome = self.sidecar(f"{host} {HOST_INTERFACE} state", host, interface_script(HOST_INTERFACE))
        problems = active_interface_problems(parse_interface(outcome.stdout))
        require(not problems, f"{host} {HOST_INTERFACE}: {', '.join(problems)}")

    def check_unused_interface_down(self):
        """An interface with no link is there, with no carrier. Its own admin state is the
        NOS's: cEOS brings all its interfaces up."""
        outcome = self.sidecar(f"sw01 {UNUSED_SW01_INTERFACE} state", "sw01",
                               interface_script(UNUSED_SW01_INTERFACE))
        state = parse_interface(outcome.stdout)
        require(state is not None, f"sw01 {UNUSED_SW01_INTERFACE} is missing")
        require(not state.carrier, f"sw01 {UNUSED_SW01_INTERFACE} should have no carrier: {state}")

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

    def check_pings(self, stage):
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

    def destroy(self):
        """Destroy the lab and check nothing is left. Returns a failure, or None."""
        if not self.started:
            return None
        if not (self.directory / "lab-info.toml").exists():
            # `sherpa up` failed early; destroy whatever it created, found by the manifest's lab name.
            self.sherpa("sherpa destroy (after failed up)", ["destroy", "--yes"], UP_TIMEOUT, check=False)
            return None
        try:
            self.sherpa("sherpa destroy", ["destroy", "--yes"], UP_TIMEOUT)
            inspect = self.sherpa("lab gone", ["inspect"], check=False)
            require(inspect.code != 0, f"lab {self.lab_id} still exists after destroy")
            self.no_host_leftovers()
        except TestFailure as error:
            return error
        return None

    def no_host_leftovers(self):
        checks = (
            ("Docker containers", ["docker", "ps", "--all", "--format", "{{.Names}}"]),
            ("Docker networks", ["docker", "network", "ls", "--format", "{{.Name}}"]),
            ("host interfaces", ["ip", "-brief", "link"]),
        )
        for kind, arguments in checks:
            listing = self.command(f"no leftover {kind}", arguments)
            leftovers = [line.split()[0] for line in listing.stdout.splitlines() if self.lab_id in line]
            require(not leftovers, f"{kind} left after destroy: {leftovers}")


def preflight(run_root):
    for tool in ("sherpa", "docker", "ip"):
        require(shutil.which(tool), f"{tool} is not on PATH")
    manifest = tomllib.loads((LAB / "manifest.toml").read_text())
    with (run_root / "preflight.log").open("w") as log:
        whoami = run_logged(log, "sherpa whoami", ["sherpa", "whoami"], run_root)
        require(whoami.code == 0, "the sherpa CLI is not logged in (run `sherpa login`)")
        docker = run_logged(log, f"{SIDECAR_IMAGE} present", ["docker", "image", "inspect", SIDECAR_IMAGE],
                            run_root)
        require(docker.code == 0, f"Docker has no {SIDECAR_IMAGE} image, or this user cannot use Docker; "
                f"this test runs on the Sherpa server")
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
    run_root = RUNS / f"p2p-containers-{uuid.uuid4().hex[:12]}"
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
        for section in ("nodes", "links"):
            self.assertEqual(rendered[section], manifest[section])

    def test_every_link_is_p2p(self):
        self.assertTrue(all(link.get("p2p") for link in self.manifest()["links"]))

    def test_every_host_has_a_unique_address_and_a_sw01_access_port(self):
        manifest = self.manifest()
        hosts = host_addresses(manifest)
        self.assertEqual(len(set(hosts.values())), len(hosts))
        self.assertNotIn(GATEWAY, hosts.values())
        access_links = {link["src"].split("::")[0]: link["dst"] for link in manifest["links"]
                        if link["src"].split("::")[0] in hosts}
        self.assertEqual(set(access_links), set(hosts))
        self.assertTrue(all(dst.startswith("sw01::") for dst in access_links.values()))

    def test_access_ports_match_the_sw01_vlan_range(self):
        ports = sorted(int(link["dst"].split("::eth")[1]) for link in self.manifest()["links"]
                       if link["dst"].startswith("sw01::"))
        self.assertEqual(ports, list(range(3, 16)))
        self.assertIn("interface Ethernet3-15", SW01_CONFIG)
        self.assertNotIn(int(UNUSED_SW01_INTERFACE.removeprefix("eth")), ports)

    def test_every_router_announces_to_both_link_peers(self):
        self.assertEqual(set(ROUTER_PEERS), set(ROUTERS))
        self.assertTrue(all(len(peers) == 2 for peers in ROUTER_PEERS.values()))

    def test_every_redeployed_node_is_in_the_lab(self):
        names = {node["name"] for node in self.manifest()["nodes"]}
        self.assertTrue(set(REDEPLOYED) <= names)

    def test_interface_state_is_parsed_from_sysfs(self):
        self.assertIsNone(parse_interface("missing\n"))
        state = parse_interface("0x1103 1 9600\n")
        self.assertEqual(active_interface_problems(state), [])
        self.assertEqual(active_interface_problems(parse_interface("0x1002 0 1500\n")),
                         ["down", "not promiscuous", "no carrier", "MTU 1500"])

    def test_ping_script_reports_each_reply(self):
        script = ping_script(["10.46.10.1", "10.46.0.1"])
        self.assertIn("-I eth1 10.46.10.1", script)
        self.assertEqual(replied("ok 10.46.10.1\nnoise\nok 10.46.0.1\n"), {"10.46.10.1", "10.46.0.1"})

    def test_outcome_keeps_streams_apart_for_parsing(self):
        outcome = Outcome(0, "x", "Warning\n")
        self.assertEqual(outcome.stdout, "x")
        self.assertIn("Warning", outcome.output)


if __name__ == "__main__":
    sys.exit(main())
