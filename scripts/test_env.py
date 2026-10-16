#!/usr/bin/env python3
"""Bring up, check and destroy Sherpa test environments (Python 3.11+, standard library only)."""

import argparse
from pathlib import Path
import shlex
import subprocess
import sys
import tomllib
import unittest


ROOT = Path(__file__).resolve().parent.parent
ENVIRONMENTS = ROOT / "test-environments"
GUEST_SCRIPTS = ("vm_release_guest.py", "test_env_guest.py")
GUEST_DIRECTORY = "/tmp/sherpa-test-env"
CONNECT_TIMEOUT = 10


def require(condition, message):
    if not condition:
        raise RuntimeError(message)


def environment_directory(name):
    directory = ENVIRONMENTS / name
    require(directory.parent == ENVIRONMENTS and (directory / "manifest.toml").is_file(),
            f"Unknown test environment: {name}")
    return directory


def environment_node(manifest):
    nodes = manifest.get("nodes", [])
    require(len(nodes) == 1 and nodes[0].get("model") == "ubuntu_linux",
            "Test environments require a single ubuntu_linux node")
    node = nodes[0]
    for key in ("version", "cpu_count", "boot_disk_size"):
        require(key in node, f"Test environment node must set {key} explicitly")
    return node


def lab_id(directory, manifest):
    path = directory / "lab-info.toml"
    require(path.is_file(), f"No lab is up for {directory.name}; run: test_env.py up {directory.name}")
    info = tomllib.loads(path.read_text())
    require(info.get("name") == manifest["name"],
            f"lab-info.toml belongs to lab {info.get('name')!r}, not {manifest['name']!r}")
    return info["id"]


def run(command, directory):
    print("+ " + shlex.join(command), flush=True)
    process = subprocess.run(command, cwd=directory, stdin=subprocess.DEVNULL)
    require(process.returncode == 0, f"Command failed ({process.returncode}): {shlex.join(command)}")


def ssh_options(directory):
    return ["-F", str(directory / "sherpa_ssh_config"), "-o", "BatchMode=yes",
            "-o", f"ConnectTimeout={CONNECT_TIMEOUT}"]


def guest_command(node):
    return ["sudo", "-n", "python3", f"{GUEST_DIRECTORY}/test_env_guest.py",
            "--version", node["version"], "--cpu-count", str(node["cpu_count"]),
            "--boot-disk-size", str(node["boot_disk_size"])]


def check(directory):
    manifest = tomllib.loads((directory / "manifest.toml").read_text())
    node = environment_node(manifest)
    target = f"{node['name']}.{lab_id(directory, manifest)}"
    options = ssh_options(directory)
    run(["ssh", *options, target, shlex.join(["mkdir", "-p", GUEST_DIRECTORY])], directory)
    run(["scp", "-q", *options, *(str(ROOT / "scripts" / script) for script in GUEST_SCRIPTS),
         f"{target}:{GUEST_DIRECTORY}/"], directory)
    run(["ssh", *options, target, shlex.join(guest_command(node))], directory)
    print(f"PASS test environment {directory.name} ({target})")


def up(directory):
    environment_node(tomllib.loads((directory / "manifest.toml").read_text()))
    run(["sherpa", "up"], directory)
    check(directory)


def destroy(directory):
    lab_id(directory, tomllib.loads((directory / "manifest.toml").read_text()))
    run(["sherpa", "destroy", "--yes"], directory)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=("up", "check", "destroy"))
    parser.add_argument("environment", help="Directory name under test-environments/")
    args = parser.parse_args()
    directory = environment_directory(args.environment)
    {"up": up, "check": check, "destroy": destroy}[args.action](directory)


class HostTests(unittest.TestCase):
    def test_node_requires_single_pinned_ubuntu_node(self):
        node = {"name": "testbox", "model": "ubuntu_linux", "version": "26.04",
                "cpu_count": 8, "boot_disk_size": 100}
        self.assertEqual(environment_node({"nodes": [node]}), node)
        for manifest in ({"nodes": []}, {"nodes": [node, node]},
                         {"nodes": [{**node, "model": "devbox_linux"}]},
                         {"nodes": [{key: value for key, value in node.items() if key != "version"}]}):
            with self.subTest(manifest=manifest), self.assertRaises(RuntimeError):
                environment_node(manifest)

    def test_unknown_environment_is_rejected(self):
        for name in ("missing", "../scripts", "ubuntu-2604/../.."):
            with self.subTest(name=name), self.assertRaises(RuntimeError):
                environment_directory(name)

    def test_guest_command_passes_manifest_expectations(self):
        node = {"version": "24.04-20260926", "cpu_count": 8, "boot_disk_size": 100}
        self.assertEqual(guest_command(node)[-6:],
                         ["--version", "24.04-20260926", "--cpu-count", "8", "--boot-disk-size", "100"])


if __name__ == "__main__":
    if sys.argv[1:] == ["--self-test"]:
        unittest.main(argv=[sys.argv[0]])
    else:
        try:
            main()
        except (Exception, KeyboardInterrupt) as error:
            print(f"FAIL: {error}", file=sys.stderr)
            sys.exit(1)
