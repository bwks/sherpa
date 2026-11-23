#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.15"
# dependencies = ["pytest==9.2.0"]
# ///
"""End-to-end test of P2p links on every pair of node kinds through the sherpa CLI (Python 3.15+, pytest).

Checks the P2p link cases of tests/specs/links-and-bridges.md on VM ⟷ VM, VM ⟷ container and
container ⟷ container links: L2 transparency (raw LLDP, STP, pause, LACP, 802.1X, CDP, VLAN and
QinQ frames, plus LLDP neighbours and two-link LACP bundles between Linux bonds and cEOS), MTU
9600 and jumbo frames, host packet capture, per-direction impairment through `sherpa link
impair`, links after redeploys, `down`/`resume`, a guest reboot and, optionally, a `sherpad`
restart, and destroy leaving nothing of the lab on the host. Runs on the Sherpa server; see
README.md.

A pytest module: the `prepared` fixture checks the server and writes the lab's manifest, the
`lab` fixture brings the lab up once for the module and destroys it at the end, and the `ready`
fixture configures cEOS and addresses the Linux endpoints. Each lifecycle step (a redeploy,
down and resume, a guest reboot, a sherpad restart) is a fixture that does the step and records
what the links look like straight after it, without asserting anything; the cases assert on
those records. The cases are the `test_<case_id>_*` functions, in case order; the last destroys
the lab. Run as a script, it runs pytest on itself; `--self-test` runs the runner's own unit
tests instead.
"""

import argparse
import json
from pathlib import Path
import re
import shlex
import shutil
import signal
import struct
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
    COMMAND_TIMEOUT,
    Outcome,
    TestFailure,
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
FRAME_TOOL = LAB / "l2frames.py"
LLDP_TOOL = LAB / "lldp.py"
# Joins a container node's network namespace to drive its interfaces, and the host's to
# capture and read qdiscs. It has Python, ping, tcpdump and tc.
SIDECAR_IMAGE = "nicolaka/netshoot:v0.14"

# Every VM is Ubuntu, reached over SSH. u1 and u2 are Linux link endpoints, b1 and b2 bundle ends.
VMS = ("u1", "u2", "b1", "b2")
CONTAINERS = ("c1", "c2")
ARISTA = ("ceos1", "ceos2")
# Frame kinds l2frames.py sends; every one must arrive, from each end. The jumbo frame is
# p2p_link_003's, the others p2p_link_001's.
JUMBO_KIND = "jumbo9000"
L2_KINDS = ("lldp", "stp", "pause", "lacp", "eapol", "garp", "cdp", "vlan10", "qinq100.10")
FRAME_KINDS = (*L2_KINDS, JUMBO_KIND)
FRAME_TAGS = {"vlan10": "0x8100/10", "qinq100.10": "0x88a8/100"}
JUMBO_LENGTH = 9014
JUMBO_MTU = 9600
JUMBO_PING_SIZE = 8972
GUEST_MAC_PREFIX = "52"
# The manifest's impairment on link 0, in milliseconds each way.
MANIFEST_DELAY = 5

UP_TIMEOUT = 1800
READY_TIMEOUT = 900
POLL_INTERVAL = 10
PING_ROUNDS = 6
RECEIVE_SECONDS = 12
SENDER_START_DELAY = 4
# A capture is stopped once both directions have been exchanged; this only bounds one left behind.
CAPTURE_LIMIT_SECONDS = 600
# Extra round-trip time allowed over the configured delay.
RTT_SLACK_MS = 8.0
# How long lldp.py sends and listens on a VM's bundle ports. cEOS sends LLDP every 5 s.
LLDP_SECONDS = 12
ARISTA_LLDP_TIMER = 5
# LACP port state bits a bundled port has: synchronised, collecting and distributing.
LACP_BUNDLED = 0x38
# What a destroyed lab must not leave on the host, and how to list it. A leftover is a line that
# holds the lab's ID.
HOST_LISTINGS = (
    ("Docker containers", ["docker", "ps", "--all", "--format", "{{.Names}}"]),
    ("Docker networks", ["docker", "network", "ls", "--format", "{{.Name}}"]),
    ("host interfaces", ["ip", "-brief", "link"]),
    ("libvirt domains", ["virsh", "-c", "qemu:///system", "list", "--all", "--name"]),
    ("libvirt networks", ["virsh", "-c", "qemu:///system", "net-list", "--all", "--name"]),
)


class LinuxLink(NamedTuple):
    """A link between Linux endpoints, with the /30 its ends are addressed in (.1 src, .2 dst)."""

    pair: str
    index: int
    src: str
    src_interface: str
    dst: str
    dst_interface: str
    subnet: str

    @property
    def src_address(self):
        return f"{self.subnet}.1"

    @property
    def dst_address(self):
        return f"{self.subnet}.2"


class Bundle(NamedTuple):
    """A two-link LACP bundle: a Linux bond on a VM, a port-channel on cEOS. Ports are named as
    the node names them: the manifest's `ethN` on a VM, `EthernetN` on cEOS."""

    pair: str
    channel: int
    a: str
    a_ports: tuple
    b: str
    b_ports: tuple


class LinkFrames(NamedTuple):
    """What one frame exchange on a Linux link saw: the frames each end received from the other,
    the marks a capture on the src host end saw, and anything that failed on the way."""

    forward: dict
    back: dict
    marks: set
    errors: list


class Step(NamedTuple):
    """What the links look like straight after a lifecycle step, recorded by the step's fixture
    for the cases to assert on. Each field is a list of problems; empty is as expected."""

    pings: list
    jumbo: list
    impairment: list = []
    mtus: list = []
    frames: dict = {}
    bundles: list = []


LINUX_LINKS = (
    LinuxLink("vm-vm", 0, "u1", "eth1", "u2", "eth1", "10.47.0"),
    LinuxLink("vm-container", 1, "u1", "eth2", "c1", "eth1", "10.47.1"),
    LinuxLink("container-container", 2, "c1", "eth2", "c2", "eth1", "10.47.2"),
)
BUNDLES = (
    Bundle("vm-vm", 1, "b1", ("eth1", "eth2"), "b2", ("eth1", "eth2")),
    Bundle("vm-container", 2, "b1", ("eth3", "eth4"), "ceos1", ("Ethernet1", "Ethernet2")),
    Bundle("container-container", 3, "ceos1", ("Ethernet3", "Ethernet4"), "ceos2",
           ("Ethernet1", "Ethernet2")),
)
# u1 is the VM end of the VM ⟷ VM and VM ⟷ container links, c1 the container end of the
# VM ⟷ container and container ⟷ container links.
REDEPLOYED_VM = "u1"
REDEPLOYED_CONTAINER = "c1"
REBOOTED = "u2"


def log_outcome(log, label, arguments, outcome):
    log.write(mask(f"=== {label}: {shlex.join(arguments)}\n--- exit {outcome.code}\n"
                   f"--- stdout\n{outcome.stdout}\n--- stderr\n{outcome.stderr}\n"))
    log.flush()


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


def host_end(index, side, lab_id):
    """A link end's interface on the host: `tpa<index>-<lab>` for src, `tpb` for dst."""
    return f"tp{side}{index}-{lab_id}"


def guest_mac(tap_mac):
    """The guest MAC of a VM interface. libvirt gives the tap the guest's MAC with `fe` first."""
    return GUEST_MAC_PREFIX + tap_mac.strip().lower()[2:]


def interface_by_mac(ip_link_output, mac):
    """The interface with `mac` in `ip -o link` output, or None."""
    for line in ip_link_output.splitlines():
        fields = line.split()
        if "link/ether" in fields and fields[fields.index("link/ether") + 1].lower() == mac:
            return fields[1].rstrip(":").split("@")[0]
    return None


def received_frames(output):
    """{(sender, kind): (length, tag)} from l2frames.py receive output."""
    frames = {}
    for line in output.splitlines():
        fields = line.split()
        if len(fields) == 4:
            frames[(fields[0], fields[1])] = (int(fields[2]), fields[3])
    return frames


def frame_problems(frames, sender, kinds):
    """What is missing or wrong among the `kinds` of frame received from `sender`."""
    problems = []
    for kind in kinds:
        if (sender, kind) not in frames:
            problems.append(f"{kind} missing")
            continue
        length, tag = frames[(sender, kind)]
        if kind in FRAME_TAGS and tag != FRAME_TAGS[kind]:
            problems.append(f"{kind} tag {tag}")
        if kind == JUMBO_KIND and length != JUMBO_LENGTH:
            problems.append(f"{kind} length {length}")
    return problems


def exchange_problems(exchanges, kinds):
    """What is missing or wrong among the `kinds` of frame each end of each Linux link received."""
    problems = []
    for link in LINUX_LINKS:
        seen = exchanges[link.index]
        problems += [f"{link.pair}: {error}" for error in seen.errors]
        problems += [f"{link.pair} {link.src} -> {link.dst}: {problem}"
                     for problem in frame_problems(seen.forward, link.src, kinds)]
        problems += [f"{link.pair} {link.dst} -> {link.src}: {problem}"
                     for problem in frame_problems(seen.back, link.dst, kinds)]
    return problems


def capture_problems(exchanges, lab_id):
    """Every frame from either end that the capture on a Linux link's src host end did not see."""
    problems = []
    for link in LINUX_LINKS:
        seen = exchanges[link.index]
        problems += [f"{link.pair}: {error}" for error in seen.errors]
        problems += [f"capture on {host_end(link.index, 'a', lab_id)}: {sender} {kind} missing"
                     for sender in (link.src, link.dst) for kind in FRAME_KINDS
                     if (sender, kind) not in seen.marks]
    return problems


def pcap_marks(data):
    """{(sender, kind)} of every l2frames.py frame in a pcap file."""
    require(len(data) >= 24, "capture file is truncated")
    magic = struct.unpack("<I", data[:4])[0]
    order = {0xA1B2C3D4: "<", 0xD4C3B2A1: ">", 0xA1B23C4D: "<", 0x4D3CB2A1: ">"}.get(magic)
    require(order is not None, f"not a pcap file (magic 0x{magic:08x})")
    marks = set()
    offset = 24
    while offset + 16 <= len(data):
        captured = struct.unpack(f"{order}I", data[offset + 8:offset + 12])[0]
        packet = data[offset + 16:offset + 16 + captured]
        offset += 16 + captured
        start = packet.find(b"SHERPA391:")
        if start >= 0:
            text = packet[start + 10:].split(b"\x00")[0].decode(errors="replace")
            if text.count(":") == 1:
                marks.add(tuple(text.split(":")))
    return marks


def average_rtt(ping_output):
    """The average round-trip time in ms from ping's summary, or None."""
    match = re.search(r"= [\d.]+/([\d.]+)/", ping_output)
    return float(match.group(1)) if match else None


def netem_delay(qdisc_output):
    """The netem delay in ms from `tc qdisc show ... root`, or None when there is no netem."""
    if "netem" not in qdisc_output:
        return None
    match = re.search(r"delay ([\d.]+)(us|ms|s)\b", qdisc_output)
    if not match:
        return 0.0
    scale = {"us": 0.001, "ms": 1.0, "s": 1000.0}[match.group(2)]
    return float(match.group(1)) * scale


def lldp_neighbours(show_json):
    """{local port: (neighbour device, neighbour port)} from `show lldp neighbors | json`."""
    neighbours = {}
    for entry in json.loads(show_json).get("lldpNeighbors", []):
        device = entry.get("neighborDevice", "").split(".")[0]
        neighbours[entry.get("port", "")] = (device, entry.get("neighborPort", ""))
    return neighbours


def bundled_ports(show_json, channel):
    """The ports bundled (collecting and distributing) in a port-channel, from
    `show port-channel <n> | json`."""
    channels = json.loads(show_json).get("portChannels", {})
    active = channels.get(f"Port-Channel{channel}", {}).get("activePorts", {})
    return set(active)


def bond_bundled(proc_bonding):
    """The member ports of a Linux 802.3ad bond that are in its active aggregator, and synchronised,
    collecting and distributing at both ends, from `/proc/net/bonding/<bond>`."""
    active = re.search(r"Active Aggregator Info:\s*\n\s*Aggregator ID: (\d+)", proc_bonding)
    if not active:
        return set()
    bundled = set()
    for section in proc_bonding.split("Slave Interface: ")[1:]:
        name = section.split()[0]
        aggregator = re.search(r"^Aggregator ID: (\d+)", section, re.MULTILINE)
        states = [int(state) for state in re.findall(r"port state: (\d+)", section)]
        if (aggregator and aggregator.group(1) == active.group(1) and len(states) == 2
                and all(state & LACP_BUNDLED == LACP_BUNDLED for state in states)):
            bundled.add(name)
    return bundled


def lldp_heard(output):
    """{interface: (neighbour device, neighbour port)} from lldp.py output."""
    heard = {}
    for line in output.splitlines():
        fields = line.split()
        if len(fields) == 3:
            heard[fields[0]] = (fields[1].split(".")[0], fields[2])
    return heard


def bond_script(channel, members):
    """Shell that makes `bond<channel>` an 802.3ad bond of `members`, from scratch. A virtio
    member reports unknown speed and duplex, and the bond sends no LACPDU from a port that is
    not full duplex, so each member is set to the veths' 10000 full duplex first."""
    bond = f"bond{channel}"
    enslave = " && ".join(f"ip link set {member} down && "
                          f"ethtool -s {member} speed 10000 duplex full && "
                          f"ip link set {member} master {bond}"
                          for member in members)
    up = " && ".join(f"ip link set {member} up" for member in members)
    return (f"modprobe bonding && {{ ip link del {bond} 2>/dev/null; true; }} && "
            f"ip link add {bond} type bond mode 802.3ad lacp_rate fast miimon 100 && {enslave} && "
            f"ip link set {bond} up && {up}")


def arista_config(bundle_ports):
    """EOS configuration lines for bundles, as {channel: ports}, saved with `write memory`, which
    checks that cEOS can write its startup-config."""
    lines = ["enable", "configure", "lldp run", f"lldp timer {ARISTA_LLDP_TIMER}"]
    for channel, ports in sorted(bundle_ports.items()):
        lines += [f"interface {ports[0]}-{ports[-1].removeprefix('Ethernet')}", "no switchport",
                  f"channel-group {channel} mode active", "exit"]
    lines += ["end", "write memory"]
    return "\n".join(lines) + "\n"


def node_bundles(node):
    """{channel: ports} for one bundle node."""
    ports = {}
    for bundle in BUNDLES:
        if bundle.a == node:
            ports[bundle.channel] = bundle.a_ports
        if bundle.b == node:
            ports[bundle.channel] = bundle.b_ports
    return ports


def leftovers(listing, lab_id):
    """The first word of each line of a host listing that holds the lab's ID."""
    return [line.split()[0] for line in listing.splitlines() if lab_id in line]


def require_none(problems, stage):
    require(not problems, f"{stage}: {'; '.join(problems)}")


# -- the lab -------------------------------------------------------------------------------------


class Lab:
    """The lab every case uses: its manifest written by `prepare`, brought up once by `up` and
    destroyed by `destroy`, by the last case or the `lab` fixture's teardown."""

    def __init__(self, run_root):
        self.manifest = tomllib.loads((LAB / "manifest.toml").read_text())
        self.name = f"{self.manifest['name']}-{run_root.name[-6:]}"
        self.directory = run_root / "lab"
        self.started = False
        self.destroyed = False
        self.lab_id = None
        self.log = None
        # Expected netem delay in ms on each Linux link, per direction; None is no netem.
        self.delays = {link.index: {"src_to_dst": None, "dst_to_src": None} for link in LINUX_LINKS}
        self.delays[0] = {"src_to_dst": MANIFEST_DELAY, "dst_to_src": MANIFEST_DELAY}
        # Each VM interface's name in the guest, by manifest interface, found by MAC.
        self.guest_interfaces = {}

    # -- commands ----------------------------------------------------------------------------

    def command(self, label, arguments, timeout=COMMAND_TIMEOUT, check=True, stdin=None):
        outcome = run_logged(self.log, label, arguments, self.directory, timeout, stdin=stdin)
        if check and outcome.code != 0:
            raise TestFailure(f"{label} failed (exit {outcome.code}); see {self.log.name}")
        return outcome

    def start(self, label, arguments):
        """Start a command in the background; `finish` waits for it and logs it."""
        print(f"[{self.directory.name}] {label} (background)", flush=True)
        return subprocess.Popen(arguments, cwd=self.directory, stdin=subprocess.DEVNULL,
                                stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)

    def finish(self, label, arguments, process):
        try:
            stdout, stderr = process.communicate(timeout=COMMAND_TIMEOUT)
        except subprocess.TimeoutExpired:
            process.kill()
            stdout, stderr = process.communicate()
        outcome = Outcome(process.returncode, stdout, stderr)
        log_outcome(self.log, label, arguments, outcome)
        return outcome

    def sherpa(self, label, arguments, timeout=COMMAND_TIMEOUT, check=True):
        return self.command(label, ["sherpa", *arguments], timeout, check)

    def ssh_arguments(self, node, remote_command=None):
        """SSH to a node; with no remote command the node's CLI reads stdin."""
        arguments = ["ssh", "-F", ".sherpa/ssh/config", "-o", "BatchMode=yes", "-o", "LogLevel=ERROR",
                     f"{node}.{self.lab_id}"]
        return arguments + [remote_command] if remote_command is not None else arguments

    def ssh(self, label, node, remote_command, check=True, stdin=None):
        return self.command(label, self.ssh_arguments(node, remote_command), check=check, stdin=stdin)

    def sidecar_arguments(self, node, script):
        return ["docker", "run", "--rm", "--network", f"container:{node}-{self.lab_id}",
                "--cap-add", "NET_ADMIN", "--cap-add", "NET_RAW",
                "-v", f"{FRAME_TOOL}:/l2frames.py:ro", SIDECAR_IMAGE, "sh", "-c", script]

    def on_host(self, label, script, check=True):
        """Run a shell script in a throwaway container on the host's network namespace."""
        return self.command(label, ["docker", "run", "--rm", "--network", "host", SIDECAR_IMAGE,
                                    "sh", "-c", script], check=check)

    def endpoint_arguments(self, node, script):
        """Run `script` as root at a Linux endpoint: over SSH on a VM, in a sidecar for a container."""
        if node in VMS:
            return self.ssh_arguments(node, f"sudo sh -c {shlex.quote(script)}")
        return self.sidecar_arguments(node, script)

    def at(self, label, node, script, check=True):
        return self.command(label, self.endpoint_arguments(node, script), check=check)

    def interface(self, node, manifest_interface):
        """The interface's name at the endpoint: the manifest's in a container, by MAC in a VM."""
        if node in VMS:
            return self.guest_interfaces[(node, manifest_interface)]
        return manifest_interface

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
        """What of the lab is left on the host, by kind: containers, Docker networks, host
        interfaces and libvirt domains and networks whose name holds the lab's ID."""
        found = {}
        for kind, arguments in HOST_LISTINGS:
            left = leftovers(self.command(f"no leftover {kind}", arguments).stdout, self.lab_id)
            if left:
                found[kind] = left
        return found

    # -- Linux endpoints ---------------------------------------------------------------------

    def wait_for_ssh(self, node):
        deadline = time.monotonic() + READY_TIMEOUT
        while self.ssh(f"{node} reachable", node, "true", check=False).code != 0:
            require(time.monotonic() < deadline, f"{node} not reachable over SSH within {READY_TIMEOUT}s")
            time.sleep(POLL_INTERVAL)

    def prepare_linux(self, nodes):
        """Find VM interface names, copy the tools, then bring up and address each Linux link
        end and make each VM's bonds."""
        for node in nodes:
            if node in VMS:
                self.wait_for_ssh(node)
                self.find_guest_interfaces(node)
                self.command(f"copy tools to {node}",
                             ["scp", "-F", ".sherpa/ssh/config", "-o", "BatchMode=yes", "-o",
                              "LogLevel=ERROR", str(FRAME_TOOL), str(LLDP_TOOL),
                              f"{node}.{self.lab_id}:/tmp/"])
        for link in LINUX_LINKS:
            for node, interface, address in ((link.src, link.src_interface, link.src_address),
                                             (link.dst, link.dst_interface, link.dst_address)):
                if node in nodes:
                    name = self.interface(node, interface)
                    self.at(f"address {node} {interface}", node,
                            f"ip link set {name} up promisc on && ip link set {name} mtu {JUMBO_MTU} && "
                            f"ip addr replace {address}/30 dev {name} && ip neigh flush dev {name}")
        for node in nodes:
            if node in VMS:
                for channel, ports in sorted(node_bundles(node).items()):
                    members = [self.interface(node, port) for port in ports]
                    self.at(f"bond{channel} on {node}", node, bond_script(channel, members))

    def find_guest_interfaces(self, node):
        """Every link interface of a VM, found by the MAC of its tap. A VM cold boots on resume,
        so its bonds, which give their members the bond's MAC, are gone when this runs."""
        listing = self.ssh(f"{node} interfaces", node, "ip -o link")
        for index, link in enumerate(self.manifest["links"]):
            for side, end in (("a", link["src"]), ("b", link["dst"])):
                end_node, interface = end.split("::")
                if end_node != node:
                    continue
                tap = Path("/sys/class/net") / host_end(index, side, self.lab_id) / "address"
                mac = guest_mac(tap.read_text())
                name = interface_by_mac(listing.stdout, mac)
                require(name, f"{node} has no interface with MAC {mac} for {interface}")
                self.guest_interfaces[(node, interface)] = name

    def ping(self, node, interface, target, size=None):
        name = self.interface(node, interface)
        jumbo = f"-M do -s {size} " if size else ""
        return self.at(f"ping {node} -> {target}{' jumbo' if size else ''}", node,
                       f"ping -c 3 -i 0.2 -W 2 {jumbo}-I {name} {target}", check=False)

    def ping_failures(self, first=None, links=LINUX_LINKS):
        """The ends of the Linux links that get no reply from the other end, retrying while
        neighbours learn new MAC addresses. Pings from `first` go first: a redeployed or rebooted
        node's peers learn its MAC from them."""
        pending = []
        for link in links:
            ends = [(link.src, link.src_interface, link.dst_address),
                    (link.dst, link.dst_interface, link.src_address)]
            if link.dst == first:
                ends.reverse()
            pending += ends
        for round_number in range(PING_ROUNDS):
            pending = [end for end in pending if self.ping(*end).code != 0]
            if not pending:
                break
            time.sleep(POLL_INTERVAL * (round_number + 1) // 2)
        return [f"no reply on {node} -> {target}" for node, _, target in pending]

    def jumbo_failures(self):
        """The ends of the Linux links whose unfragmented 8972-byte pings get no reply."""
        failures = []
        for link in LINUX_LINKS:
            for node, interface, target in ((link.src, link.src_interface, link.dst_address),
                                            (link.dst, link.dst_interface, link.src_address)):
                if self.ping(node, interface, target, size=JUMBO_PING_SIZE).code != 0:
                    failures.append(f"{link.pair}: {node} -> {target} drops {JUMBO_PING_SIZE}-byte "
                                    f"unfragmented pings")
        return failures

    def host_mtu_problems(self):
        """Every host end of every link, tap or veth, that is missing or not MTU 9600."""
        problems = []
        for index in range(len(self.manifest["links"])):
            for side in ("a", "b"):
                name = host_end(index, side, self.lab_id)
                path = Path("/sys/class/net") / name / "mtu"
                if not path.exists():
                    problems.append(f"host end {name} is missing")
                    continue
                mtu = int(path.read_text())
                if mtu != JUMBO_MTU:
                    problems.append(f"host end {name} is MTU {mtu}, not {JUMBO_MTU}")
        return problems

    def frame_exchange(self, link, sender, sender_interface, receiver, receiver_interface, errors):
        """Send the frame set from `sender` and return what `receiver` got."""
        receive_arguments = self.endpoint_arguments(
            receiver, f"python3 {self.frame_tool(receiver)} receive "
                      f"{self.interface(receiver, receiver_interface)} {RECEIVE_SECONDS}")
        label = f"{link.pair}: {receiver} receives frames"
        process = self.start(label, receive_arguments)
        time.sleep(SENDER_START_DELAY)
        self.at(f"{link.pair}: {sender} sends frames", sender,
                f"python3 {self.frame_tool(sender)} send {self.interface(sender, sender_interface)} {sender}")
        outcome = self.finish(label, receive_arguments, process)
        if outcome.code != 0:
            errors.append(f"{receiver} receiving frames failed (exit {outcome.code}); see {self.log.name}")
        return received_frames(outcome.stdout)

    def frame_tool(self, node):
        return "/tmp/l2frames.py" if node in VMS else "/l2frames.py"

    def exchange_frames(self, stage):
        """{link index: LinkFrames}: the frame set sent each way on each Linux link, with a
        capture on the src host end."""
        exchanges = {}
        for link in LINUX_LINKS:
            device = host_end(link.index, "a", self.lab_id)
            capture = f"cap-{link.index}-{self.lab_id}"
            errors = []
            self.command(f"{link.pair}: capture on {device}",
                         ["docker", "run", "-d", "--name", capture, "--network", "host", "--cap-add",
                          "NET_RAW", SIDECAR_IMAGE, "timeout", str(CAPTURE_LIMIT_SECONDS), "tcpdump", "-U",
                          "-i", device, "-w", "/tmp/capture.pcap"])
            try:
                time.sleep(SENDER_START_DELAY)
                forward = self.frame_exchange(link, link.src, link.src_interface, link.dst,
                                              link.dst_interface, errors)
                back = self.frame_exchange(link, link.dst, link.dst_interface, link.src,
                                           link.src_interface, errors)
                # Stopped, not timed: slow sidecar starts must not end it before both directions.
                self.command(f"{link.pair}: capture ends", ["docker", "stop", "--timeout", "10", capture])
                pcap = self.directory / f"{stage.replace(' ', '-')}-link{link.index}.pcap"
                self.command(f"{link.pair}: copy capture", ["docker", "cp", f"{capture}:/tmp/capture.pcap",
                                                             str(pcap)])
            finally:
                self.command(f"{link.pair}: remove capture", ["docker", "rm", "-f", capture], check=False)
            exchanges[link.index] = LinkFrames(forward, back, pcap_marks(pcap.read_bytes()), errors)
        return exchanges

    # -- impairment --------------------------------------------------------------------------

    def impair(self, link, *arguments):
        self.sherpa(f"impair {link.pair} {' '.join(arguments)}",
                    ["link", "impair", f"{link.src}::{link.src_interface}", *arguments])

    def qdisc(self, link, side):
        device = host_end(link.index, side, self.lab_id)
        return self.on_host(f"qdisc on {device}", f"tc qdisc show dev {device} root").stdout

    def impairment_problems(self, links=LINUX_LINKS):
        """Where a link's host ends do not have the netem delay of the direction leaving through
        them, or the round-trip time across it is not the sum of both directions."""
        problems = []
        for link in links:
            expected = self.delays[link.index]
            # Traffic from src to dst leaves the host through the dst end, tpb.
            for side, direction in (("b", "src_to_dst"), ("a", "dst_to_src")):
                delay = netem_delay(self.qdisc(link, side))
                if delay != expected[direction]:
                    problems.append(f"{link.pair} {direction}: netem delay on "
                                    f"{host_end(link.index, side, self.lab_id)} is {delay}, "
                                    f"expected {expected[direction]}")
            total = sum(delay or 0 for delay in expected.values())
            rtt = average_rtt(self.ping(link.src, link.src_interface, link.dst_address).stdout)
            if rtt is None or not total <= rtt <= total + RTT_SLACK_MS:
                problems.append(f"{link.pair}: RTT {rtt} ms, expected {total}–{total + RTT_SLACK_MS} ms")
        return problems

    def set_delays(self, link, src_to_dst, dst_to_src):
        self.delays[link.index] = {"src_to_dst": src_to_dst, "dst_to_src": dst_to_src}

    # -- LACP bundles and LLDP --------------------------------------------------------------

    def configure_arista(self, node):
        """Configure the node's bundles, retrying while EOS is still starting."""
        config = arista_config(node_bundles(node))
        deadline = time.monotonic() + READY_TIMEOUT
        while True:
            outcome = self.ssh(f"configure {node}", node, None, check=False, stdin=config)
            if outcome.code == 0 and "% " not in outcome.output:
                return
            require(time.monotonic() < deadline, f"could not configure {node} within {READY_TIMEOUT}s")
            time.sleep(POLL_INTERVAL)

    def vm_lldp(self):
        """{node: {manifest interface: neighbour}} from lldp.py run on every bundle VM at once,
        so each hears the other while it sends."""
        running = []
        for node in sorted({end for bundle in BUNDLES for end in (bundle.a, bundle.b)} & set(VMS)):
            ports = [port for _, node_ports in sorted(node_bundles(node).items()) for port in node_ports]
            pairs = " ".join(f"{self.interface(node, port)}={port}" for port in ports)
            arguments = self.endpoint_arguments(node, f"python3 /tmp/lldp.py {LLDP_SECONDS} {node} {pairs}")
            running.append((node, arguments, self.start(f"{node} LLDP", arguments)))
        heard = {}
        for node, arguments, process in running:
            outcome = self.finish(f"{node} LLDP", arguments, process)
            names = {self.interface(node, port): port
                     for ports in node_bundles(node).values() for port in ports}
            heard[node] = {names.get(interface, interface): neighbour
                           for interface, neighbour in lldp_heard(outcome.stdout).items()}
        return heard

    def bundle_end_state(self, node, channel, vm_heard):
        """(LLDP neighbours by port, bundled ports) of one end of a bundle, or None when the
        commands failed. A VM's neighbours are what lldp.py heard, from `vm_heard`."""
        if node in ARISTA:
            lldp = self.ssh(f"{node} LLDP", node, "show lldp neighbors | json", check=False)
            port_channel = self.ssh(f"{node} Port-Channel{channel}", node,
                                    f"show port-channel {channel} | json", check=False)
            if lldp.code != 0 or port_channel.code != 0:
                return None
            return lldp_neighbours(lldp.stdout), bundled_ports(port_channel.stdout, channel)
        bond = self.at(f"{node} bond{channel}", node, f"cat /proc/net/bonding/bond{channel}", check=False)
        if bond.code != 0:
            return None
        names = {self.interface(node, port): port for port in node_bundles(node)[channel]}
        return vm_heard.get(node, {}), {names.get(name, name) for name in bond_bundled(bond.stdout)}

    def bundle_problems(self):
        vm_heard = self.vm_lldp()
        problems = []
        for bundle in BUNDLES:
            for node, ports, peer, peer_ports in ((bundle.a, bundle.a_ports, bundle.b, bundle.b_ports),
                                                  (bundle.b, bundle.b_ports, bundle.a, bundle.a_ports)):
                state = self.bundle_end_state(node, bundle.channel, vm_heard)
                if state is None:
                    problems.append(f"{node}: reading bundle {bundle.channel} failed")
                    continue
                neighbours, bundled = state
                for port, peer_port in zip(ports, peer_ports):
                    if neighbours.get(port) != (peer, peer_port):
                        problems.append(f"{node} {port}: LLDP neighbour {neighbours.get(port)}, "
                                        f"expected {peer} {peer_port}")
                if bundled != set(ports):
                    problems.append(f"{node} bundle {bundle.channel}: bundled {sorted(bundled)}")
        return problems

    def settled_bundle_problems(self):
        """What is still wrong with the bundles once LLDP and LACP have had time to settle."""
        deadline = time.monotonic() + READY_TIMEOUT
        while (problems := self.bundle_problems()) and time.monotonic() < deadline:
            time.sleep(POLL_INTERVAL)
        return problems


# -- the fixtures --------------------------------------------------------------------------------


def preflight(run_root):
    for tool in ("sherpa", "docker", "ip", "ssh", "scp", "virsh"):
        require(shutil.which(tool), f"{tool} is not on PATH")
    manifest = tomllib.loads((LAB / "manifest.toml").read_text())
    with (run_root / "preflight.log").open("w") as log:
        whoami = run_logged(log, "sherpa whoami", ["sherpa", "whoami"], run_root)
        require(whoami.code == 0, "the sherpa CLI is not logged in (run `sherpa login`)")
        require_device_password(log, run_root)
        docker = run_logged(log, f"{SIDECAR_IMAGE} present", ["docker", "image", "inspect", SIDECAR_IMAGE],
                            run_root)
        require(docker.code == 0, f"Docker has no {SIDECAR_IMAGE} image (docker pull {SIDECAR_IMAGE}), "
                f"or this user cannot use Docker; this test runs on the Sherpa server")
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
    run_root = RUNS / f"p2p-links-{uuid.uuid4().hex[:12]}"
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
def ready(lab):
    """The lab with cEOS configured, every Linux link end up and addressed and every VM's bonds
    made, and one ping round, which asserts nothing, so the ends have learnt each other's MAC
    addresses before a round-trip time is measured."""
    for node in ARISTA:
        lab.configure_arista(node)
    lab.prepare_linux(VMS + CONTAINERS)
    lab.ping_failures()
    return lab


@pytest.fixture(scope="module")
def frames_after_up(ready):
    """The frame set sent each way on every Linux link after up, with a capture on its src host end."""
    return ready.exchange_frames("after up")


@pytest.fixture(scope="module")
def lifecycle_impairment(ready):
    """The Linux links left with different impairments for the lifecycle steps: link 0 as the
    manifest has it, link 1 one direction only, link 2 none."""
    for link in LINUX_LINKS:
        ready.impair(link, "--clear")
        ready.set_delays(link, None, None)
    ready.impair(LINUX_LINKS[0], "--delay", str(MANIFEST_DELAY))
    ready.set_delays(LINUX_LINKS[0], MANIFEST_DELAY, MANIFEST_DELAY)
    ready.impair(LINUX_LINKS[1], "--direction", "dst-to-src", "--delay", "15")
    ready.set_delays(LINUX_LINKS[1], None, 15)
    return ready


def redeployed_step(lab, node):
    """Redeploy a node, address it again and record its links and their impairments."""
    lab.redeploy(node)
    lab.prepare_linux((node,))
    return Step(pings=lab.ping_failures(first=node), jumbo=lab.jumbo_failures(),
                impairment=lab.impairment_problems())


@pytest.fixture(scope="module")
def redeployed_vm(lifecycle_impairment):
    """The VM end of two links redeployed, and its links and their impairments recorded."""
    return redeployed_step(lifecycle_impairment, REDEPLOYED_VM)


@pytest.fixture(scope="module")
def redeployed_container(lifecycle_impairment):
    """The container end of two links redeployed, and its links and their impairments recorded."""
    return redeployed_step(lifecycle_impairment, REDEPLOYED_CONTAINER)


@pytest.fixture(scope="module")
def resumed(lifecycle_impairment):
    """The lab down and resumed and, the VMs having cold booted, their link ends addressed and
    bonds made again; then everything checked after up recorded."""
    lab = lifecycle_impairment
    lab.sherpa("sherpa down", ["down"], UP_TIMEOUT)
    lab.sherpa("sherpa resume", ["resume"], UP_TIMEOUT)
    lab.prepare_linux(VMS)
    return Step(mtus=lab.host_mtu_problems(), pings=lab.ping_failures(), jumbo=lab.jumbo_failures(),
                frames=lab.exchange_frames("after resume"), bundles=lab.settled_bundle_problems(),
                impairment=lab.impairment_problems())


@pytest.fixture(scope="module")
def rebooted(ready):
    """A VM rebooted from inside its guest, addressed again, and its links recorded."""
    lab = ready
    lab.ssh(f"reboot {REBOOTED}", REBOOTED, "sudo systemctl reboot", check=False)
    time.sleep(POLL_INTERVAL)
    lab.prepare_linux((REBOOTED,))
    return Step(pings=lab.ping_failures(first=REBOOTED), jumbo=lab.jumbo_failures())


@pytest.fixture(scope="module")
def sherpad_restarted(request, lifecycle_impairment):
    """sherpad restarted with `--sherpad-restart`'s command, and the links, bundles and
    impairments recorded; None when the option is not given."""
    restart = request.config.getoption("sherpad_restart")
    if not restart:
        return None
    lab = lifecycle_impairment
    lab.command("restart sherpad", shlex.split(restart))
    time.sleep(POLL_INTERVAL)
    return Step(pings=lab.ping_failures(), jumbo=lab.jumbo_failures(),
                bundles=lab.settled_bundle_problems(), impairment=lab.impairment_problems())


# -- cases ---------------------------------------------------------------------------------------


def test_p2p_link_001_l2_control_and_tagged_frames_cross_every_kind_of_link_each_way(frames_after_up):
    require_none(exchange_problems(frames_after_up, L2_KINDS), "after up")


def test_p2p_link_002_two_linked_ends_are_lldp_neighbours_and_bundle_both_links_with_lacp(ready):
    require_none(ready.settled_bundle_problems(), "after up")


def test_p2p_link_003_every_host_end_is_mtu_9600_and_jumbo_pings_and_frames_pass_each_way(ready, frames_after_up):
    problems = ready.host_mtu_problems() + ready.ping_failures() + ready.jumbo_failures()
    problems += exchange_problems(frames_after_up, (JUMBO_KIND,))
    require_none(problems, "after up")


def test_p2p_link_005_a_capture_on_a_host_end_sees_every_frame_from_both_ends(ready, frames_after_up):
    require_none(capture_problems(frames_after_up, ready.lab_id), "after up")


def test_link_impair_001_the_manifest_impairment_is_on_both_directions_after_up(ready):
    require_none(ready.impairment_problems(), "after up")


def test_link_impair_002_a_direction_delays_only_the_traffic_going_that_way(ready):
    problems = []
    for link in LINUX_LINKS:
        before = ready.delays[link.index]["dst_to_src"]
        ready.impair(link, "--direction", "src-to-dst", "--delay", "20")
        ready.set_delays(link, 20, before)
        problems += [f"src-to-dst 20 ms, {problem}" for problem in ready.impairment_problems((link,))]
        ready.impair(link, "--direction", "dst-to-src", "--delay", "10")
        ready.set_delays(link, 20, 10)
        problems += [f"dst-to-src 10 ms, {problem}" for problem in ready.impairment_problems((link,))]
    require_none(problems, "sherpa link impair --direction")


def test_link_impair_003_reorder_and_corruption_on_both_directions_are_set_and_traffic_still_passes(ready):
    problems = []
    for link in LINUX_LINKS:
        ready.impair(link, "--delay", "10", "--reorder", "25", "--corrupt", "5")
        ready.set_delays(link, 10, 10)
        for side in ("a", "b"):
            qdisc = ready.qdisc(link, side)
            if "reorder 25%" not in qdisc or "corrupt 5%" not in qdisc:
                problems.append(f"{link.pair}: {host_end(link.index, side, ready.lab_id)} lacks reorder "
                                f"or corruption: {qdisc.strip()}")
        problems += [f"{link.pair}: {failure}" for failure in ready.ping_failures(links=(link,))]
    require_none(problems, "reorder 25% and corrupt 5%")


def test_link_impair_004_clear_removes_the_impairment_and_the_rtt_is_as_without_one(ready):
    problems = []
    for link in LINUX_LINKS:
        ready.impair(link, "--clear")
        ready.set_delays(link, None, None)
        problems += ready.impairment_problems((link,))
    require_none(problems, "after --clear")


def test_link_reattach_001_after_redeploying_a_vm_ending_two_links_pings_pass_each_way(redeployed_vm):
    require_none(redeployed_vm.pings + redeployed_vm.jumbo, f"after redeploying {REDEPLOYED_VM}")


def test_link_reattach_002_after_redeploying_a_container_ending_two_links_pings_pass_each_way(redeployed_container):
    require_none(redeployed_container.pings + redeployed_container.jumbo,
                 f"after redeploying {REDEPLOYED_CONTAINER}")


def test_link_reattach_003_after_down_and_resume_pings_frames_capture_and_bundles_work_as_after_up(lab, resumed):
    problems = resumed.mtus + resumed.pings + resumed.jumbo
    problems += exchange_problems(resumed.frames, FRAME_KINDS)
    problems += capture_problems(resumed.frames, lab.lab_id)
    problems += resumed.bundles
    require_none(problems, "after resume")


def test_link_reattach_004_a_reboot_from_inside_a_vm_guest_leaves_its_links_working(rebooted):
    require_none(rebooted.pings + rebooted.jumbo, f"after rebooting {REBOOTED} from inside the guest")


def test_p2p_link_009_links_carry_traffic_across_a_sherpad_restart(sherpad_restarted):
    if sherpad_restarted is None:
        pytest.skip("give --sherpad-restart COMMAND to restart sherpad")
    require_none(sherpad_restarted.pings + sherpad_restarted.jumbo + sherpad_restarted.bundles,
                 "after a sherpad restart")


def test_link_impair_005_an_impairment_stays_across_redeploys_down_and_resume_and_a_sherpad_restart(
        redeployed_vm, redeployed_container, resumed, sherpad_restarted):
    steps = {f"after redeploying {REDEPLOYED_VM}": redeployed_vm,
             f"after redeploying {REDEPLOYED_CONTAINER}": redeployed_container,
             "after resume": resumed}
    if sherpad_restarted is not None:
        steps["after a sherpad restart"] = sherpad_restarted
    problems = [f"{stage}, {problem}" for stage, step in steps.items() for problem in step.impairment]
    require_none(problems, "impairment not as set")
    if sherpad_restarted is None:
        pytest.skip("not checked after a sherpad restart: give --sherpad-restart COMMAND")


def test_link_cleanup_001_destroying_a_lab_of_vm_and_container_links_leaves_nothing_of_it(lab, request):
    if keep_failed_lab(request):
        pytest.skip("--keep-failed: a case failed, so the lab is kept")
    lab.destroy()
    found = lab.host_leftovers()
    require(not found, "left after destroy: " + "; ".join(f"{kind} {names}" for kind, names in found.items()))


# -- running -------------------------------------------------------------------------------------


def main():
    parser = argparse.ArgumentParser(description=__doc__,
                                     formatter_class=argparse.RawDescriptionHelpFormatter,
                                     epilog="Other arguments, such as --keep-failed, --sherpad-restart "
                                            "COMMAND or -k, are passed to pytest.")
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

    def test_a_partly_created_lab_keeps_its_id_for_cleanup(self):
        # `sherpa up` exits 1 on a partly created lab after writing lab-info.toml; the run must
        # still know the lab's ID, so destroy checks the lab is gone instead of skipping it.
        with tempfile.TemporaryDirectory() as temp:
            lab = Lab(Path(temp) / "p2p-links-000000abcdef")
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

    def test_a_leftover_is_a_listed_name_holding_the_lab_id(self):
        listing = "tpa0-4ed6360e UP\nlo UNKNOWN\nc1-4ed6360e\n"
        self.assertEqual(leftovers(listing, "4ed6360e"), ["tpa0-4ed6360e", "c1-4ed6360e"])
        self.assertEqual(leftovers("lo UNKNOWN\n", "4ed6360e"), [])

    def test_bond_bundled_needs_the_active_aggregator_and_both_ends_in_sync(self):
        def member(name, aggregator, actor, partner):
            return (f"Slave Interface: {name}\nMII Status: up\nAggregator ID: {aggregator}\n"
                    f"details actor lacp pdu:\n    port state: {actor}\n"
                    f"details partner lacp pdu:\n    port state: {partner}\n\n")

        head = ("Bonding Mode: IEEE 802.3ad Dynamic link aggregation\n\n802.3ad info\n"
                "Active Aggregator Info:\n\tAggregator ID: 1\n\tNumber of ports: 2\n\n")
        both = head + member("enp3s0", 1, 63, 63) + member("enp4s0", 1, 61, 63)
        self.assertEqual(bond_bundled(both), {"enp3s0", "enp4s0"})
        self.assertEqual(bond_bundled(head + member("enp3s0", 1, 63, 63) + member("enp4s0", 2, 63, 63)),
                         {"enp3s0"})
        # Partner not yet collecting and distributing.
        self.assertEqual(bond_bundled(head + member("enp3s0", 1, 63, 7)), set())
        self.assertEqual(bond_bundled("Bonding Mode: IEEE 802.3ad\n"), set())

    def test_guest_mac_and_interface_lookup(self):
        mac = guest_mac("fe:54:00:b6:39:ad\n")
        self.assertEqual(mac, "52:54:00:b6:39:ad")
        listing = ("2: enp3s0: <BROADCAST> mtu 1500 qdisc fq_codel state UP\\    link/ether "
                   "52:54:00:aa:aa:aa brd ff:ff:ff:ff:ff:ff\n"
                   "3: enp4s0: <BROADCAST> mtu 1500 qdisc noop state DOWN\\    link/ether "
                   "52:54:00:b6:39:ad brd ff:ff:ff:ff:ff:ff\n")
        self.assertEqual(interface_by_mac(listing, mac), "enp4s0")
        self.assertIsNone(interface_by_mac(listing, "52:54:00:00:00:00"))

    def test_frame_problems_report_missing_and_mangled_frames(self):
        frames = {("u1", kind): (60, FRAME_TAGS.get(kind, "-")) for kind in FRAME_KINDS}
        frames[("u1", JUMBO_KIND)] = (JUMBO_LENGTH, "-")
        self.assertEqual(frame_problems(frames, "u1", FRAME_KINDS), [])
        del frames[("u1", "pause")]
        frames[("u1", "vlan10")] = (60, "-")
        frames[("u1", JUMBO_KIND)] = (1514, "-")
        self.assertEqual(frame_problems(frames, "u1", L2_KINDS), ["pause missing", "vlan10 tag -"])
        self.assertEqual(frame_problems(frames, "u1", (JUMBO_KIND,)), ["jumbo9000 length 1514"])
        self.assertEqual(received_frames("u1 lldp 32 -\nnoise\n"), {("u1", "lldp"): (32, "-")})

    def test_a_failed_exchange_or_a_frame_the_capture_missed_is_a_problem(self):
        whole = {(sender, kind) for link in LINUX_LINKS for sender in (link.src, link.dst)
                 for kind in FRAME_KINDS}
        exchanges = {link.index: LinkFrames({}, {}, set(whole), []) for link in LINUX_LINKS}
        self.assertEqual(capture_problems(exchanges, "4ed6360e"), [])
        exchanges[2] = LinkFrames({}, {}, whole - {("c2", "lldp")}, ["c1 receiving frames failed"])
        self.assertEqual(capture_problems(exchanges, "4ed6360e"),
                         ["container-container: c1 receiving frames failed",
                          "capture on tpa2-4ed6360e: c2 lldp missing"])

    def test_ping_and_qdisc_parsing(self):
        self.assertEqual(average_rtt("rtt min/avg/max/mdev = 20.1/25.5/30.0/1.0 ms"), 25.5)
        self.assertIsNone(average_rtt("3 packets transmitted, 0 received"))
        self.assertEqual(netem_delay("qdisc netem 1: root refcnt 2 limit 1000 delay 20ms seed 1"), 20.0)
        self.assertEqual(netem_delay("qdisc netem 1: root limit 1000 delay 500us"), 0.5)
        self.assertIsNone(netem_delay("qdisc noqueue 0: root refcnt 2"))

    def test_port_channel_parsing(self):
        channel = json.dumps({"portChannels": {"Port-Channel1": {"activePorts": {"Ethernet1": {},
                                                                                 "Ethernet2": {}}}}})
        self.assertEqual(bundled_ports(channel, 1), {"Ethernet1", "Ethernet2"})
        self.assertEqual(bundled_ports(channel, 2), set())


if __name__ == "__main__":
    sys.exit(main())
