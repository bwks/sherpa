#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.15"
# dependencies = ["pytest==9.2.0"]
# ///
"""End-to-end test of Cisco IOL container nodes through the sherpa CLI (Python 3.15+, pytest).

Brings up two `cisco_iol` routers and an Alpine host on P2p links, and checks the IOL cases of
tests/specs/ztp.md, links-and-bridges.md and nodes.md: the generated config read over the lab
owner's SSH login; TCP from the host into IOL across the links, which needs transmit checksum
offload off on IOL's veths, at up and after a cold boot; both routers started by resume and
reachable after a cold boot (`sherpa down`, the containers stopped, `sherpa resume`); r1 keeping
the config it saved with `write memory` and r2, which saved nothing, back on its startup config;
and the links working again after the cold boot. Always destroys the lab. Runs on the Sherpa
server; see README.md.

A pytest module: the `prepared` fixture checks the server and writes the lab's manifest, the
`lab` fixture brings the lab up once for the module and destroys it at the end, the `configured`
fixture configures the routers and addresses the host, the `cold_booted` fixture saves r1's
config and cold-boots both routers, and the `reconfigured` fixture configures r2 again after
it. The cases are the `test_<case_id>_*` functions, in the order the lab goes through. Run as a
script, it runs pytest on itself; `--self-test` runs the runner's own unit tests instead.
"""

import argparse
from pathlib import Path
import shlex
import shutil
import signal
import sys
import time
import tomllib
import unittest
import uuid

import pytest

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))

from labkit import (
    COMMAND_TIMEOUT,
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
ROUTERS = ("r1", "r2")
HOST = "linux01"
HOST_INTERFACE = "eth1"
# The image of the throwaway container that joins the host's network namespace to address and
# test its interface.
SIDECAR_IMAGE = "alpine:3.22"
# IOL's MTU is fixed at 1500, so the host's link to it must be 1500 too.
IOL_MTU = 1500

R1_TO_R2 = "10.47.0.1"
R2_TO_R1 = "10.47.0.2"
R1_TO_HOST = "10.47.1.1"
HOST_ADDRESS = "10.47.1.10"
ROUTER_CONFIG = {
    "r1": ("interface Ethernet0/1", "ip address 10.47.0.1 255.255.255.252", "no shutdown",
           "interface Ethernet0/2", "ip address 10.47.1.1 255.255.255.0", "no shutdown"),
    "r2": ("interface Ethernet0/1", "ip address 10.47.0.2 255.255.255.252", "no shutdown",
           "exit", "ip route 10.47.1.0 255.255.255.0 10.47.0.1"),
}
# Each router's link addresses, as `show running-config` lists them once configured.
ROUTER_ADDRESSES = {"r1": (R1_TO_R2, R1_TO_HOST), "r2": (R2_TO_R1,)}
ROUTER_PINGS = (("r1", R2_TO_R1), ("r2", R1_TO_R2), ("r1", HOST_ADDRESS), ("r2", HOST_ADDRESS))
HOST_TARGETS = (R1_TO_HOST, R1_TO_R2, R2_TO_R1)
# The routers' SSH servers the host reads a banner from across the links.
TCP_TARGETS = (R1_TO_HOST, R2_TO_R1)
AFTER_UP = "after up"
AFTER_COLD_BOOT = "after a cold boot"

UP_TIMEOUT = 1200
READY_TIMEOUT = 600
POLL_INTERVAL = 10
PING_ROUNDS = 6
# IOS runs one command per exec session and closes it before a ping ends, so pings and config
# go through an interactive session fed from stdin, held open this long.
IOS_SESSION_SECONDS = 10
# What a destroyed lab must not leave on the host, and how to list it. A leftover is a line that
# holds the lab's ID.
HOST_LISTINGS = (
    ("Docker containers", ["docker", "ps", "--all", "--format", "{{.Names}}"]),
    ("Docker networks", ["docker", "network", "ls", "--format", "{{.Name}}"]),
    ("host interfaces", ["ip", "-brief", "link"]),
)


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


def ios_session(lines):
    """What is typed into an interactive IOS session to run `lines`."""
    return "\n".join(["terminal length 0", *lines]) + "\n"


def config_session(router):
    """The session that applies a router's link config."""
    return ios_session(["configure terminal", *ROUTER_CONFIG[router], "end"])


def ping_succeeded(output):
    """Whether an IOS ping got at least one reply."""
    for line in output.splitlines():
        if "Success rate is" in line:
            return not line.strip().startswith("Success rate is 0 percent")
    return False


def configured_addresses(running_config):
    """The IPv4 addresses on the interfaces of an IOS running config."""
    return {line.split()[2] for line in running_config.splitlines()
            if line.startswith(" ip address ") and len(line.split()) >= 4}


def host_script():
    """A command addressing the host's link to r1, at IOL's MTU, with a route to the r1–r2 link."""
    return (f"ip link set {HOST_INTERFACE} mtu {IOL_MTU} && "
            f"ip addr replace {HOST_ADDRESS}/24 dev {HOST_INTERFACE} && "
            f"ip route replace 10.47.0.0/30 via {R1_TO_HOST}")


def ping_script(targets):
    """A host command pinging each target once, printing `ok <target>` for each reply."""
    return "; ".join(f"ping -c 1 -W 2 {target} >/dev/null 2>&1 && echo ok {target}"
                     for target in targets) + "; true"


def ssh_banner_script(targets):
    """A host command printing each target's SSH banner, read over TCP, as `<target> <banner>`."""
    return "; ".join(f"echo {target} $(echo | nc -w 5 {target} 22 | head -n 1)" for target in targets)


def banners(output):
    """The targets whose SSH server answered, from `ssh_banner_script` output."""
    answered = set()
    for line in output.splitlines():
        fields = line.split(maxsplit=1)
        if len(fields) == 2 and fields[1].startswith("SSH-"):
            answered.add(fields[0])
    return answered


def replied(output):
    return {line.split()[1] for line in output.splitlines() if line.startswith("ok ")}


def leftovers(listing, lab_id):
    """The first word of each line of a host listing that holds the lab's ID."""
    return [line.split()[0] for line in listing.splitlines() if lab_id in line]


# -- the lab -------------------------------------------------------------------------------------


class Lab:
    """The lab every case uses: its manifest written by `prepare`, brought up once by `up` and
    destroyed by `destroy` in the `lab` fixture's teardown."""

    def __init__(self, run_root, owner):
        self.owner = owner
        self.manifest = tomllib.loads((LAB / "manifest.toml").read_text())
        self.name = f"{self.manifest['name']}-{run_root.name[-6:]}"
        self.directory = run_root / "lab"
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

    def container(self, node):
        return f"{node}-{self.lab_id}"

    def ssh_arguments(self, node, terminal=False):
        tty = ["-tt"] if terminal else []
        return ["ssh", "-F", ".sherpa/ssh/config", "-o", "BatchMode=yes", "-o", "ConnectTimeout=10",
                *tty, f"{node}.{self.lab_id}"]

    def ios_exec(self, label, router, command, check=False):
        """Run one IOS exec command over SSH as the lab owner."""
        return self.command(label, [*self.ssh_arguments(router), command], check=check)

    def ios_interactive(self, label, router, lines):
        """Type `lines` into an interactive IOS session, keep it open while they run, then log
        out. IOS drops what is typed while a command runs, so nothing follows but the logout."""
        ssh = shlex.join(self.ssh_arguments(router, terminal=True))
        script = f'{{ printf "%s" "$1"; sleep {IOS_SESSION_SECONDS}; printf "exit\\n"; }} | {ssh}'
        return self.command(label, ["sh", "-c", script, "sh", lines], check=False)

    def host_exec(self, label, script, check=True):
        """Run a shell script as root in a throwaway container sharing the host's network
        namespace: the host node is not privileged, so it cannot address its own interface."""
        return self.command(label, ["docker", "run", "--rm", "--network", f"container:{self.container(HOST)}",
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

    def cold_boot(self):
        """Save r1's config, then cold-boot both routers: `sherpa down` pauses containers, so the
        routers are stopped too, as a host restart would, and `sherpa resume` starts them from cold."""
        save = self.ios_interactive("write memory on r1", "r1", ios_session(["write memory"]))
        require("[OK]" in save.output, f"write memory on r1 failed; see {self.log.name}")
        self.sherpa("sherpa down", ["down"], UP_TIMEOUT)
        self.command("stop the routers", ["docker", "stop", "-t", "5",
                                          *(self.container(router) for router in ROUTERS)])
        self.sherpa("sherpa resume", ["resume"], UP_TIMEOUT)

    def destroy(self):
        """Destroy the lab and check `sherpa inspect` no longer finds it and nothing of it is left
        on the host. Does nothing when the lab was never started."""
        if not self.started:
            return
        if self.lab_id is None:
            # `sherpa up` failed early; destroy whatever it created, found by the manifest's lab name.
            self.sherpa("sherpa destroy (after failed up)", ["destroy", "--yes"], UP_TIMEOUT, check=False)
            return
        self.sherpa("sherpa destroy", ["destroy", "--yes"], UP_TIMEOUT)
        inspect = self.sherpa("lab gone", ["inspect"], check=False)
        require(inspect.code != 0, f"lab {self.lab_id} still exists after destroy")
        found = {}
        for kind, arguments in HOST_LISTINGS:
            left = leftovers(self.command(f"no leftover {kind}", arguments).stdout, self.lab_id)
            if left:
                found[kind] = left
        require(not found, "left after destroy: " + "; ".join(f"{kind} {names}" for kind, names in found.items()))

    # -- routers and the host ----------------------------------------------------------------

    def wait_for_ssh(self, router, stage, timeout=READY_TIMEOUT):
        """Wait until the router's SSH server takes the lab owner's login on its management address."""
        deadline = time.monotonic() + timeout
        while self.ios_exec(f"{router} SSH", router, "show clock").code != 0:
            require(time.monotonic() < deadline, f"{stage}, {router} SSH did not answer within {timeout}s")
            time.sleep(POLL_INTERVAL)

    def running_config(self, router, stage):
        return self.ios_exec(f"{router} running config {stage}", router, "show running-config").stdout

    def configure(self, router):
        outcome = self.ios_interactive(f"configure {router}", router, config_session(router))
        require("% Invalid" not in outcome.output, f"{router} refused its config; see {self.log.name}")

    def require_pings(self, stage):
        """Each router reaches its peer and the host, and the host both routers, retrying while
        neighbours learn the addresses."""
        pending_routers = set(ROUTER_PINGS)
        pending_host = set(HOST_TARGETS)
        for round_number in range(PING_ROUNDS):
            for router, target in sorted(pending_routers):
                outcome = self.ios_interactive(f"ping {router} -> {target}", router,
                                               ios_session([f"ping {target} repeat 5"]))
                if ping_succeeded(outcome.output):
                    pending_routers.discard((router, target))
            if pending_host:
                outcome = self.host_exec("pings from linux01", ping_script(sorted(pending_host)),
                                         check=False)
                pending_host -= replied(outcome.stdout)
            if not pending_routers and not pending_host:
                return
            time.sleep(POLL_INTERVAL * (round_number + 1) // 2)
        failed = [f"{router} -> {target}" for router, target in sorted(pending_routers)]
        failed += [f"{HOST} -> {target}" for target in sorted(pending_host)]
        raise TestFailure(f"{stage}, no reply on: {'; '.join(failed)}")

    def tcp_unanswered(self, targets):
        """The targets whose SSH banner the host could not read across the links."""
        outcome = self.host_exec("SSH banners from linux01", ssh_banner_script(targets), check=False)
        return set(targets) - banners(outcome.stdout)

    def require_tcp(self, targets, stage, timeout=0):
        """The host reads each target's SSH banner across the links: TCP into IOL works. Retries
        for up to `timeout` seconds while the routers boot."""
        deadline = time.monotonic() + timeout
        missing = self.tcp_unanswered(targets)
        while missing and time.monotonic() < deadline:
            time.sleep(POLL_INTERVAL)
            missing = self.tcp_unanswered(sorted(missing))
        require(not missing, f"{stage}, no TCP answer from {sorted(missing)}: transmit checksum "
                f"offload may be on at IOL's veths")


# -- the fixtures --------------------------------------------------------------------------------


def preflight(run_root):
    for tool in ("sherpa", "docker", "ip", "ssh"):
        require(shutil.which(tool), f"{tool} is not on PATH")
    manifest = tomllib.loads((LAB / "manifest.toml").read_text())
    with (run_root / "preflight.log").open("w") as log:
        whoami = run_logged(log, "sherpa whoami", ["sherpa", "whoami"], run_root)
        require(whoami.code == 0, "the sherpa CLI is not logged in (run `sherpa login`)")
        owner = require_device_password(log, run_root)
        docker = run_logged(log, f"{SIDECAR_IMAGE} present", ["docker", "image", "inspect", SIDECAR_IMAGE],
                            run_root)
        require(docker.code == 0, f"Docker has no {SIDECAR_IMAGE} image, or this user cannot use Docker; "
                f"this test runs on the Sherpa server")
        images = sorted({(node["model"], node.get("kind", ""), node["version"]) for node in manifest["nodes"]})
        for model, kind, version in images:
            arguments = ["sherpa", "server", "image", "show", model, "--version", version]
            if kind:
                arguments += ["--kind", kind]
            image = run_logged(log, f"{model} {kind} {version} image present", arguments, run_root)
            require(image.code == 0, f"the server has no {model} {kind} {version} image; build the IOL "
                    f"images with docker-cisco-iol (see README.md)")
    return owner


@pytest.fixture(scope="module")
def prepared():
    """The lab before it is up: the server checked and the run's manifest written."""
    signal.signal(signal.SIGTERM, stop_on_sigterm)
    run_root = RUNS / f"cisco-iol-{uuid.uuid4().hex[:12]}"
    run_root.mkdir(parents=True)
    print(f"Logs: {run_root}", flush=True)
    lab = Lab(run_root, preflight(run_root))
    lab.prepare()
    try:
        yield lab
    finally:
        lab.log.close()


@pytest.fixture(scope="module")
def lab(request, prepared):
    """The lab every case uses: brought up once and destroyed at the end, even when a case fails,
    unless `--keep-failed` is given and something failed. The teardown also checks nothing of
    the lab is left on the host."""
    keep_failed = request.config.getoption("keep_failed")
    keep = False
    try:
        try:
            prepared.up()
        except Exception:
            keep = keep_failed
            raise
        yield prepared
        keep = keep_failed and request.session.testsfailed > 0
    finally:
        if keep:
            print(f"keeping failed lab {prepared.name} ({prepared.lab_id}) in {prepared.directory}", flush=True)
        else:
            prepared.destroy()


@pytest.fixture(scope="module")
def configured(lab):
    """The lab with both routers' link addresses configured and the host addressed, once for the module."""
    for router in ROUTERS:
        lab.wait_for_ssh(router, AFTER_UP)
        lab.configure(router)
    lab.host_exec("address linux01", host_script())
    return lab


@pytest.fixture(scope="module")
def cold_booted(configured):
    """The lab after r1's config is saved with `write memory` and both routers are cold-booted,
    once for the module. Nothing is configured again."""
    configured.cold_boot()
    return configured


@pytest.fixture(scope="module")
def reconfigured(cold_booted):
    """The cold-booted lab with r2, which saved nothing, configured again."""
    cold_booted.wait_for_ssh("r2", AFTER_COLD_BOOT)
    cold_booted.configure("r2")
    return cold_booted


# -- cases ---------------------------------------------------------------------------------------


def test_config_delivery_002_a_cisco_iol_node_has_its_generated_name_management_interface_and_owner_login(lab):
    for router in ROUTERS:
        lab.wait_for_ssh(router, AFTER_UP)
        config = lab.running_config(router, AFTER_UP)
        require(f"hostname {router}" in config, f"{router} has not its generated hostname")
        require("interface Ethernet0/0" in config, f"{router} has no management interface config")
        require(f"username {lab.owner} privilege 15" in config, f"{router} has no login for {lab.owner}")


def test_veth_checksums_001_tcp_across_a_link_into_a_cisco_iol_node_works_at_up_and_after_a_cold_boot(
        configured, request):
    # Pings first: ping working while TCP fails is what points at checksum offload.
    configured.require_pings(AFTER_UP)
    configured.require_tcp(TCP_TARGETS, AFTER_UP)
    # The cold boot comes after the checks above, so they see the lab as up left it. Only r1,
    # which saved its config, answers on its link address before r2 is configured again.
    cold_booted = request.getfixturevalue("cold_booted")
    cold_booted.require_tcp((R1_TO_HOST,), AFTER_COLD_BOOT, timeout=READY_TIMEOUT)


def test_node_resume_004_a_container_stopped_outside_sherpa_is_started_by_resume_and_reachable(cold_booted):
    for router in ROUTERS:
        cold_booted.wait_for_ssh(router, AFTER_COLD_BOOT)


def test_node_resume_006_a_cisco_iol_node_keeps_a_saved_config_across_a_cold_boot_and_an_unsaved_one_does_not(
        cold_booted):
    for router in ROUTERS:
        cold_booted.wait_for_ssh(router, AFTER_COLD_BOOT)
    kept = configured_addresses(cold_booted.running_config("r1", AFTER_COLD_BOOT))
    require(set(ROUTER_ADDRESSES["r1"]) <= kept, f"r1 lost its saved config: {sorted(kept)}")
    reset = configured_addresses(cold_booted.running_config("r2", AFTER_COLD_BOOT))
    require(not set(ROUTER_ADDRESSES["r2"]) & reset, f"r2 kept a config it never saved: {sorted(reset)}")


def test_link_reattach_026_a_link_of_a_container_stopped_outside_sherpa_works_again_after_resume(reconfigured):
    reconfigured.require_pings(AFTER_COLD_BOOT)


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

    def test_ios_ping_results_are_read(self):
        self.assertTrue(ping_succeeded("Success rate is 100 percent (5/5), round-trip"))
        self.assertTrue(ping_succeeded("Success rate is 80 percent (4/5), round-trip"))
        self.assertFalse(ping_succeeded("Success rate is 0 percent (0/5)"))
        self.assertFalse(ping_succeeded("% Unrecognized host or address"))

    def test_configured_addresses_are_read_from_the_running_config(self):
        config = ("interface Ethernet0/0\n ip address 172.31.1.21 255.255.255.0\n"
                  "interface Ethernet0/1\n ip address 10.47.0.1 255.255.255.252\n"
                  "interface Ethernet0/3\n no ip address\n")
        self.assertEqual(configured_addresses(config), {"172.31.1.21", "10.47.0.1"})

    def test_ssh_banners_are_read_per_target(self):
        output = "10.47.1.1 SSH-2.0-Cisco-1.25\n10.47.0.2\n"
        self.assertEqual(banners(output), {"10.47.1.1"})

    def test_only_addresses_that_replied_are_counted(self):
        self.assertEqual(replied("ok 10.47.1.1\nnoise\n"), {"10.47.1.1"})

    def test_a_leftover_is_a_listed_name_holding_the_lab_id(self):
        listing = "veth-4ed6360e-x UP\nlo UNKNOWN\nr1-4ed6360e\n"
        self.assertEqual(leftovers(listing, "4ed6360e"), ["veth-4ed6360e-x", "r1-4ed6360e"])
        self.assertEqual(leftovers("lo UNKNOWN\n", "4ed6360e"), [])


if __name__ == "__main__":
    sys.exit(main())
