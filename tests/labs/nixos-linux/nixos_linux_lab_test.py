#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.15"
# dependencies = ["pytest==9.2.0"]
# ///
"""End-to-end test of NixOS lab nodes through the sherpa CLI (Python 3.15+, pytest).

Brings up two `nixos_linux` VMs built from bwks/nixos-image, and checks the cases NixOS shows
because it does things differently from the other distros: cloud-init finishing cleanly, the SSH
host key Sherpa pinned being the one on disk, the lab owner's login with NixOS's bash, a group of
their own and sudo through wheel, eth interface names, a text file owned by the owner's own group,
and an SSH provisioner run as the owner. Always destroys the lab. See README.md.

A pytest module: the `prepared` fixture checks the server and writes the lab's manifest and
files, the `lab` fixture brings the lab up once for the module and destroys it at the end, and
the `ready` fixture waits until every node takes SSH and has finished cloud-init. The cases are
the `test_<case_id>_*` functions, in the order the lab goes through. Run as a script, it runs
pytest on itself; `--self-test` runs the runner's own unit tests instead.
"""

import argparse
from pathlib import Path
import shlex
import shutil
import signal
import sys
import tempfile
import time
import tomllib
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
# NixOS has no /bin/bash; Sherpa gives its accounts the system profile's bash.
NIXOS_SHELL = "/run/current-system/sw/bin/bash"
TEXT_FILE = "hello.txt"
TEXT_FILE_CONTENT = "written by the nixos-linux lab test\n"
# The SSH provisioner's script records who ran it and where, in the login's home.
PROVISION_SCRIPT = "provision.sh"
PROVISION_SCRIPT_CONTENT = '#!/bin/sh\necho "$(id -un)@$(hostname)" > "$HOME/provisioned.txt"\n'
# The interfaces every node has: management, then the link.
INTERFACES = ("eth0", "eth1")
AFTER_UP = "after up"

UP_TIMEOUT = 900
READY_TIMEOUT = 600
POLL_INTERVAL = 10

# OpenSSH options for lab nodes: no prompts, and no host key notices mixed into command output.
SSH_OPTIONS = ("-F", ".sherpa/ssh/config", "-o", "BatchMode=yes", "-o", "LogLevel=ERROR")
# What a destroyed lab must not leave on the Sherpa server, and how to list it. A leftover is a
# line that holds the lab's ID. A listing whose tool is missing, off the server, is skipped.
HOST_LISTINGS = (
    ("libvirt domains", ["virsh", "-c", "qemu:///system", "list", "--all", "--name"]),
    ("host interfaces", ["ip", "-brief", "link"]),
)


def render_manifest(manifest, name):
    """The lab manifest for one run, under a unique lab name."""
    lines = [f"name = {value_toml(name)}"]
    lines += [f"{key} = {value_toml(value)}" for key, value in manifest.items()
              if key not in ("name", "nodes", "links")]
    for section in ("nodes", "links"):
        lines.append("")
        lines.append(f"{section} = [")
        for entry in manifest.get(section, []):
            lines.append(f"  {value_toml(entry)},")
        lines.append("]")
    return "\n".join(lines) + "\n"


def provisioned_nodes(manifest):
    """The nodes with an SSH provisioner."""
    return [node["name"] for node in manifest["nodes"]
            if any(item["type"] == "ssh" for item in node.get("provisioners", []))]


def text_file_targets(manifest):
    """(node, dst, "user:group mode") for every text file the manifest writes."""
    return [(node["name"], item["dst"], f"{item['user']}:{item['group']} {item['permissions']}")
            for node in manifest["nodes"] for item in node.get("text_files", [])]


def with_owner(manifest, owner):
    """The manifest with `{owner}` in its text file paths and owners replaced by the lab owner's
    name: the owner's account is the only login on the node, so the file belongs to it."""
    nodes = [{**node, "text_files": [{key: value.replace("{owner}", owner) if isinstance(value, str)
                                      else value for key, value in item.items()}
                                     for item in node["text_files"]]} if "text_files" in node else node
             for node in manifest["nodes"]]
    return {**manifest, "nodes": nodes}


def interfaces_script(interfaces):
    """A node command printing the name of each of the interfaces the node has."""
    return "; ".join(f"[ -e /sys/class/net/{name} ] && echo {name}" for name in interfaces) + "; true"


def missing_interfaces(output, interfaces):
    """The named interfaces that `interfaces_script` output does not list."""
    present = set(output.split())
    return [name for name in interfaces if name not in present]


def leftovers(listing, lab_id):
    """The first word of each line of a host listing that holds the lab's ID."""
    return [line.split()[0] for line in listing.splitlines() if lab_id in line]


# -- the lab -------------------------------------------------------------------------------------


class Lab:
    """The lab every case uses: its manifest written by `prepare`, brought up once by `up` and
    destroyed by `destroy` in the `lab` fixture's teardown."""

    def __init__(self, run_root, owner):
        self.owner = owner
        self.manifest = with_owner(tomllib.loads((LAB / "manifest.toml").read_text()), owner)
        self.name = f"{self.manifest['name']}-{run_root.name[-6:]}"
        self.directory = run_root / "lab"
        self.nodes = [node["name"] for node in self.manifest["nodes"]]
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

    # -- lifecycle ---------------------------------------------------------------------------

    def prepare(self):
        self.directory.mkdir(parents=True)
        self.log = (self.directory / "commands.log").open("w")
        (self.directory / "manifest.toml").write_text(render_manifest(self.manifest, self.name))
        (self.directory / TEXT_FILE).write_text(TEXT_FILE_CONTENT)
        (self.directory / PROVISION_SCRIPT).write_text(PROVISION_SCRIPT_CONTENT)

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

    def destroy(self):
        """Destroy the lab and check `sherpa inspect` no longer finds it and, on the Sherpa server,
        nothing of it is left on the host. Does nothing when the lab was never started."""
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
            if not shutil.which(arguments[0]):
                self.log.write(f"=== {kind} leftover check skipped: {arguments[0]} not found\n")
                continue
            listing = self.command(f"no leftover {kind}", arguments, check=False)
            if listing.code != 0:
                self.log.write(f"=== {kind} leftover check skipped: exit {listing.code}\n")
                continue
            left = leftovers(listing.stdout, self.lab_id)
            if left:
                found[kind] = left
        require(not found, "left after destroy: " + "; ".join(f"{kind} {names}" for kind, names in found.items()))

    # -- nodes -------------------------------------------------------------------------------

    def wait_for_node(self, node, timeout=READY_TIMEOUT):
        deadline = time.monotonic() + timeout
        while self.node(f"{node} reachable", node, "true", check=False).code != 0:
            require(time.monotonic() < deadline, f"{node} not reachable over SSH after {timeout}s")
            time.sleep(POLL_INTERVAL)

    def cloud_init_status(self, node):
        """Wait for cloud-init to finish on the node and return its status command's outcome."""
        return self.node(f"{node} cloud-init finished", node, "cloud-init status --wait --long",
                         timeout=READY_TIMEOUT, check=False)


# -- the fixtures --------------------------------------------------------------------------------


def preflight(run_root):
    for tool in ("sherpa", "ssh"):
        require(shutil.which(tool), f"{tool} is not on PATH")
    manifest = tomllib.loads((LAB / "manifest.toml").read_text())
    with (run_root / "preflight.log").open("w") as log:
        whoami = run_logged(log, "sherpa whoami", ["sherpa", "whoami"], run_root)
        require(whoami.code == 0, "the sherpa CLI is not logged in (run `sherpa login`)")
        owner = require_device_password(log, run_root)
        images = sorted({(node["model"], node["version"]) for node in manifest["nodes"]})
        for model, version in images:
            image = run_logged(log, f"{model} {version} image present",
                               ["sherpa", "image", "show", model, "--version", version], run_root)
            require(image.code == 0, f"the server has no {model} {version} image "
                    "(build it from bwks/nixos-image and import it)")
    return owner


@pytest.fixture(scope="module")
def prepared():
    """The lab before it is up: the server checked and the run's manifest and files written."""
    signal.signal(signal.SIGTERM, stop_on_sigterm)
    run_root = RUNS / f"nixos-linux-{uuid.uuid4().hex[:12]}"
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
def ready(lab):
    """The lab once every node takes SSH and has finished cloud-init, however it finished, once
    for the module: the text files and the provisioner come after cloud-init."""
    for node in lab.nodes:
        lab.wait_for_node(node)
        lab.cloud_init_status(node)
    return lab


# -- cases ---------------------------------------------------------------------------------------


def test_config_delivery_003_a_cloud_init_vm_finishes_cloud_init_without_an_error(ready):
    # Exit 0 is a clean finish; 2 would mean warnings, which the image is built to avoid.
    failed = []
    for node in ready.nodes:
        outcome = ready.cloud_init_status(node)
        if outcome.code != 0:
            failed.append(f"{node} (exit {outcome.code}): {outcome.output.strip()}")
    require(not failed, f"cloud-init did not finish cleanly on {'; '.join(failed)}")


def test_node_host_keys_001_the_pinned_key_of_a_vm_is_the_host_key_on_its_disk(ready):
    known_hosts = (ready.directory / ".sherpa" / "ssh" / "known_hosts").read_text().splitlines()
    pinned = [line.split(None, 1)[1] for line in known_hosts if line.strip()]
    for node in ready.nodes:
        outcome = ready.node(f"{node} host key on disk", node,
                             "cut -d' ' -f1,2 /etc/ssh/ssh_host_ed25519_key.pub")
        on_disk = outcome.stdout.strip()
        require(on_disk and on_disk in pinned,
                f"{node}: the pinned host key is not the key on disk ({on_disk!r})")


def test_owner_login_027_on_a_nixos_vm_the_owner_has_the_system_bash_a_group_of_their_own_and_sudo_through_wheel(ready):
    for node in ready.nodes:
        outcome = ready.node(f"{node} lab owner account", node,
                             'u=$(id -un); getent passwd "$u" | cut -d: -f7; id -gn; id -Gn; '
                             'sudo -n true && echo sudo-ok')
        shell, primary, groups, sudo = (outcome.stdout.splitlines() + [""] * 4)[:4]
        require(shell == NIXOS_SHELL, f"{node}: lab owner's shell is {shell!r}, not {NIXOS_SHELL}")
        require(primary not in ("users", "wheel", ""),
                f"{node}: lab owner has no group of its own: {primary!r}")
        require("wheel" in groups.split(), f"{node}: lab owner is not in wheel: {groups!r}")
        require(sudo == "sudo-ok", f"{node}: lab owner has no passwordless sudo")


def test_node_interfaces_009_a_vm_has_its_interfaces_named_eth0_eth1(ready):
    problems = []
    for node in ready.nodes:
        outcome = ready.node(f"{node} interfaces", node, interfaces_script(INTERFACES))
        problems += [f"{node} {name} missing" for name in missing_interfaces(outcome.stdout, INTERFACES)]
    require(not problems, f"{AFTER_UP}, {'; '.join(problems)}")


def test_boot_files_001_a_text_file_in_the_owners_home_has_its_owner_group_and_mode(ready):
    targets = text_file_targets(ready.manifest)
    require(targets, "the manifest writes no text file")
    for node, dst, expected in targets:
        outcome = ready.node(f"{node} text file {dst}", node, f"sudo -n stat -c '%U:%G %a' {shlex.quote(dst)}")
        require(outcome.stdout.strip() == expected,
                f"{node}: {dst} is {outcome.stdout.strip()!r}, not {expected!r}")


def test_provisioner_scripts_001_at_up_a_script_runs_as_the_owner_in_their_home(ready):
    nodes = provisioned_nodes(ready.manifest)
    require(nodes, "the manifest has no SSH provisioner")
    for node in nodes:
        outcome = ready.node(f"{node} provisioned", node, 'echo "$(id -un)@$(hostname)"; cat ~/provisioned.txt')
        lines = outcome.stdout.splitlines()
        require(len(lines) == 2 and lines[0] == lines[1],
                f"{node}: the SSH provisioner's script did not run as the lab owner: {lines!r}")


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

    def test_interfaces_must_be_there(self):
        self.assertEqual(missing_interfaces("eth0\neth1\n", INTERFACES), [])
        self.assertEqual(missing_interfaces("eth0\n", INTERFACES), ["eth1"])
        self.assertEqual(missing_interfaces("", INTERFACES), ["eth0", "eth1"])

    def test_a_partly_created_lab_keeps_its_id_for_cleanup(self):
        # `sherpa up` exits 1 on a partly created lab after writing lab-info.toml; the run must
        # still know the lab's ID, so destroy checks for leftovers instead of skipping them.
        with tempfile.TemporaryDirectory() as temp:
            lab = Lab(Path(temp) / "nixos-linux-000000abcdef", owner="alice")
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

            def leftover(log, label, arguments, *rest, **options):
                return Outcome(1 if "inspect" in arguments else 0, "node-4ed6360e-x\n", "")

            with (mock.patch(f"{__name__}.run_logged", leftover),
                  mock.patch("shutil.which", return_value="/usr/bin/true"),
                  self.assertRaises(TestFailure)):
                lab.destroy()


if __name__ == "__main__":
    sys.exit(main())
