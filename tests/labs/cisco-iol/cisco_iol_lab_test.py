#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.15"
# dependencies = []
# ///
"""End-to-end test of Cisco IOL container nodes through the sherpa CLI (Python 3.15+, stdlib only).

Brings up two `cisco_iol` routers and an Alpine host on P2p links. Checks the generated config
and SSH on the management address, pings across both links and routed through r1, and TCP from
the host to IOL across the links, which needs transmit checksum offload off on IOL's veths.
Then saves r1's config with `write memory`, cold-boots both routers (`sherpa down`, the
containers stopped, `sherpa resume`) and checks r1 kept its config and r2 went back to its
startup config. Always destroys the lab. Runs on the Sherpa server; see README.md.
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

COMMAND_TIMEOUT = 180
UP_TIMEOUT = 1200
READY_TIMEOUT = 600
POLL_INTERVAL = 10
PING_ROUNDS = 6
# IOS runs one command per exec session and closes it before a ping ends, so pings and config
# go through an interactive session fed from stdin, held open this long.
IOS_SESSION_SECONDS = 10


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


class LabRun:
    def __init__(self, run_root, keep_failed):
        self.manifest = tomllib.loads((LAB / "manifest.toml").read_text())
        self.name = f"{self.manifest['name']}-{run_root.name[-6:]}"
        self.directory = run_root / "lab"
        self.keep_failed = keep_failed
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

    def ssh_arguments(self, node, terminal=False):
        tty = ["-tt"] if terminal else []
        return ["ssh", "-F", ".sherpa/ssh/config", "-o", "BatchMode=yes", "-o", "ConnectTimeout=10",
                *tty, f"{node}.{self.lab_id}"]

    def ios_exec(self, label, router, command, check=False):
        """Run one IOS exec command over SSH as `sherpa`."""
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
        outcome = self.sherpa("sherpa up", ["up"], UP_TIMEOUT, check=False)
        info_path = self.directory / ".sherpa" / "lab-info.toml"
        if info_path.exists():
            info = tomllib.loads(info_path.read_text())
            require(info["name"] == self.name, f".sherpa/lab-info.toml names {info['name']}, not {self.name}")
            self.lab_id = info["id"]
        require(outcome.code == 0, f"sherpa up failed (exit {outcome.code}); see {self.log.name}")
        require(self.lab_id is not None, "sherpa up wrote no .sherpa/lab-info.toml")

    def checks(self):
        for router in ROUTERS:
            self.wait_for_ssh(router)
            self.check_generated_config(router)
            self.configure(router)
        self.host_exec("address linux01", host_script())
        self.check_pings("after up")
        self.check_tcp("after up")

        save = self.ios_interactive("write memory on r1", "r1", ios_session(["write memory"]))
        require("[OK]" in save.output, f"write memory on r1 failed; see {self.log.name}")
        self.cold_boot()
        for router in ROUTERS:
            self.wait_for_ssh(router)
        kept = configured_addresses(self.ios_exec("r1 config after cold boot", "r1",
                                                  "show running-config").stdout)
        require(set(ROUTER_ADDRESSES["r1"]) <= kept, f"r1 lost its saved config: {sorted(kept)}")
        reset = configured_addresses(self.ios_exec("r2 config after cold boot", "r2",
                                                   "show running-config").stdout)
        require(not set(ROUTER_ADDRESSES["r2"]) & reset,
                f"r2 kept a config it never saved: {sorted(reset)}")
        self.configure("r2")
        self.check_pings("after a cold boot")
        self.check_tcp("after a cold boot")

    def wait_for_ssh(self, router, timeout=READY_TIMEOUT):
        """Wait until the router's SSH server answers on its management address."""
        deadline = time.monotonic() + timeout
        while self.ios_exec(f"{router} SSH", router, "show clock").code != 0:
            require(time.monotonic() < deadline, f"{router} SSH did not answer within {timeout}s")
            time.sleep(POLL_INTERVAL)

    def check_generated_config(self, router):
        config = self.ios_exec(f"{router} running config", router, "show running-config").stdout
        require(f"hostname {router}" in config, f"{router} has not its generated hostname")
        require("interface Ethernet0/0" in config, f"{router} has no management interface config")
        require("username sherpa privilege 15" in config, f"{router} has no sherpa login")

    def configure(self, router):
        outcome = self.ios_interactive(f"configure {router}", router, config_session(router))
        require("% Invalid" not in outcome.output, f"{router} refused its config; see {self.log.name}")

    def cold_boot(self):
        """`sherpa down` pauses containers, so the routers are stopped too, as a host restart
        would, and `sherpa resume` starts them from cold."""
        self.sherpa("sherpa down", ["down"], UP_TIMEOUT)
        self.command("stop the routers", ["docker", "stop", "-t", "5",
                                          *(self.container(router) for router in ROUTERS)])
        self.sherpa("sherpa resume", ["resume"], UP_TIMEOUT)

    def check_pings(self, stage):
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

    def check_tcp(self, stage):
        """The host reads both routers' SSH banners across the links: TCP into IOL works."""
        targets = (R1_TO_HOST, R2_TO_R1)
        outcome = self.host_exec("SSH banners from linux01", ssh_banner_script(targets), check=False)
        missing = set(targets) - banners(outcome.stdout)
        require(not missing, f"{stage}, no TCP answer from {sorted(missing)}: transmit checksum "
                f"offload may be on at IOL's veths")

    def destroy(self):
        """Destroy the lab and check nothing is left. Returns a failure, or None."""
        if not self.started:
            return None
        if not (self.directory / ".sherpa" / "lab-info.toml").exists():
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
    for tool in ("sherpa", "docker", "ip", "ssh"):
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
                               ["sherpa", "server", "image", "show", model, "--version", version],
                               run_root)
            require(image.code == 0, f"the server has no {model} {version} image; build the IOL "
                    f"images with docker-cisco-iol (see README.md)")


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
    run_root = RUNS / f"cisco-iol-{uuid.uuid4().hex[:12]}"
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

    def test_the_routers_are_iol_and_every_router_has_a_config(self):
        models = {node["name"]: node["model"] for node in self.manifest()["nodes"]}
        self.assertEqual({models[router] for router in ROUTERS}, {"cisco_iol"})
        self.assertEqual(set(ROUTER_CONFIG), set(ROUTERS))
        self.assertEqual(set(ROUTER_ADDRESSES), set(ROUTERS))

    def test_links_use_iol_interface_names(self):
        for link in self.manifest()["links"]:
            for end in (link["src"], link["dst"]):
                node, interface = end.split("::")
                if node in ROUTERS:
                    self.assertRegex(interface, r"^Ethernet[0-3]/[0-3]$")

    def test_a_config_session_enters_and_leaves_config_mode(self):
        session = config_session("r1").splitlines()
        self.assertEqual(session[:2], ["terminal length 0", "configure terminal"])
        self.assertEqual(session[-1], "end")

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

    def test_the_host_link_is_set_to_iol_mtu(self):
        self.assertIn(f"mtu {IOL_MTU}", host_script())
        self.assertEqual(IOL_MTU, 1500)

    def test_ssh_banners_are_read_per_target(self):
        output = "10.47.1.1 SSH-2.0-Cisco-1.25\n10.47.0.2\n"
        self.assertEqual(banners(output), {"10.47.1.1"})

    def test_ping_script_reports_each_reply(self):
        self.assertIn("ping -c 1 -W 2 10.47.1.1", ping_script(["10.47.1.1"]))
        self.assertEqual(replied("ok 10.47.1.1\nnoise\n"), {"10.47.1.1"})

    def test_outcome_keeps_streams_apart_for_parsing(self):
        outcome = Outcome(0, "x", "Warning\n")
        self.assertEqual(outcome.stdout, "x")
        self.assertIn("Warning", outcome.output)


if __name__ == "__main__":
    sys.exit(main())
