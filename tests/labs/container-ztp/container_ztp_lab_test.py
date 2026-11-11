#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.15"
# dependencies = []
# ///
"""End-to-end test of the ZTP setting on container nodes through the sherpa CLI (Python 3.15+, stdlib only).

Brings up each container model whose config Sherpa mounts (cEOS, SR Linux, cJunosEvolved, FRR)
twice, with its image's ZTP setting and with `ztp = "disabled"`, plus an application container.
Checks a node with ZTP on has Sherpa's config mounts and takes an SSH login as `sherpa`, and a
node with ZTP off is running with no such mount and no `sherpa` login, for the reason its model
has (SR Linux: no management address; FRR: no SSH server): after `sherpa up`, and after
redeploying one node of each setting. Checks `sherpa validate` refuses a method a model
does not take. Always destroys the lab. Runs on the Sherpa server; see README.md.
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

DISABLED = "disabled"
# Where Sherpa mounts the config it generates, by model. A node with ZTP on has every one of
# its model's; a node with ZTP off has none.
ZTP_MOUNTS = {
    "arista_ceos": ("/mnt/sherpa-ztp/startup-config",),
    "nokia_srlinux": ("/etc/opt/srlinux/config.json",),
    "juniper_cevolved": ("/home/evo/configdisk",),
    "frr_linux": ("/etc/frr/daemons", "/etc/frr/frr.conf", "/tmp/sherpa-init.sh"),
}
# A command that succeeds over SSH once the `sherpa` login works, by model.
LOGIN_COMMANDS = {
    "arista_ceos": "show version",
    "nokia_srlinux": "show version",
    "juniper_cevolved": "show version",
    "frr_linux": "true",
}
SSH_OPTIONS = ("-F", ".sherpa/ssh/config", "-o", "BatchMode=yes", "-o", "LogLevel=ERROR",
               "-o", "ConnectTimeout=10")
SHERPA_USER = "sherpa"
# Why an SSH login to a node with ZTP off fails, read from ssh's own message.
NO_ADDRESS = "no management address"
NO_SSH_SERVER = "no SSH server"
LOGIN_REFUSED = "login refused"
SSH_REFUSALS = (
    ("No route to host", NO_ADDRESS),
    ("Connection timed out", NO_ADDRESS),
    ("Connection refused", NO_SSH_SERVER),
    ("Permission denied", LOGIN_REFUSED),
)
# What a node with ZTP off is once it has booted, by model, where it has been observed. With
# no Sherpa config, SR Linux takes its management interface without the address its container
# was given, so nothing answers there. For about a minute before that, while it boots, the
# container still holds the address and the image's SSH server refuses the login. FRR is a Linux container: its address answers, and nothing listens for SSH. Models not
# listed must still refuse the login, for any of the reasons above.
ZTP_OFF_REFUSAL = {
    "nokia_srlinux": NO_ADDRESS,
    "frr_linux": NO_SSH_SERVER,
}
REFUSED_METHOD = "cdrom"

COMMAND_TIMEOUT = 180
UP_TIMEOUT = 1800
READY_TIMEOUT = 1200
POLL_INTERVAL = 10
# How long a node with ZTP off is watched to stay up after `sherpa up` reports it running.
SETTLE_SECONDS = 20


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
    raise TypeError(f"Unsupported TOML value: {value!r}")


def render_manifest(nodes, name):
    """The lab manifest for one run, under a unique lab name."""
    lines = [f"name = {value_toml(name)}", "", "nodes = ["]
    for node in nodes:
        fields = ", ".join(f"{key} = {value_toml(item)}" for key, item in node.items())
        lines.append(f"  {{ {fields} }},")
    lines.append("]")
    return "\n".join(lines) + "\n"


def select_nodes(manifest, models):
    """The manifest's nodes of the given models, or every node when none is given."""
    if not models:
        return list(manifest["nodes"])
    known = {node["model"] for node in manifest["nodes"]}
    unknown = sorted(set(models) - known)
    require(not unknown, f"the manifest has no node of model: {', '.join(unknown)}")
    return [node for node in manifest["nodes"] if node["model"] in models]


def ztp_is_off(node):
    return node.get("ztp") == DISABLED


def mount_problems(node, mounts):
    """What is wrong with a node's container mounts for its ZTP setting, if anything."""
    expected = ZTP_MOUNTS.get(node["model"], ())
    if ztp_is_off(node):
        return [f"has Sherpa's {path} mount" for path in expected if path in mounts]
    return [f"has no {path} mount" for path in expected if path not in mounts]


def ssh_refusal(output):
    """Why ssh could not log in, from its message, or None when the message names no reason."""
    for message, reason in SSH_REFUSALS:
        if message in output:
            return reason
    return None


def ssh_hostname(config):
    """The address `ssh -G` resolved for a host."""
    for line in config.splitlines():
        key, _, value = line.partition(" ")
        if key == "hostname":
            return value.strip()
    raise TestFailure("ssh -G printed no hostname")


def redeployed_nodes(nodes):
    """One node with ZTP on and one with it off, of the first model that has both."""
    for model in ZTP_MOUNTS:
        on = [node["name"] for node in nodes if node["model"] == model and not ztp_is_off(node)]
        off = [node["name"] for node in nodes if node["model"] == model and ztp_is_off(node)]
        if on and off:
            return [on[0], off[0]]
    return []


def refused_manifest(nodes, name):
    """A manifest whose first mounted-config node asks for a method its model does not take."""
    node = next(node for node in nodes if node["model"] in ZTP_MOUNTS)
    return render_manifest([{**node, "ztp": REFUSED_METHOD}], name), node["name"]


class LabRun:
    def __init__(self, run_root, keep_failed, models):
        manifest = tomllib.loads((LAB / "manifest.toml").read_text())
        self.nodes = select_nodes(manifest, models)
        self.name = f"{manifest['name']}-{run_root.name[-6:]}"
        self.directory = run_root / "lab"
        self.keep_failed = keep_failed
        self.started = False
        self.lab_id = None
        self.log = None

    # -- commands ----------------------------------------------------------------------------

    def command(self, label, arguments, timeout=COMMAND_TIMEOUT, check=True, cwd=None):
        outcome = run_logged(self.log, label, arguments, cwd or self.directory, timeout)
        if check and outcome.code != 0:
            raise TestFailure(f"{label} failed (exit {outcome.code}); see {self.log.name}")
        return outcome

    def sherpa(self, label, arguments, timeout=COMMAND_TIMEOUT, check=True, cwd=None):
        return self.command(label, ["sherpa", *arguments], timeout, check, cwd)

    def container(self, node):
        return f"{node['name']}-{self.lab_id}"

    def inspect(self, node, template):
        """A field of the node's container, through `docker inspect`."""
        outcome = self.command(f"inspect {node['name']}",
                               ["docker", "inspect", "--format", template, self.container(node)])
        return outcome.stdout.strip()

    def ssh(self, node):
        """Log in to the node as the lab's SSH config does, and run its model's command."""
        command = LOGIN_COMMANDS.get(node["model"], "true")
        return self.command(f"ssh {node['name']}",
                            ["ssh", *SSH_OPTIONS, f"{node['name']}.{self.lab_id}", command],
                            check=False)

    # -- lab ---------------------------------------------------------------------------------

    def run(self):
        self.directory.mkdir(parents=True)
        with (self.directory / "commands.log").open("w") as self.log:
            (self.directory / "manifest.toml").write_text(render_manifest(self.nodes, self.name))
            failure = None
            try:
                self.check_validate_refuses_an_unsupported_method()
                self.up()
                self.checks("after up")
                for name in redeployed_nodes(self.nodes):
                    self.sherpa(f"sherpa redeploy {name}", ["redeploy", "--node", name], UP_TIMEOUT)
                    self.checks(f"after redeploying {name}")
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

    def check_validate_refuses_an_unsupported_method(self):
        """`sherpa validate` names the node and the methods its model takes."""
        directory = self.directory.parent / "refused"
        directory.mkdir()
        manifest, node = refused_manifest(self.nodes, f"{self.name}-refused")
        (directory / "manifest.toml").write_text(manifest)
        outcome = self.sherpa("sherpa validate (unsupported method)", ["validate"], check=False,
                              cwd=directory)
        require(outcome.code != 0, f"sherpa validate accepted ztp = {REFUSED_METHOD!r} on {node}")
        require(node in outcome.output and "allows: disabled, volume" in outcome.output,
                f"sherpa validate did not say why {node} was refused: {outcome.output.strip()}")
        self.sherpa("sherpa validate", ["validate"])

    def up(self):
        self.started = True
        self.sherpa("sherpa up", ["up"], UP_TIMEOUT)
        info = tomllib.loads((self.directory / ".sherpa" / "lab-info.toml").read_text())
        require(info["name"] == self.name, f".sherpa/lab-info.toml names {info['name']}, not {self.name}")
        self.lab_id = info["id"]
        config = (self.directory / ".sherpa" / "ssh" / "config").read_text()
        require(f"User {SHERPA_USER}" in config, f"the lab's SSH config does not log in as {SHERPA_USER}")

    def checks(self, stage):
        for node in self.nodes:
            self.check_mounts(node, stage)
        for node in self.nodes:
            if node["model"] not in ZTP_MOUNTS:
                self.check_running(node, stage)
            elif ztp_is_off(node):
                self.check_no_sherpa_login(node, stage)
            else:
                self.wait_for_login(node, stage)
        # A node with ZTP off must still be up once the others have booted.
        time.sleep(SETTLE_SECONDS)
        for node in self.nodes:
            self.check_running(node, stage)

    def check_running(self, node, stage):
        state = self.inspect(node, "{{.State.Status}} {{.RestartCount}}")
        require(state == "running 0", f"{stage}, {node['name']} is {state!r}, not running")

    def check_mounts(self, node, stage):
        mounts = self.inspect(node, "{{range .Mounts}}{{.Destination}}\n{{end}}").split()
        problems = mount_problems(node, mounts)
        require(not problems, f"{stage}, {node['name']} {'; '.join(problems)}")

    def wait_for_login(self, node, stage, timeout=READY_TIMEOUT):
        """The node takes an SSH login as `sherpa`, retrying while its NOS is still booting."""
        deadline = time.monotonic() + timeout
        while self.ssh(node).code != 0:
            require(time.monotonic() < deadline,
                    f"{stage}, no SSH login as {SHERPA_USER} on {node['name']} within {timeout}s")
            time.sleep(POLL_INTERVAL)

    def address(self, node):
        """The management address the lab's SSH config reaches the node at."""
        outcome = self.command(f"address of {node['name']}",
                               ["ssh", "-G", *SSH_OPTIONS, f"{node['name']}.{self.lab_id}"])
        return ssh_hostname(outcome.stdout)

    def answers_ping(self, node):
        outcome = self.command(f"ping {node['name']}", ["ping", "-c", "2", "-W", "2", self.address(node)],
                               check=False)
        return outcome.code == 0

    def refused_login(self, node, stage):
        """Why the node refuses an SSH login as `sherpa`. Taking the login is a failure."""
        name = node["name"]
        self.check_running(node, stage)
        outcome = self.ssh(node)
        require(outcome.code != 0, f"{stage}, {name} has ZTP off but took an SSH login as {SHERPA_USER}")
        refusal = ssh_refusal(outcome.output)
        require(refusal is not None,
                f"{stage}, the SSH login to {name} failed for an unknown reason: {outcome.output.strip()}")
        return refusal

    def check_no_sherpa_login(self, node, stage, timeout=READY_TIMEOUT):
        """The node is running and has no `sherpa` login, for the reason its model has with ZTP
        off once it has booted. While it boots, the reason may be another; a login never works."""
        name = node["name"]
        expected = ZTP_OFF_REFUSAL.get(node["model"])
        refusal = self.refused_login(node, stage)
        if expected is None:
            print(f"[{self.directory.name}] {name} with ZTP off: {refusal}", flush=True)
            return
        deadline = time.monotonic() + timeout
        while refusal != expected:
            require(time.monotonic() < deadline,
                    f"{stage}, {name} with ZTP off: expected {expected}, still {refusal} after {timeout}s")
            time.sleep(POLL_INTERVAL)
            refusal = self.refused_login(node, stage)
        if expected == NO_ADDRESS:
            require(not self.answers_ping(node),
                    f"{stage}, {name} has ZTP off but answers on its management address")
        if expected == NO_SSH_SERVER:
            require(self.answers_ping(node),
                    f"{stage}, {name} does not answer on its management address")

    def destroy(self):
        """Destroy the lab and check nothing is left. Returns a failure, or None."""
        if not self.started:
            return None
        if not (self.directory / ".sherpa" / "lab-info.toml").exists():
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


def preflight(run_root, models):
    for tool in ("sherpa", "docker", "ssh", "ip", "ping"):
        require(shutil.which(tool), f"{tool} is not on PATH")
    nodes = select_nodes(tomllib.loads((LAB / "manifest.toml").read_text()), models)
    with (run_root / "preflight.log").open("w") as log:
        whoami = run_logged(log, "sherpa whoami", ["sherpa", "whoami"], run_root)
        require(whoami.code == 0, "the sherpa CLI is not logged in (run `sherpa login`)")
        docker = run_logged(log, "docker usable", ["docker", "ps", "--quiet"], run_root)
        require(docker.code == 0, "this user cannot use Docker; this test runs on the Sherpa server")
        for model, version in sorted({(node["model"], node["version"]) for node in nodes}):
            image = run_logged(log, f"{model} {version} image present",
                               ["sherpa", "image", "show", model, "--version", version], run_root)
            require(image.code == 0, f"the server has no {model} {version} image")


def main():
    parser = argparse.ArgumentParser(description=__doc__,
                                     formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--keep-failed", action="store_true", help="Leave a failed lab running for debugging")
    parser.add_argument("--self-test", action="store_true", help="Run the runner's own unit tests")
    parser.add_argument("--models", default="",
                        help="Comma-separated models to run, for a server without every image "
                             "(default: every node in the manifest)")
    args = parser.parse_args()
    if args.self_test:
        sys.argv = sys.argv[:1]
        unittest.main()
        return 0
    signal.signal(signal.SIGTERM, stop_on_sigterm)
    models = [model for model in args.models.split(",") if model]
    run_root = RUNS / f"container-ztp-{uuid.uuid4().hex[:12]}"
    run_root.mkdir(parents=True)
    print(f"Logs: {run_root}", flush=True)
    try:
        preflight(run_root, models)
        LabRun(run_root, args.keep_failed, models).run()
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
        rendered = tomllib.loads(render_manifest(manifest["nodes"], "renamed"))
        self.assertEqual(rendered["name"], "renamed")
        self.assertEqual(rendered["nodes"], manifest["nodes"])

    def test_every_mounted_config_model_runs_with_ztp_on_and_off(self):
        nodes = self.manifest()["nodes"]
        for model in ZTP_MOUNTS:
            settings = sorted(ztp_is_off(node) for node in nodes if node["model"] == model)
            self.assertEqual(settings, [False, True], model)
        self.assertEqual(set(LOGIN_COMMANDS), set(ZTP_MOUNTS))

    def test_the_lab_has_an_application_container_with_no_ztp_field(self):
        others = [node for node in self.manifest()["nodes"] if node["model"] not in ZTP_MOUNTS]
        self.assertTrue(others)
        self.assertTrue(all("ztp" not in node for node in others))

    def test_every_node_pins_its_version(self):
        self.assertTrue(all(node.get("version") for node in self.manifest()["nodes"]))

    def test_models_select_their_nodes_and_unknown_models_are_refused(self):
        manifest = self.manifest()
        self.assertEqual(select_nodes(manifest, []), manifest["nodes"])
        chosen = select_nodes(manifest, ["frr_linux", "nats_bus"])
        self.assertEqual([node["name"] for node in chosen], ["frr01", "frr02", "nats01"])
        with self.assertRaises(TestFailure):
            select_nodes(manifest, ["no_such_model"])

    def test_mounts_are_required_with_ztp_on_and_forbidden_with_it_off(self):
        on = {"name": "frr01", "model": "frr_linux"}
        off = {"name": "frr02", "model": "frr_linux", "ztp": DISABLED}
        mounts = list(ZTP_MOUNTS["frr_linux"])
        self.assertEqual(mount_problems(on, mounts), [])
        self.assertEqual(mount_problems(off, ["/etc/own.conf"]), [])
        self.assertEqual(mount_problems(on, mounts[:2]), ["has no /tmp/sherpa-init.sh mount"])
        self.assertEqual(mount_problems(off, mounts[:1]), ["has Sherpa's /etc/frr/daemons mount"])
        self.assertEqual(mount_problems({"name": "nats01", "model": "nats_bus"}, []), [])

    def test_one_node_of_each_setting_is_redeployed(self):
        nodes = self.manifest()["nodes"]
        self.assertEqual(redeployed_nodes(nodes), ["ceos01", "ceos02"])
        self.assertEqual(redeployed_nodes(select_nodes(self.manifest(), ["frr_linux"])),
                         ["frr01", "frr02"])
        self.assertEqual(redeployed_nodes(select_nodes(self.manifest(), ["nats_bus"])), [])

    def test_the_refused_manifest_asks_for_a_method_no_container_takes(self):
        manifest, node = refused_manifest(self.manifest()["nodes"], "refused")
        parsed = tomllib.loads(manifest)
        self.assertEqual(node, "ceos01")
        self.assertEqual(parsed["nodes"], [{"name": "ceos01", "model": "arista_ceos",
                                            "version": "4.35.1F", "ztp": REFUSED_METHOD}])

    def test_ssh_refusals_are_read_from_its_message(self):
        self.assertEqual(ssh_refusal("channel 0: open failed: connect failed: No route to host\n"),
                         NO_ADDRESS)
        self.assertEqual(ssh_refusal("channel 0: open failed: connect failed: Connection refused\n"),
                         NO_SSH_SERVER)
        self.assertEqual(ssh_refusal("sherpa@192.0.2.1: Permission denied (publickey).\n"), LOGIN_REFUSED)
        self.assertIsNone(ssh_refusal("kex_exchange_identification: read: Connection reset\n"))

    def test_expected_refusals_are_for_mounted_config_models_and_known_reasons(self):
        self.assertTrue(set(ZTP_OFF_REFUSAL) <= set(ZTP_MOUNTS))
        reasons = {reason for _, reason in SSH_REFUSALS}
        self.assertTrue(set(ZTP_OFF_REFUSAL.values()) <= reasons)

    def test_the_address_is_read_from_ssh_g(self):
        self.assertEqual(ssh_hostname("user sherpa\nhostname 172.31.9.12\nport 22\n"), "172.31.9.12")
        with self.assertRaises(TestFailure):
            ssh_hostname("user sherpa\n")

    def test_outcome_keeps_streams_apart_for_parsing(self):
        outcome = Outcome(0, "x", "Warning\n")
        self.assertEqual(outcome.stdout, "x")
        self.assertIn("Warning", outcome.output)


if __name__ == "__main__":
    sys.exit(main())
