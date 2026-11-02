#!/usr/bin/env python3
"""End-to-end test of NixOS lab nodes through the sherpa CLI (Python 3.11+, stdlib only).

Brings up two `nixos_linux` VMs built from images/nixos, and checks what NixOS does differently
from the other distros: the lab owner's login with NixOS's bash, sudo through wheel, per-user
groups for text-file ownership, cloud-init finishing cleanly, an SSH provisioner, eth interface
names, password login allowed by the image, and a P2p link. Always destroys the lab. See README.md.
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
# NixOS has no /bin/bash; Sherpa gives its accounts the system profile's bash.
NIXOS_SHELL = "/run/current-system/sw/bin/bash"
TEXT_FILE = "hello.txt"
TEXT_FILE_CONTENT = "written by the nixos-linux lab test\n"
# The SSH provisioner's script records who ran it and where, in the login's home.
PROVISION_SCRIPT = "provision.sh"
PROVISION_SCRIPT_CONTENT = '#!/bin/sh\necho "$(id -un)@$(hostname)" > "$HOME/provisioned.txt"\n'
# Link `n` (0-based) uses this /24 with `n` as its third octet.
SUBNET_PREFIX = "10.46"

COMMAND_TIMEOUT = 180
UP_TIMEOUT = 900
READY_TIMEOUT = 600
POLL_INTERVAL = 10
PING_ATTEMPTS = 5

# OpenSSH options for lab nodes: no prompts, and no host key notices mixed into command output.
SSH_OPTIONS = ("-F", "sherpa_ssh_config", "-o", "BatchMode=yes", "-o", "LogLevel=ERROR")


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


class Address(NamedTuple):
    node: str
    interface: str
    address: str


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
    if isinstance(value, list):
        return "[" + ", ".join(value_toml(item) for item in value) + "]"
    if isinstance(value, dict):
        return "{ " + ", ".join(f"{key} = {value_toml(item)}" for key, item in value.items()) + " }"
    raise TypeError(f"Unsupported TOML value: {value!r}")


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


def endpoint(text):
    node, interface = text.split("::")
    return node, interface


def link_plan(manifest):
    """Addresses for both ends of every link, and the pings that check them."""
    addresses = []
    pings = []
    for index, link in enumerate(manifest.get("links", [])):
        subnet = f"{SUBNET_PREFIX}.{index}"
        ends = [Address(*endpoint(link["src"]), f"{subnet}.1/24"),
                Address(*endpoint(link["dst"]), f"{subnet}.2/24")]
        addresses += ends
        pings += [(ends[0], ends[1]), (ends[1], ends[0])]
    return addresses, pings


def host(address):
    return address.address.split("/")[0]


def configure_command(addresses):
    """One remote command that brings up and addresses a node's interfaces."""
    return " && ".join(f"sudo -n ip link set {item.interface} up && "
                       f"sudo -n ip addr replace {item.address} dev {item.interface}"
                       for item in addresses)


def provisioned_nodes(manifest):
    """The nodes with an SSH provisioner."""
    return [node["name"] for node in manifest["nodes"]
            if any(item["type"] == "ssh" for item in node.get("provisioners", []))]


def text_file_targets(manifest):
    """(node, dst, "user:group mode") for every text file the manifest writes."""
    return [(node["name"], item["dst"], f"{item['user']}:{item['group']} {item['permissions']}")
            for node in manifest["nodes"] for item in node.get("text_files", [])]


class LabRun:
    def __init__(self, run_root, keep_failed):
        self.manifest = tomllib.loads((LAB / "manifest.toml").read_text())
        self.name = f"{self.manifest['name']}-{run_root.name[-6:]}"
        self.directory = run_root / "lab"
        self.keep_failed = keep_failed
        self.addresses, self.pings = link_plan(self.manifest)
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

    # -- lab ---------------------------------------------------------------------------------

    def run(self):
        self.directory.mkdir(parents=True)
        with (self.directory / "commands.log").open("w") as self.log:
            (self.directory / "manifest.toml").write_text(render_manifest(self.manifest, self.name))
            (self.directory / TEXT_FILE).write_text(TEXT_FILE_CONTENT)
            (self.directory / PROVISION_SCRIPT).write_text(PROVISION_SCRIPT_CONTENT)
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
        for node in self.manifest["nodes"]:
            name = node["name"]
            self.wait_for_node(name)
            self.check_cloud_init(name)
            self.check_host_key(name)
            self.check_account(name)
            self.check_system(name)
        for node, dst, expected in text_file_targets(self.manifest):
            self.check_text_file(node, dst, expected)
        for node in provisioned_nodes(self.manifest):
            self.check_provisioned(node)
        for node in self.manifest["nodes"]:
            self.configure(node["name"])
        self.check_pings()

    def wait_for_node(self, node, timeout=READY_TIMEOUT):
        deadline = time.monotonic() + timeout
        while self.node(f"{node} reachable", node, "true", check=False).code != 0:
            require(time.monotonic() < deadline, f"{node} not reachable over SSH after {timeout}s")
            time.sleep(POLL_INTERVAL)

    def check_cloud_init(self, node):
        # Exit 0 is a clean finish; 2 would mean warnings, which the image is built to avoid.
        outcome = self.node(f"{node} cloud-init finished", node, "cloud-init status --wait --long",
                            timeout=READY_TIMEOUT, check=False)
        require(outcome.code == 0, f"{node}: cloud-init did not finish cleanly "
                f"(exit {outcome.code}): {outcome.output.strip()}")

    def check_host_key(self, node):
        """The host key Sherpa pinned is the node's key on disk, so it survives an sshd restart."""
        outcome = self.node(f"{node} host key on disk", node,
                            "cut -d' ' -f1,2 /etc/ssh/ssh_host_ed25519_key.pub")
        on_disk = outcome.stdout.strip()
        known_hosts = (self.directory / "sherpa_known_hosts").read_text().splitlines()
        pinned = [line.split(None, 1)[1] for line in known_hosts if line.strip()]
        require(on_disk and on_disk in pinned,
                f"{node}: the pinned host key is not the key on disk ({on_disk!r})")

    def check_account(self, node):
        """The lab owner logs in with NixOS's bash, has its own group, and gets sudo from wheel."""
        outcome = self.node(f"{node} lab owner account", node,
                            'u=$(id -un); getent passwd "$u" | cut -d: -f7; id -gn; id -Gn; '
                            'sudo -n true && echo sudo-ok')
        shell, primary, groups, sudo = (outcome.stdout.splitlines() + [""] * 4)[:4]
        require(shell == NIXOS_SHELL, f"{node}: lab owner's shell is {shell!r}, not {NIXOS_SHELL}")
        require("wheel" in groups.split(), f"{node}: lab owner is not in wheel: {groups!r}")
        require(primary not in ("users", "wheel", ""),
                f"{node}: lab owner has no group of its own: {primary!r}")
        require(sudo == "sudo-ok", f"{node}: lab owner has no passwordless sudo")

    def check_system(self, node):
        """eth names, password logins allowed by the image, and no failed units."""
        outcome = self.node(f"{node} system", node,
                            "ip -o link show eth0 >/dev/null && echo eth0-ok; "
                            "sudo -n sshd -T | grep -i '^passwordauthentication '; "
                            "systemctl --failed --no-legend --plain | wc -l")
        # sshd -T prints keywords in lower case on some builds and as configured on others.
        lines = outcome.stdout.lower().splitlines()
        require("eth0-ok" in lines, f"{node}: no eth0 interface")
        require("passwordauthentication yes" in lines,
                f"{node}: the image does not allow password logins: {outcome.stdout.strip()!r}")
        require(lines and lines[-1].strip() == "0", f"{node}: failed systemd units: {lines[-1:]}")

    def check_provisioned(self, node):
        """`sherpa up` ran the SSH provisioner's script as the lab owner, after cloud-init."""
        outcome = self.node(f"{node} provisioned", node, 'echo "$(id -un)@$(hostname)"; cat ~/provisioned.txt')
        lines = outcome.stdout.splitlines()
        require(len(lines) == 2 and lines[0] == lines[1],
                f"{node}: the SSH provisioner's script did not run as the lab owner: {lines!r}")

    def check_text_file(self, node, dst, expected):
        outcome = self.node(f"{node} text file {dst}", node, f"sudo -n stat -c '%U:%G %a' {shlex.quote(dst)}")
        require(outcome.stdout.strip() == expected,
                f"{node}: {dst} is {outcome.stdout.strip()!r}, not {expected!r}")

    def configure(self, node):
        addresses = [item for item in self.addresses if item.node == node]
        if not addresses:
            return
        outcome = self.node(f"address {node}", node, configure_command(addresses), check=False)
        require(outcome.code == 0, f"could not address {node} {[a.interface for a in addresses]}: "
                f"{outcome.output.strip()}")

    def check_pings(self):
        failed = [f"{source.node}:{source.interface} -> {target.node}:{target.interface} ({host(target)})"
                  for source, target in self.pings if not self.ping(source, target)]
        require(not failed, f"no reply on: {'; '.join(failed)}")

    def ping(self, source, target):
        for attempt in range(PING_ATTEMPTS):
            outcome = self.node(f"ping {source.node} -> {target.node} {host(target)}", source.node,
                                f"ping -c 1 -W 2 -I {source.interface} {host(target)}", check=False)
            if outcome.code == 0:
                return True
            time.sleep(1 + attempt)
        return False

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
        """When run on the Sherpa server, check its libvirt domains and host interfaces."""
        checks = (
            ("libvirt domains", ["virsh", "-c", "qemu:///system", "list", "--all", "--name"]),
            ("host interfaces", ["ip", "-brief", "link"]),
        )
        for kind, arguments in checks:
            if not shutil.which(arguments[0]):
                self.log.write(f"=== {kind} leftover check skipped: {arguments[0]} not found\n")
                continue
            listing = self.command(f"no leftover {kind}", arguments, check=False)
            if listing.code != 0:
                self.log.write(f"=== {kind} leftover check skipped: exit {listing.code}\n")
                continue
            leftovers = [line.split()[0] for line in listing.stdout.splitlines()
                         if self.lab_id in line]
            require(not leftovers, f"{kind} left after destroy: {leftovers}")


def preflight(run_root):
    for tool in ("sherpa", "ssh"):
        require(shutil.which(tool), f"{tool} is not on PATH")
    manifest = tomllib.loads((LAB / "manifest.toml").read_text())
    with (run_root / "preflight.log").open("w") as log:
        whoami = run_logged(log, "sherpa whoami", ["sherpa", "whoami"], run_root)
        require(whoami.code == 0, "the sherpa CLI is not logged in (run `sherpa login`)")
        images = sorted({(node["model"], node["version"]) for node in manifest["nodes"]})
        for model, version in images:
            image = run_logged(log, f"{model} {version} image present",
                               ["sherpa", "image", "show", model, "--version", version], run_root)
            require(image.code == 0, f"the server has no {model} {version} image "
                    "(build it from images/nixos and import it)")


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
    run_root = RUNS / f"nixos-linux-{uuid.uuid4().hex[:12]}"
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
        self.assertTrue(rendered["provision_current_user"])
        for section in ("nodes", "links"):
            self.assertEqual(rendered[section], manifest[section])

    def test_every_node_is_a_pinned_nixos_vm(self):
        for node in self.manifest()["nodes"]:
            self.assertEqual(node["model"], "nixos_linux")
            self.assertTrue(node["version"])

    def test_link_ends_get_unique_addresses_and_ping_each_other(self):
        addresses, pings = link_plan(self.manifest())
        self.assertEqual(len(addresses), 2)
        self.assertEqual(len({item.address for item in addresses}), 2)
        self.assertIn((Address("nix01", "eth1", "10.46.0.1/24"), Address("nix02", "eth1", "10.46.0.2/24")),
                      pings)
        self.assertEqual(len(pings), 2)

    def test_text_file_is_owned_by_its_user_and_group(self):
        self.assertEqual(text_file_targets(self.manifest()),
                         [("nix01", "/home/sherpa/hello.txt", "sherpa:sherpa 600")])

    def test_provisioner_script_matches_the_manifest(self):
        self.assertEqual(provisioned_nodes(self.manifest()), ["nix01"])
        scripts = {script for node in self.manifest()["nodes"]
                   for item in node.get("provisioners", []) for script in item["scripts"]}
        self.assertEqual(scripts, {PROVISION_SCRIPT})

    def test_text_file_source_matches_the_manifest(self):
        sources = {item["src"] for node in self.manifest()["nodes"] for item in node.get("text_files", [])}
        self.assertEqual(sources, {TEXT_FILE})

    def test_configure_command_addresses_each_interface_with_sudo(self):
        command = configure_command([Address("nix01", "eth1", "10.46.0.1/24")])
        self.assertIn("sudo -n ip link set eth1 up", command)
        self.assertIn("sudo -n ip addr replace 10.46.0.1/24 dev eth1", command)


if __name__ == "__main__":
    sys.exit(main())
