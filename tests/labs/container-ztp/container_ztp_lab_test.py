#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.15"
# dependencies = ["pytest==9.2.0"]
# ///
"""End-to-end test of the ZTP setting on container nodes through the sherpa CLI (Python 3.15+,
pytest).

Brings up each container model whose config Sherpa mounts (cEOS, SR Linux, FRR) twice, with its
image's ZTP setting and with `ztp.method = "disabled"`, plus an application container, and checks
the container cases of tests/specs/ztp.md: `sherpa validate` refuses a method a model does not
take; a node with ZTP on has Sherpa's config mounts and takes the lab owner's SSH login by key,
after up and after a redeploy; a node with ZTP off is running with no such mount and no owner
login, for the reason its model has (SR Linux and cEOS: no management address; FRR: no SSH
server), after up and after a redeploy; and every node is still running once the others have
booted. Always destroys the lab. Runs on the Sherpa server; see README.md.

A pytest module: the `prepared` fixture checks the server and writes the lab's manifest, the
`lab` fixture brings the lab up once for the module and destroys it at the end, and the
`redeployed` fixture redeploys every node with ZTP on or off for the cases that need it. The cases are
the `test_<case_id>_*` functions, in case order. Run as a script, it runs pytest on itself;
`--self-test` runs the runner's own unit tests instead.
"""

import argparse
import json
from pathlib import Path
import re
import shlex
import shutil
import signal
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


LAB = Path(__file__).resolve().parent
ROOT = LAB.parent.parent.parent
RUNS = ROOT / ".tmp" / "lab-tests"

DISABLED = "disabled"
# Where Sherpa mounts the config it generates, by model. A node with ZTP on has every one of
# its model's; a node with ZTP off has none.
ZTP_MOUNTS = {
    "arista_eos": ("/mnt/sherpa-ztp/startup-config",),
    "nokia_srlinux": ("/etc/opt/srlinux/config.json",),
    "frr_linux": ("/etc/frr/daemons", "/etc/frr/frr.conf", "/tmp/sherpa-init.sh"),
}
# A command that succeeds over SSH once the lab owner's login works, by model.
LOGIN_COMMANDS = {
    "arista_eos": "show version",
    "nokia_srlinux": "show version",
    "frr_linux": "true",
}
SSH_OPTIONS = ("-F", ".sherpa/ssh/config", "-o", "BatchMode=yes", "-o", "LogLevel=ERROR",
               "-o", "ConnectTimeout=10")
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
# What a node with ZTP off is once it has booted, by model, as observed. With no Sherpa config,
# SR Linux and cEOS take their management interface without the address their container was
# given, so nothing answers there. While they boot, the container still holds the address and
# the image's SSH server refuses the login (SR Linux, for about a minute) or nothing listens yet
# (cEOS). FRR is a Linux container: its address answers, and nothing listens for SSH.
ZTP_OFF_REFUSAL = {
    "arista_eos": NO_ADDRESS,
    "nokia_srlinux": NO_ADDRESS,
    "frr_linux": NO_SSH_SERVER,
}
REFUSED_METHOD = "cdrom"
AFTER_UP = "after up"

COMMAND_TIMEOUT = 180
UP_TIMEOUT = 1800
READY_TIMEOUT = 1200
POLL_INTERVAL = 10
# How long every node is watched to stay up once the nodes with ZTP on have booted.
SETTLE_SECONDS = 20


class TestFailure(Exception):
    """A check failed; the message says which and why."""

    __test__ = False


class Outcome(NamedTuple):
    """A finished command. Parse `stdout`; search `output` for messages, which may be on either stream."""

    code: int | None
    stdout: str
    stderr: str

    @property
    def output(self):
        return self.stdout + self.stderr


class Redeployed(NamedTuple):
    """The nodes the `redeployed` fixture redeployed: those with ZTP on and those with it off."""

    on: list
    off: list


def require(condition, message):
    if not condition:
        raise TestFailure(message)


def run_logged(log, label, arguments, cwd, timeout=COMMAND_TIMEOUT):
    """Run a command, record it and its output in the log, and return its Outcome."""
    print(f"[{cwd.name}] {label}", flush=True)
    try:
        result = subprocess.run(arguments, cwd=cwd, capture_output=True, text=True, timeout=timeout,
                                stdin=subprocess.DEVNULL)
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
    if isinstance(value, dict):
        return "{ " + ", ".join(f"{key} = {value_toml(item)}" for key, item in value.items()) + " }"
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
    return node.get("ztp", {}).get("method") == DISABLED


def ztp_on_nodes(nodes):
    """The nodes whose config Sherpa mounts, with ZTP on."""
    return [node for node in nodes if node["model"] in ZTP_MOUNTS and not ztp_is_off(node)]


def ztp_off_nodes(nodes):
    """The nodes whose model's config Sherpa mounts, with ZTP off."""
    return [node for node in nodes if node["model"] in ZTP_MOUNTS and ztp_is_off(node)]


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


def refused_manifest(nodes, name):
    """A manifest whose first mounted-config node asks for a method its model does not take."""
    node = next((node for node in nodes if node["model"] in ZTP_MOUNTS), None)
    require(node is not None, "the run has no node whose model's config Sherpa mounts")
    return render_manifest([{**node, "ztp": {"method": REFUSED_METHOD}}], name), node["name"]


# -- the lab -------------------------------------------------------------------------------------


class Lab:
    """The lab every case uses: its manifest written by `prepare`, brought up once by `up` and
    always destroyed by `destroy`."""

    def __init__(self, run_root, models, owner):
        self.owner = owner
        manifest = tomllib.loads((LAB / "manifest.toml").read_text())
        self.nodes = select_nodes(manifest, models)
        self.name = f"{manifest['name']}-{run_root.name[-6:]}"
        self.directory = run_root / "lab"
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
        """Log in to the node as the lab owner, through the lab's SSH config, and run its model's
        command."""
        command = LOGIN_COMMANDS.get(node["model"], "true")
        return self.command(f"ssh {self.owner}@{node['name']}",
                            ["ssh", *SSH_OPTIONS, "-l", self.owner, f"{node['name']}.{self.lab_id}", command],
                            check=False)

    # -- lifecycle ---------------------------------------------------------------------------

    def prepare(self):
        self.directory.mkdir(parents=True)
        self.log = (self.directory / "commands.log").open("w")
        (self.directory / "manifest.toml").write_text(render_manifest(self.nodes, self.name))

    def up(self):
        self.started = True
        outcome = self.sherpa("sherpa up", ["up"], UP_TIMEOUT, check=False)
        # A partly created lab exits non-zero after writing lab-info.toml: record its ID first, so
        # destroy can check the lab is gone.
        info_path = self.directory / ".sherpa" / "lab-info.toml"
        if info_path.exists():
            info = tomllib.loads(info_path.read_text())
            require(info["name"] == self.name, f".sherpa/lab-info.toml names {info['name']}, not {self.name}")
            self.lab_id = info["id"]
        require(outcome.code == 0, f"sherpa up failed (exit {outcome.code}); see {self.log.name}")
        require(self.lab_id is not None, "sherpa up wrote no .sherpa/lab-info.toml")

    def redeploy(self, node):
        name = node["name"]
        self.sherpa(f"sherpa redeploy {name}", ["redeploy", "--node", name], UP_TIMEOUT)

    def destroy(self):
        """Destroy the lab and check it is gone. Returns a failure, or None."""
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
        except TestFailure as error:
            return error
        return None

    # -- what a node shows -------------------------------------------------------------------

    def require_running(self, node, stage):
        """The node's container is running and has never restarted."""
        state = self.inspect(node, "{{.State.Status}} {{.RestartCount}}")
        require(state == "running 0", f"{stage}, {node['name']} is {state!r}, not running")

    def require_mounts(self, node, stage):
        """The node's container has its model's config mounts if ZTP is on, and none if it is off."""
        mounts = self.inspect(node, "{{range .Mounts}}{{.Destination}}\n{{end}}").split()
        problems = mount_problems(node, mounts)
        require(not problems, f"{stage}, {node['name']} {'; '.join(problems)}")

    def wait_for_login(self, node, stage, timeout=READY_TIMEOUT):
        """The node takes the lab owner's SSH login by key, retrying while its NOS is still booting."""
        deadline = time.monotonic() + timeout
        while self.ssh(node).code != 0:
            require(time.monotonic() < deadline,
                    f"{stage}, no SSH login as {self.owner} on {node['name']} within {timeout}s")
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
        """Why the running node refuses an SSH login as the lab owner. Taking the login is a failure,
        and so is a container that is not running, which would refuse it for no reason of ZTP."""
        name = node["name"]
        self.require_running(node, stage)
        outcome = self.ssh(node)
        require(outcome.code != 0, f"{stage}, {name} has ZTP off but took an SSH login as {self.owner}")
        refusal = ssh_refusal(outcome.output)
        require(refusal is not None,
                f"{stage}, the SSH login to {name} failed for an unknown reason: {outcome.output.strip()}")
        return refusal

    def require_no_owner_login(self, node, stage, timeout=READY_TIMEOUT):
        """The node has no login for the lab owner, for the reason its model has with ZTP off once
        it has booted. While it boots, the reason may be another; a login never works."""
        name = node["name"]
        expected = ZTP_OFF_REFUSAL[node["model"]]
        refusal = self.refused_login(node, stage)
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


# -- the fixtures --------------------------------------------------------------------------------


DEVICE_PASSWORD_SET = re.compile(r"^\s*Device password:\s*set\s*$", re.MULTILINE)
INFO_USERNAME = re.compile(r"^\s*Username:\s*(\S+)\s*$", re.MULTILINE)


def require_device_password(log, run_root):
    """Stop unless the lab owner has a device password, which `sherpa up` needs; return the owner."""
    info = run_logged(log, "sherpa server user info", ["sherpa", "server", "user", "info"], run_root)
    require(info.code == 0, "could not read the user's details (`sherpa server user info`)")
    require(DEVICE_PASSWORD_SET.search(info.stdout),
            "the user has no device password, which `sherpa up` needs: "
            "run `sherpa server user device-password`")
    owner = INFO_USERNAME.search(info.stdout)
    require(owner, "`sherpa server user info` did not name the user")
    return owner.group(1)


def preflight(run_root, models):
    for tool in ("sherpa", "docker", "ssh", "ip", "ping"):
        require(shutil.which(tool), f"{tool} is not on PATH")
    nodes = select_nodes(tomllib.loads((LAB / "manifest.toml").read_text()), models)
    with (run_root / "preflight.log").open("w") as log:
        whoami = run_logged(log, "sherpa whoami", ["sherpa", "whoami"], run_root)
        require(whoami.code == 0, "the sherpa CLI is not logged in (run `sherpa login`)")
        owner = require_device_password(log, run_root)
        docker = run_logged(log, "docker usable", ["docker", "ps", "--quiet"], run_root)
        require(docker.code == 0, "this user cannot use Docker; this test runs on the Sherpa server")
        images = {(node["model"], node.get("kind", ""), node["version"]) for node in nodes}
        for model, kind, version in sorted(images):
            arguments = ["sherpa", "image", "show", model, "--version", version]
            if kind:
                arguments += ["--kind", kind]
            image = run_logged(log, f"{model} {kind} {version} image present", arguments, run_root)
            require(image.code == 0, f"the server has no {model} {kind} {version} image")
    return owner


@pytest.fixture(scope="module")
def prepared(request):
    """The lab before it is up: the server checked and the run's manifest written."""
    signal.signal(signal.SIGTERM, stop_on_sigterm)
    models = [model for model in request.config.getoption("models").split(",") if model]
    run_root = RUNS / f"container-ztp-{uuid.uuid4().hex[:12]}"
    run_root.mkdir(parents=True)
    print(f"Logs: {run_root}", flush=True)
    lab = Lab(run_root, models, preflight(run_root, models))
    lab.prepare()
    try:
        yield lab
    finally:
        lab.log.close()


@pytest.fixture(scope="module")
def lab(request, prepared):
    """The lab every case but the validate one uses: brought up once and destroyed at the end,
    even when a case fails, unless `--keep-failed` is given and something failed."""
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
            cleanup = prepared.destroy()
            if cleanup:
                raise cleanup


@pytest.fixture(scope="module")
def redeployed(lab):
    """Every node with ZTP on or off, redeployed once for the module."""
    redeployed = Redeployed(ztp_on_nodes(lab.nodes), ztp_off_nodes(lab.nodes))
    for node in [*redeployed.on, *redeployed.off]:
        lab.redeploy(node)
    return redeployed


# -- cases ---------------------------------------------------------------------------------------


def test_ztp_setting_001_validate_refuses_a_method_the_model_does_not_take(prepared):
    directory = prepared.directory.parent / "refused"
    directory.mkdir()
    manifest, node = refused_manifest(prepared.nodes, f"{prepared.name}-refused")
    (directory / "manifest.toml").write_text(manifest)
    outcome = prepared.sherpa("sherpa validate (unsupported method)", ["validate"], check=False,
                              cwd=directory)
    require(outcome.code != 0, f"sherpa validate accepted ztp.method = {REFUSED_METHOD!r} on {node}")
    require(node in outcome.output and "allows: disabled, volume" in outcome.output,
            f"sherpa validate did not say why {node} was refused: {outcome.output.strip()}")
    prepared.sherpa("sherpa validate", ["validate"])


def test_ztp_disabled_001_a_container_with_ztp_off_runs_with_no_config_mount(lab):
    nodes = ztp_off_nodes(lab.nodes)
    require(nodes, "the run has no node with ZTP off")
    for node in nodes:
        lab.require_running(node, AFTER_UP)
        lab.require_mounts(node, AFTER_UP)


def test_ztp_disabled_002_a_container_with_ztp_off_refuses_its_owners_login(lab):
    nodes = ztp_off_nodes(lab.nodes)
    require(nodes, "the run has no node with ZTP off")
    for node in nodes:
        lab.require_no_owner_login(node, AFTER_UP)


def test_config_delivery_001_a_container_with_ztp_on_has_its_config_mounts_and_its_owners_login(lab, request):
    nodes = ztp_on_nodes(lab.nodes)
    require(nodes, "the run has no node with ZTP on")
    for node in nodes:
        lab.require_running(node, AFTER_UP)
        lab.require_mounts(node, AFTER_UP)
        lab.wait_for_login(node, AFTER_UP)
    # The redeploy comes after the checks above, so they see every node as up left it.
    for node in request.getfixturevalue("redeployed").on:
        stage = f"after redeploying {node['name']}"
        lab.require_running(node, stage)
        lab.require_mounts(node, stage)
        lab.wait_for_login(node, stage)


def test_ztp_disabled_003_a_container_redeployed_with_ztp_off_is_as_it_was_after_up(lab, redeployed):
    require(redeployed.off, "the run has no node with ZTP off")
    for node in redeployed.off:
        stage = f"after redeploying {node['name']}"
        lab.require_running(node, stage)
        lab.require_mounts(node, stage)
        lab.require_no_owner_login(node, stage)


def test_ztp_disabled_004_every_node_is_still_running_once_the_others_have_booted(lab):
    for node in ztp_on_nodes(lab.nodes):
        lab.wait_for_login(node, "waiting for the nodes with ZTP on to boot")
    time.sleep(SETTLE_SECONDS)
    for node in lab.nodes:
        lab.require_running(node, f"{SETTLE_SECONDS}s after the nodes with ZTP on booted")


# -- running -------------------------------------------------------------------------------------


def main():
    parser = argparse.ArgumentParser(description=__doc__,
                                     formatter_class=argparse.RawDescriptionHelpFormatter,
                                     epilog="Other arguments, such as --keep-failed, --models or -k, are passed "
                                            "to pytest.")
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

    def manifest(self):
        return tomllib.loads((LAB / "manifest.toml").read_text())

    def test_a_partly_created_lab_keeps_its_id_for_cleanup(self):
        # `sherpa up` exits 1 on a partly created lab after writing lab-info.toml; the run must
        # still know the lab's ID, so destroy checks the lab is gone instead of skipping it.
        with tempfile.TemporaryDirectory() as temp:
            lab = Lab(Path(temp) / "container-ztp-000000abcdef", models=[], owner="alice")
            lab.prepare()
            self.addCleanup(lab.log.close)
            (lab.directory / ".sherpa").mkdir()
            (lab.directory / ".sherpa" / "lab-info.toml").write_text(
                f'id = "4ed6360e"\nname = "{lab.name}"\n')

            def failing_up(log, label, arguments, *rest):
                return Outcome(1 if arguments[:2] == ["sherpa", "up"] else 0, "", "")

            with mock.patch(f"{__name__}.run_logged", failing_up), self.assertRaises(TestFailure):
                lab.up()
            self.assertEqual(lab.lab_id, "4ed6360e")

            def still_there(log, label, arguments, *rest):
                return Outcome(0, "", "")

            with mock.patch(f"{__name__}.run_logged", still_there):
                self.assertIsInstance(lab.destroy(), TestFailure)

    def test_models_select_their_nodes_and_unknown_models_are_refused(self):
        manifest = self.manifest()
        self.assertEqual(select_nodes(manifest, []), manifest["nodes"])
        chosen = select_nodes(manifest, ["frr_linux", "nats_bus"])
        self.assertEqual([node["name"] for node in chosen], ["frr01", "frr02", "nats01"])
        with self.assertRaises(TestFailure):
            select_nodes(manifest, ["no_such_model"])

    def test_mounts_are_required_with_ztp_on_and_forbidden_with_it_off(self):
        on = {"name": "frr01", "model": "frr_linux"}
        off = {"name": "frr02", "model": "frr_linux", "ztp": {"method": DISABLED}}
        mounts = list(ZTP_MOUNTS["frr_linux"])
        self.assertEqual(mount_problems(on, mounts), [])
        self.assertEqual(mount_problems(off, ["/etc/own.conf"]), [])
        self.assertEqual(mount_problems(on, mounts[:2]), ["has no /tmp/sherpa-init.sh mount"])
        self.assertEqual(mount_problems(off, mounts[:1]), ["has Sherpa's /etc/frr/daemons mount"])
        self.assertEqual(mount_problems({"name": "nats01", "model": "nats_bus"}, []), [])

    def test_ssh_refusals_are_read_from_its_message(self):
        self.assertEqual(ssh_refusal("channel 0: open failed: connect failed: No route to host\n"),
                         NO_ADDRESS)
        self.assertEqual(ssh_refusal("channel 0: open failed: connect failed: Connection refused\n"),
                         NO_SSH_SERVER)
        self.assertEqual(ssh_refusal("alice@192.0.2.1: Permission denied (publickey).\n"), LOGIN_REFUSED)
        self.assertIsNone(ssh_refusal("kex_exchange_identification: read: Connection reset\n"))


if __name__ == "__main__":
    sys.exit(main())
