#!/usr/bin/env python3
"""End-to-end test of P2p links on every pair of node kinds through the sherpa CLI (Python 3.11+, stdlib only).

Checks the #391 requirements for eBPF links on VM ⟷ VM, VM ⟷ container and container ⟷ container
links: L2 transparency (raw LLDP, STP, pause, LACP, 802.1X, CDP, VLAN, QinQ and jumbo frames,
plus Arista LLDP neighbours and LACP bundles), host packet capture, per-direction impairment
through `sherpa link impair`, MTU 9600, and connectivity after `up`, redeploys, a guest reboot,
`down`/`resume` and, optionally, a `sherpad` restart. Always destroys the lab and checks nothing
is left. Runs on the Sherpa server; see README.md.
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
import time
import tomllib
from typing import NamedTuple
import unittest
import uuid


LAB = Path(__file__).resolve().parent
ROOT = LAB.parent.parent.parent
RUNS = ROOT / ".tmp" / "lab-tests"
FRAME_TOOL = LAB / "l2frames.py"
# Joins a container node's network namespace to drive its interfaces, and the host's to
# capture and read qdiscs. It has Python, ping, tcpdump and tc.
SIDECAR_IMAGE = "nicolaka/netshoot:v0.14"

VMS = ("u1", "u2")
CONTAINERS = ("c1", "c2")
ARISTA = ("veos1", "veos2", "ceos1", "ceos2")
# Frame kinds l2frames.py sends; every one must arrive, from each end.
FRAME_KINDS = ("lldp", "stp", "pause", "lacp", "eapol", "garp", "cdp", "vlan10", "qinq100.10",
               "jumbo9000")
FRAME_TAGS = {"vlan10": "0x8100/10", "qinq100.10": "0x88a8/100"}
JUMBO_LENGTH = 9014
JUMBO_MTU = 9600
JUMBO_PING_SIZE = 8972
GUEST_MAC_PREFIX = "52"
# The manifest's impairment on link 0, in milliseconds each way.
MANIFEST_DELAY = 5

COMMAND_TIMEOUT = 180
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
    """A two-link LACP bundle between Arista nodes."""

    pair: str
    channel: int
    a: str
    a_ports: tuple
    b: str
    b_ports: tuple


LINUX_LINKS = (
    LinuxLink("vm-vm", 0, "u1", "eth1", "u2", "eth1", "10.47.0"),
    LinuxLink("vm-container", 1, "u1", "eth2", "c1", "eth1", "10.47.1"),
    LinuxLink("container-container", 2, "c1", "eth2", "c2", "eth1", "10.47.2"),
)
BUNDLES = (
    Bundle("vm-vm", 1, "veos1", ("Ethernet1", "Ethernet2"), "veos2", ("Ethernet1", "Ethernet2")),
    Bundle("vm-container", 2, "veos1", ("Ethernet3", "Ethernet4"), "ceos1", ("Ethernet1", "Ethernet2")),
    Bundle("container-container", 3, "ceos1", ("Ethernet3", "Ethernet4"), "ceos2",
           ("Ethernet1", "Ethernet2")),
)
# Redeployed in this order: u1 is the VM end of the VM ⟷ VM and VM ⟷ container links, c1 the
# container end of the VM ⟷ container and container ⟷ container links.
REDEPLOYED = ("u1", "c1")
REBOOTED = "u2"


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
    log_outcome(log, label, arguments, outcome)
    return outcome


def log_outcome(log, label, arguments, outcome):
    log.write(f"=== {label}: {shlex.join(arguments)}\n--- exit {outcome.code}\n"
              f"--- stdout\n{outcome.stdout}\n--- stderr\n{outcome.stderr}\n")
    log.flush()


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


def frame_problems(frames, sender):
    """What is missing or wrong in the frames received from `sender`."""
    problems = []
    for kind in FRAME_KINDS:
        if (sender, kind) not in frames:
            problems.append(f"{kind} missing")
            continue
        length, tag = frames[(sender, kind)]
        if kind in FRAME_TAGS and tag != FRAME_TAGS[kind]:
            problems.append(f"{kind} tag {tag}")
        if kind == "jumbo9000" and length != JUMBO_LENGTH:
            problems.append(f"{kind} length {length}")
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


def arista_config(bundle_ports):
    """EOS configuration lines for bundles, as {channel: ports}, saved with `write memory`. A vEOS
    VM cold boots on resume, so it needs the saved configuration; on cEOS the save checks that
    its startup-config can be written."""
    lines = ["enable", "configure", "lldp run"]
    for channel, ports in sorted(bundle_ports.items()):
        lines += [f"interface {ports[0]}-{ports[-1].removeprefix('Ethernet')}", "no switchport",
                  f"channel-group {channel} mode active", "exit"]
    lines += ["end", "write memory"]
    return "\n".join(lines) + "\n"


def arista_bundles(node):
    """{channel: ports} for one Arista node."""
    ports = {}
    for bundle in BUNDLES:
        if bundle.a == node:
            ports[bundle.channel] = bundle.a_ports
        if bundle.b == node:
            ports[bundle.channel] = bundle.b_ports
    return ports


class LabRun:
    def __init__(self, run_root, keep_failed, sherpad_restart):
        self.manifest = tomllib.loads((LAB / "manifest.toml").read_text())
        self.name = f"{self.manifest['name']}-{run_root.name[-6:]}"
        self.directory = run_root / "lab"
        self.keep_failed = keep_failed
        self.sherpad_restart = sherpad_restart
        self.started = False
        self.lab_id = None
        self.log = None
        # Expected netem delay in ms on each Linux link, per direction; None is no netem.
        self.delays = {link.index: {"src_to_dst": None, "dst_to_src": None} for link in LINUX_LINKS}
        self.delays[0] = {"src_to_dst": MANIFEST_DELAY, "dst_to_src": MANIFEST_DELAY}
        # Each VM interface's name in the guest, by manifest interface, found by MAC.
        self.guest_interfaces = {}

    # -- commands ----------------------------------------------------------------------------

    def command(self, label, arguments, timeout=COMMAND_TIMEOUT, check=True, stdin=None):
        outcome = run_logged(self.log, label, arguments, self.directory, timeout, stdin)
        if check and outcome.code != 0:
            raise TestFailure(f"{label} failed (exit {outcome.code}); see {self.log.name}")
        return outcome

    def start(self, label, arguments):
        """Start a command in the background; `finish` waits for it and logs it."""
        print(f"[{self.directory.name}] {label} (background)", flush=True)
        return subprocess.Popen(arguments, cwd=self.directory, stdout=subprocess.PIPE,
                                stderr=subprocess.PIPE, text=True)

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
        arguments = ["ssh", "-F", "sherpa_ssh_config", "-o", "BatchMode=yes", "-o", "LogLevel=ERROR",
                     f"{node}.{self.lab_id}"]
        return arguments + [remote_command] if remote_command is not None else arguments

    def ssh(self, label, node, remote_command, check=True, stdin=None):
        return self.command(label, self.ssh_arguments(node, remote_command), check=check, stdin=stdin)

    def sidecar_arguments(self, node, script):
        return ["docker", "run", "--rm", "--network", f"container:{node}-{self.lab_id}",
                "--cap-add", "NET_ADMIN", "--cap-add", "NET_RAW",
                "-v", f"{FRAME_TOOL}:/l2frames.py:ro", SIDECAR_IMAGE, "sh", "-c", script]

    def sidecar(self, label, node, script, check=True):
        """Run a shell script as root in a throwaway container sharing the node's network namespace."""
        return self.command(label, self.sidecar_arguments(node, script), check=check)

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
        for node in ARISTA:
            self.configure_arista(node)
        self.prepare_linux(VMS + CONTAINERS)
        self.check_host_mtus()
        self.check_connectivity("after up")
        self.check_frames_and_capture("after up")
        self.check_arista("after up")
        self.check_impairment("after up")
        self.change_impairment()

        for node in REDEPLOYED:
            self.sherpa(f"sherpa redeploy {node}", ["redeploy", "--node", node], UP_TIMEOUT)
            self.prepare_linux((node,))
            self.check_connectivity(f"after redeploying {node}", first=node)
            self.check_impairment(f"after redeploying {node}")

        self.ssh(f"reboot {REBOOTED}", REBOOTED, "sudo systemctl reboot", check=False)
        time.sleep(POLL_INTERVAL)
        self.prepare_linux((REBOOTED,))
        self.check_connectivity(f"after rebooting {REBOOTED}", first=REBOOTED)

        self.sherpa("sherpa down", ["down"], UP_TIMEOUT)
        self.sherpa("sherpa resume", ["resume"], UP_TIMEOUT)
        self.prepare_linux(VMS)
        self.check_host_mtus()
        self.check_connectivity("after resume")
        self.check_frames_and_capture("after resume")
        self.check_arista("after resume")
        self.check_impairment("after resume")

        if self.sherpad_restart:
            self.command("restart sherpad", shlex.split(self.sherpad_restart))
            time.sleep(POLL_INTERVAL)
            self.check_connectivity("after a sherpad restart")
            self.check_arista("after a sherpad restart")
            self.check_impairment("after a sherpad restart")

    # -- Linux endpoints ---------------------------------------------------------------------

    def wait_for_ssh(self, node):
        deadline = time.monotonic() + READY_TIMEOUT
        while self.ssh(f"{node} reachable", node, "true", check=False).code != 0:
            require(time.monotonic() < deadline, f"{node} not reachable over SSH within {READY_TIMEOUT}s")
            time.sleep(POLL_INTERVAL)

    def prepare_linux(self, nodes):
        """Find VM interface names, copy the frame tool, then bring up and address each end."""
        for node in nodes:
            if node in VMS:
                self.wait_for_ssh(node)
                self.find_guest_interfaces(node)
                self.command(f"copy frame tool to {node}",
                             ["scp", "-F", "sherpa_ssh_config", "-o", "BatchMode=yes", "-o",
                              "LogLevel=ERROR", str(FRAME_TOOL), f"{node}.{self.lab_id}:/tmp/l2frames.py"])
        for link in LINUX_LINKS:
            for node, interface, address in ((link.src, link.src_interface, link.src_address),
                                             (link.dst, link.dst_interface, link.dst_address)):
                if node in nodes:
                    name = self.interface(node, interface)
                    self.at(f"address {node} {interface}", node,
                            f"ip link set {name} up promisc on && ip link set {name} mtu {JUMBO_MTU} && "
                            f"ip addr replace {address}/30 dev {name} && ip neigh flush dev {name}")

    def find_guest_interfaces(self, node):
        listing = self.ssh(f"{node} interfaces", node, "ip -o link")
        for link in LINUX_LINKS:
            for side, end, interface in (("a", link.src, link.src_interface),
                                         ("b", link.dst, link.dst_interface)):
                if end != node:
                    continue
                tap = Path("/sys/class/net") / host_end(link.index, side, self.lab_id) / "address"
                mac = guest_mac(tap.read_text())
                name = interface_by_mac(listing.stdout, mac)
                require(name, f"{node} has no interface with MAC {mac} for {interface}")
                self.guest_interfaces[(node, interface)] = name

    def ping(self, node, interface, target, size=None):
        name = self.interface(node, interface)
        jumbo = f"-M do -s {size} " if size else ""
        return self.at(f"ping {node} -> {target}{' jumbo' if size else ''}", node,
                       f"ping -c 3 -i 0.2 -W 2 {jumbo}-I {name} {target}", check=False)

    def check_connectivity(self, stage, first=None):
        """Each Linux link passes traffic both ways, jumbo frames included. Pings from `first`
        go first: a redeployed or rebooted node's peers learn its MAC from them."""
        pending = []
        for link in LINUX_LINKS:
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
        require(not pending, f"{stage}, no reply on: "
                f"{'; '.join(f'{node} -> {target}' for node, _, target in pending)}")
        for link in LINUX_LINKS:
            for node, interface, target in ((link.src, link.src_interface, link.dst_address),
                                            (link.dst, link.dst_interface, link.src_address)):
                outcome = self.ping(node, interface, target, size=JUMBO_PING_SIZE)
                require(outcome.code == 0, f"{stage}, {link.pair}: {node} -> {target} drops "
                        f"{JUMBO_PING_SIZE}-byte unfragmented pings")

    def check_host_mtus(self):
        """Every host end of every link, tap or veth, is MTU 9600."""
        for index in range(len(self.manifest["links"])):
            for side in ("a", "b"):
                name = host_end(index, side, self.lab_id)
                path = Path("/sys/class/net") / name / "mtu"
                require(path.exists(), f"host end {name} is missing")
                mtu = int(path.read_text())
                require(mtu == JUMBO_MTU, f"host end {name} is MTU {mtu}, not {JUMBO_MTU}")

    def frame_exchange(self, link, sender, sender_interface, receiver, receiver_interface):
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
        require(outcome.code == 0, f"{label} failed (exit {outcome.code}); see {self.log.name}")
        return received_frames(outcome.stdout)

    def frame_tool(self, node):
        return "/tmp/l2frames.py" if node in VMS else "/l2frames.py"

    def check_frames_and_capture(self, stage):
        """Every frame kind crosses each Linux link both ways, and a capture on the src host end
        sees all of them from both ends."""
        for link in LINUX_LINKS:
            device = host_end(link.index, "a", self.lab_id)
            capture = f"cap-{link.index}-{self.lab_id}"
            self.command(f"{link.pair}: capture on {device}",
                         ["docker", "run", "-d", "--name", capture, "--network", "host", "--cap-add",
                          "NET_RAW", SIDECAR_IMAGE, "timeout", str(CAPTURE_LIMIT_SECONDS), "tcpdump", "-U",
                          "-i", device, "-w", "/tmp/capture.pcap"])
            try:
                time.sleep(SENDER_START_DELAY)
                forward = self.frame_exchange(link, link.src, link.src_interface, link.dst,
                                              link.dst_interface)
                back = self.frame_exchange(link, link.dst, link.dst_interface, link.src,
                                           link.src_interface)
                # Stopped, not timed: slow sidecar starts must not end it before both directions.
                self.command(f"{link.pair}: capture ends", ["docker", "stop", "--timeout", "10", capture])
                pcap = self.directory / f"{stage.replace(' ', '-')}-link{link.index}.pcap"
                self.command(f"{link.pair}: copy capture", ["docker", "cp", f"{capture}:/tmp/capture.pcap",
                                                             str(pcap)])
            finally:
                self.command(f"{link.pair}: remove capture", ["docker", "rm", "-f", capture], check=False)
            problems = [f"{link.src} -> {link.dst}: {problem}" for problem in frame_problems(forward, link.src)]
            problems += [f"{link.dst} -> {link.src}: {problem}" for problem in frame_problems(back, link.dst)]
            marks = pcap_marks(pcap.read_bytes())
            problems += [f"capture on {device}: {sender} {kind} missing"
                         for sender in (link.src, link.dst) for kind in FRAME_KINDS
                         if (sender, kind) not in marks]
            require(not problems, f"{stage}, {link.pair}: {'; '.join(problems)}")

    # -- impairment --------------------------------------------------------------------------

    def impair(self, link, *arguments):
        self.sherpa(f"impair {link.pair} {' '.join(arguments)}",
                    ["link", "impair", f"{link.src}::{link.src_interface}", *arguments])

    def check_impairment(self, stage):
        """Each host end has the netem delay of the direction leaving through it, and the RTT
        across each Linux link is the sum of both directions."""
        for link in LINUX_LINKS:
            expected = self.delays[link.index]
            # Traffic from src to dst leaves the host through the dst end, tpb.
            for side, direction in (("b", "src_to_dst"), ("a", "dst_to_src")):
                device = host_end(link.index, side, self.lab_id)
                outcome = self.on_host(f"qdisc on {device}", f"tc qdisc show dev {device} root")
                delay = netem_delay(outcome.stdout)
                require(delay == expected[direction],
                        f"{stage}, {link.pair} {direction}: netem delay on {device} is {delay}, "
                        f"expected {expected[direction]}")
            self.check_rtt(stage, link)

    def check_rtt(self, stage, link):
        expected = sum(delay or 0 for delay in self.delays[link.index].values())
        outcome = self.ping(link.src, link.src_interface, link.dst_address)
        rtt = average_rtt(outcome.stdout)
        require(rtt is not None and expected <= rtt <= expected + RTT_SLACK_MS,
                f"{stage}, {link.pair}: RTT {rtt} ms, expected {expected}–{expected + RTT_SLACK_MS} ms")

    def change_impairment(self):
        """Set each direction on its own, then both with reorder and corruption, on every Linux
        link. Leaves each link in a different state for the lifecycle checks."""
        for link in LINUX_LINKS:
            self.impair(link, "--direction", "src-to-dst", "--delay", "20")
            self.delays[link.index]["src_to_dst"] = 20
            self.check_impairment(f"{link.pair} src-to-dst 20 ms")
            self.impair(link, "--direction", "dst-to-src", "--delay", "10")
            self.delays[link.index]["dst_to_src"] = 10
            self.check_impairment(f"{link.pair} dst-to-src 10 ms")

            self.impair(link, "--delay", "10", "--reorder", "25", "--corrupt", "5")
            for side in ("a", "b"):
                device = host_end(link.index, side, self.lab_id)
                qdisc = self.on_host(f"qdisc on {device}", f"tc qdisc show dev {device} root").stdout
                require("reorder 25%" in qdisc and "corrupt 5%" in qdisc,
                        f"{link.pair}: {device} lacks reorder or corruption: {qdisc.strip()}")
            self.impair(link, "--clear")
            self.delays[link.index] = {"src_to_dst": None, "dst_to_src": None}
            self.check_impairment(f"{link.pair} cleared")

        # Left for the lifecycle checks: link 0 as the manifest had it, link 1 one direction
        # only, link 2 clear.
        self.impair(LINUX_LINKS[0], "--delay", str(MANIFEST_DELAY))
        self.delays[0] = {"src_to_dst": MANIFEST_DELAY, "dst_to_src": MANIFEST_DELAY}
        self.impair(LINUX_LINKS[1], "--direction", "dst-to-src", "--delay", "15")
        self.delays[1] = {"src_to_dst": None, "dst_to_src": 15}
        self.check_impairment("after setting impairment for the lifecycle checks")

    # -- Arista ------------------------------------------------------------------------------

    def configure_arista(self, node):
        """Configure the node's bundles, retrying while EOS is still starting."""
        config = arista_config(arista_bundles(node))
        deadline = time.monotonic() + READY_TIMEOUT
        while True:
            outcome = self.ssh(f"configure {node}", node, None, check=False, stdin=config)
            if outcome.code == 0 and "% " not in outcome.output:
                return
            require(time.monotonic() < deadline, f"could not configure {node} within {READY_TIMEOUT}s")
            time.sleep(POLL_INTERVAL)

    def arista_problems(self):
        problems = []
        for bundle in BUNDLES:
            for node, ports, peer, peer_ports in ((bundle.a, bundle.a_ports, bundle.b, bundle.b_ports),
                                                  (bundle.b, bundle.b_ports, bundle.a, bundle.a_ports)):
                lldp = self.ssh(f"{node} LLDP", node, "show lldp neighbors | json", check=False)
                channel = self.ssh(f"{node} Port-Channel{bundle.channel}", node,
                                   f"show port-channel {bundle.channel} | json", check=False)
                if lldp.code != 0 or channel.code != 0:
                    problems.append(f"{node}: show commands failed")
                    continue
                neighbours = lldp_neighbours(lldp.stdout)
                for port, peer_port in zip(ports, peer_ports):
                    if neighbours.get(port) != (peer, peer_port):
                        problems.append(f"{node} {port}: LLDP neighbour {neighbours.get(port)}, "
                                        f"expected {peer} {peer_port}")
                bundled = bundled_ports(channel.stdout, bundle.channel)
                if bundled != set(ports):
                    problems.append(f"{node} Port-Channel{bundle.channel}: bundled {sorted(bundled)}")
        return problems

    def check_arista(self, stage):
        """Each bundle's ends see each other over LLDP and bundle both links, waiting for LACP."""
        deadline = time.monotonic() + READY_TIMEOUT
        while problems := self.arista_problems():
            require(time.monotonic() < deadline, f"{stage}: {'; '.join(problems)}")
            time.sleep(POLL_INTERVAL)

    # -- destroy -----------------------------------------------------------------------------

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
            ("libvirt domains", ["virsh", "-c", "qemu:///system", "list", "--all", "--name"]),
        )
        for kind, arguments in checks:
            listing = self.command(f"no leftover {kind}", arguments)
            leftovers = [line.split()[0] for line in listing.stdout.splitlines() if self.lab_id in line]
            require(not leftovers, f"{kind} left after destroy: {leftovers}")


def preflight(run_root):
    for tool in ("sherpa", "docker", "ip", "ssh", "scp", "virsh"):
        require(shutil.which(tool), f"{tool} is not on PATH")
    manifest = tomllib.loads((LAB / "manifest.toml").read_text())
    with (run_root / "preflight.log").open("w") as log:
        whoami = run_logged(log, "sherpa whoami", ["sherpa", "whoami"], run_root)
        require(whoami.code == 0, "the sherpa CLI is not logged in (run `sherpa login`)")
        docker = run_logged(log, f"{SIDECAR_IMAGE} present", ["docker", "image", "inspect", SIDECAR_IMAGE],
                            run_root)
        require(docker.code == 0, f"Docker has no {SIDECAR_IMAGE} image (docker pull {SIDECAR_IMAGE}), "
                f"or this user cannot use Docker; this test runs on the Sherpa server")
        images = sorted({(node["model"], node["version"]) for node in manifest["nodes"]})
        for model, version in images:
            image = run_logged(log, f"{model} {version} image present",
                               ["sherpa", "image", "show", model, "--version", version], run_root)
            require(image.code == 0, f"the server has no {model} {version} image")


def main():
    parser = argparse.ArgumentParser(description=__doc__,
                                     formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--keep-failed", action="store_true", help="Leave a failed lab running for debugging")
    parser.add_argument("--sherpad-restart", metavar="COMMAND",
                        help="Also check links after running COMMAND, which restarts sherpad "
                             "(e.g. 'sudo systemctl restart sherpad')")
    parser.add_argument("--self-test", action="store_true", help="Run the runner's own unit tests")
    args = parser.parse_args()
    if args.self_test:
        sys.argv = sys.argv[:1]
        unittest.main()
        return 0
    signal.signal(signal.SIGTERM, stop_on_sigterm)
    run_root = RUNS / f"p2p-links-{uuid.uuid4().hex[:12]}"
    run_root.mkdir(parents=True)
    print(f"Logs: {run_root}", flush=True)
    try:
        preflight(run_root)
        LabRun(run_root, args.keep_failed, args.sherpad_restart).run()
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

    def test_no_link_sets_the_removed_p2p_field(self):
        self.assertFalse(any("p2p" in link for link in self.manifest()["links"]))

    def test_linux_links_match_the_manifest(self):
        links = self.manifest()["links"]
        for link in LINUX_LINKS:
            self.assertEqual(links[link.index]["src"], f"{link.src}::{link.src_interface}")
            self.assertEqual(links[link.index]["dst"], f"{link.dst}::{link.dst_interface}")
        self.assertEqual(links[0]["impairment"], {"delay": MANIFEST_DELAY})
        self.assertEqual({link.pair for link in LINUX_LINKS},
                         {"vm-vm", "vm-container", "container-container"})

    def test_bundles_match_the_manifest(self):
        links = {(link["src"], link["dst"]) for link in self.manifest()["links"]}
        for bundle in BUNDLES:
            for a_port, b_port in zip(bundle.a_ports, bundle.b_ports):
                pair = (f"{bundle.a}::eth{a_port.removeprefix('Ethernet')}",
                        f"{bundle.b}::eth{b_port.removeprefix('Ethernet')}")
                self.assertIn(pair, links)
        self.assertEqual({bundle.pair for bundle in BUNDLES},
                         {"vm-vm", "vm-container", "container-container"})

    def test_node_kinds_match_the_manifest(self):
        models = {node["name"]: node["model"] for node in self.manifest()["nodes"]}
        self.assertTrue(all(models[node] == "ubuntu_linux" for node in VMS))
        self.assertTrue(all(models[node] == "generic_container" for node in CONTAINERS))
        self.assertEqual(set(REDEPLOYED) | {REBOOTED}, set(VMS) | {"c1"})

    def test_arista_config_bundles_each_ports_pair(self):
        config = arista_config(arista_bundles("veos1"))
        self.assertIn("interface Ethernet1-2\nno switchport\nchannel-group 1 mode active", config)
        self.assertIn("interface Ethernet3-4\nno switchport\nchannel-group 2 mode active", config)
        self.assertTrue(config.endswith("write memory\n"))
        self.assertTrue(arista_config(arista_bundles("ceos1")).endswith("write memory\n"))
        self.assertEqual(arista_bundles("ceos1"), {2: ("Ethernet1", "Ethernet2"),
                                                   3: ("Ethernet3", "Ethernet4")})

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
        frames[("u1", "jumbo9000")] = (JUMBO_LENGTH, "-")
        self.assertEqual(frame_problems(frames, "u1"), [])
        del frames[("u1", "pause")]
        frames[("u1", "vlan10")] = (60, "-")
        self.assertEqual(frame_problems(frames, "u1"), ["pause missing", "vlan10 tag -"])
        self.assertEqual(received_frames("u1 lldp 32 -\nnoise\n"), {("u1", "lldp"): (32, "-")})

    def test_frame_tool_sends_every_kind(self):
        tool = {}
        exec(compile(FRAME_TOOL.read_text(), str(FRAME_TOOL), "exec"), tool)
        self.assertEqual(tuple(kind for kind, _, _ in tool["KINDS"]), FRAME_KINDS)
        self.assertEqual(len(tool["frame"]("u1", "jumbo9000", "ffffffffffff", "88b5")), JUMBO_LENGTH)

    def test_pcap_marks_finds_marked_frames(self):
        header = struct.pack("<IHHiIII", 0xA1B2C3D4, 2, 4, 0, 0, 65535, 1)
        packet = bytes(12) + b"\x88\xcc" + b"SHERPA391:c1:lldp\x00"
        record = struct.pack("<IIII", 0, 0, len(packet), len(packet)) + packet
        self.assertEqual(pcap_marks(header + record + record), {("c1", "lldp")})

    def test_ping_and_qdisc_parsing(self):
        self.assertEqual(average_rtt("rtt min/avg/max/mdev = 20.1/25.5/30.0/1.0 ms"), 25.5)
        self.assertIsNone(average_rtt("3 packets transmitted, 0 received"))
        self.assertEqual(netem_delay("qdisc netem 1: root refcnt 2 limit 1000 delay 20ms seed 1"), 20.0)
        self.assertEqual(netem_delay("qdisc netem 1: root limit 1000 delay 500us"), 0.5)
        self.assertIsNone(netem_delay("qdisc noqueue 0: root refcnt 2"))

    def test_lldp_and_port_channel_parsing(self):
        lldp = json.dumps({"lldpNeighbors": [{"port": "Ethernet1", "neighborDevice": "veos2.sherpa.lab.local",
                                              "neighborPort": "Ethernet1", "ttl": 120}]})
        self.assertEqual(lldp_neighbours(lldp), {"Ethernet1": ("veos2", "Ethernet1")})
        channel = json.dumps({"portChannels": {"Port-Channel1": {"activePorts": {"Ethernet1": {},
                                                                                 "Ethernet2": {}}}}})
        self.assertEqual(bundled_ports(channel, 1), {"Ethernet1", "Ethernet2"})
        self.assertEqual(bundled_ports(channel, 2), set())

    def test_outcome_keeps_streams_apart_for_parsing(self):
        outcome = Outcome(0, "x", "Warning\n")
        self.assertEqual(outcome.stdout, "x")
        self.assertIn("Warning", outcome.output)


if __name__ == "__main__":
    sys.exit(main())
