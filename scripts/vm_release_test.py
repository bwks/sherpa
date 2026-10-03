#!/usr/bin/env python3
"""Local Sherpa VM release-test runner (Python 3.11+, standard library only)."""

import argparse
from contextlib import redirect_stdout
import hashlib
import io
import json
import os
from pathlib import Path, PurePosixPath
import re
import secrets
import shlex
import signal
import subprocess
import sys
import tarfile
import tempfile
import time
import tomllib
import unittest
from unittest.mock import patch
import uuid
from urllib.parse import urlsplit


ROOT = Path(__file__).resolve().parent.parent
RUNNER_SOURCE = Path(__file__).read_bytes()
STEPS = ("baseline", "preflight", "install", "initialize", "authenticate",
         "reboot", "authenticate", "reinstall", "authenticate",
         "keep-data", "restore", "authenticate", "remove-data",
         "new-database", "authenticate", "remove-all", "repeat-uninstall")


def value_toml(value):
    if isinstance(value, bool):
        return str(value).lower()
    if isinstance(value, (str, int, float, list)):
        return json.dumps(value, ensure_ascii=False)
    raise ValueError(f"Unsupported TOML value: {type(value).__name__}")


def encode_toml(data):
    lines = []
    for section, values in data.items():
        lines.append(f"[{section}]")
        lines.extend(f"{key} = {value_toml(value)}" for key, value in values.items())
        lines.append("")
    return "\n".join(lines)


def sha256(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def archive_binary_hash(path, binary):
    with tarfile.open(path, "r:gz") as archive:
        members = archive.getmembers()
        for member in members:
            name = PurePosixPath(member.name)
            if name.is_absolute() or ".." in name.parts or not member.isfile():
                raise ValueError(f"Unsafe archive entry: {member.name}")
        matches = [member for member in members if PurePosixPath(member.name) == PurePosixPath(binary)]
        if len(matches) != 1:
            raise ValueError(f"Archive must contain exactly one {binary} at its root")
        with archive.extractfile(matches[0]) as stream:
            return hashlib.file_digest(stream, "sha256").hexdigest()


def check_lab(info, expected_name, expected_id):
    if info["name"] != expected_name or info["id"] != expected_id:
        raise ValueError("Refusing cleanup: lab ownership changed")


def jump_host_arguments(value):
    # OpenSSH brackets IPv4 as well as IPv6 in its expanded ProxyJump output.
    value = re.sub(r"\[([0-9]+(?:\.[0-9]+){3})\]", r"\1", value)
    parsed = urlsplit("ssh://" + value)
    if not parsed.hostname or parsed.hostname.startswith("-") or parsed.password or parsed.path:
        raise ValueError("Invalid SSH jump host")
    arguments = []
    if parsed.port:
        arguments.extend(["-p", str(parsed.port)])
    if parsed.username:
        arguments.extend(["-l", parsed.username])
    return arguments + [parsed.hostname]


def load_config(path):
    config = tomllib.loads(path.read_text())
    return validate_config(config)


def validate_config(config):
    required = {
        "host": ("sherpa", "server_url", "libvirt_uri"),
        "vm": ("manifest", "lab_prefix", "node_prefix"),
        "image": ("path", "build", "sha256"),
        "candidate": ("version", "artifact_directory", "target", "download_url", "scripts_directory"),
        "runner": ("output_directory", "retain_failed", "retain_successful"),
        "guest": ("workspace_prefix", "listen_ip", "client_host", "ws_port", "http_port", "db_port",
                  "admin_prefix", "surrealdb_image", "router_image"),
        "timeouts": ("provision", "connect", "command", "download", "install", "initialize",
                     "startup", "reboot", "cleanup", "poll", "terminate"),
    }
    if set(config) != set(required):
        raise ValueError("Unknown or missing configuration section")
    for section, keys in required.items():
        if set(config[section]) != set(keys):
            raise ValueError(f"Unknown or missing settings in [{section}]")
    for key, value in config["timeouts"].items():
        if type(value) is not int or value <= 0:
            raise ValueError(f"Timeout {key} must be a positive integer")
    if not re.fullmatch(r"v\d+\.\d+\.\d+(?:[-.][A-Za-z0-9.]+)?", config["candidate"]["version"]):
        raise ValueError("Select an explicit release version, such as v0.3.79")
    if not re.fullmatch(r"[0-9a-f]{64}", config["image"]["sha256"]):
        raise ValueError("Image SHA-256 must contain 64 lowercase hex digits")
    for key in ("lab_prefix", "node_prefix"):
        if not re.fullmatch(r"[a-z][a-z0-9-]*", config["vm"][key]):
            raise ValueError(f"Invalid {key}")
    for key in ("ws_port", "http_port", "db_port"):
        if type(config["guest"][key]) is not int or not 1 <= config["guest"][key] <= 65535:
            raise ValueError(f"Invalid {key}")
    return config


class Runner:
    def __init__(self, config, keep_vm=False):
        self.config = config
        self.run_id = uuid.uuid4().hex
        self.directory = (ROOT / config["runner"]["output_directory"] / self.run_id).resolve()
        self.directory.mkdir(parents=True, mode=0o700)
        # Custom output paths inside a checkout must also ignore credentials.
        (self.directory / ".gitignore").write_text("*\n")
        self.lab_name = config["vm"]["lab_prefix"] + self.run_id[:8]
        self.node_name = config["vm"]["node_prefix"] + self.run_id[:8]
        self.lab_id = ""
        self.guest_uuid = ""
        self.jump_host = ""
        self.keep_vm = keep_vm
        self.secrets = []
        self.sequence = 0
        self.report = {
            "run": {"id": self.run_id, "status": "running", "directory": str(self.directory),
                    "lab_name": self.lab_name, "node_name": self.node_name, "retained": False,
                    "completed_steps": []},
            "candidate": dict(config["candidate"]),
            "image": dict(config["image"]),
            "scripts": {},
            "artifacts": {},
        }
        self.save()

    def redact(self, output):
        for secret in self.secrets:
            output = output.replace(secret, "[REDACTED]")
        return output

    def save(self):
        (self.directory / "result.toml").write_text(encode_toml(self.report))

    def command(self, label, command, timeout, cwd=None, input_text=None, check=True):
        self.sequence += 1
        log = self.directory / f"{self.sequence:02d}-{label}.log"
        process = subprocess.Popen(command, cwd=cwd, stdin=subprocess.PIPE,
                                   stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                                   text=True, start_new_session=True)
        try:
            output, _ = process.communicate(input_text, timeout=timeout)
        except (subprocess.TimeoutExpired, KeyboardInterrupt):
            os.killpg(process.pid, signal.SIGTERM)
            try:
                output, _ = process.communicate(timeout=self.config["timeouts"]["terminate"])
            except subprocess.TimeoutExpired:
                os.killpg(process.pid, signal.SIGKILL)
                output, _ = process.communicate()
            log.write_text(self.redact(output))
            raise
        log.write_text(self.redact(output))
        if check and process.returncode:
            raise RuntimeError(f"{label} exited {process.returncode}; see {log}")
        return process.returncode, output

    def sherpa(self, label, arguments, timeout):
        command = [self.config["host"]["sherpa"], *arguments]
        if self.config["host"]["server_url"]:
            command.extend(["--server-url", self.config["host"]["server_url"]])
        return self.command(label, command, timeout, cwd=self.directory)

    def ssh(self, label, command, timeout, check=True):
        return self.command(label, ["ssh", "-F", str(self.directory / "sherpa_ssh_config"),
                                  "-o", "BatchMode=yes", "-o",
                                  f"ConnectTimeout={self.config['timeouts']['connect']}",
                                  f"{self.node_name}.{self.lab_id}", shlex.join(command)],
                            timeout, check=check)

    def host(self, label, command):
        return self.command(label, ["ssh", "-o", "BatchMode=yes", "-o",
                                   f"ConnectTimeout={self.config['timeouts']['connect']}",
                                   *jump_host_arguments(self.jump_host), shlex.join(command)],
                            self.config["timeouts"]["command"])

    def inputs(self):
        candidate = self.config["candidate"]
        _, status = self.sherpa("host-status", ["server", "status"], self.config["timeouts"]["command"])
        urls = re.findall(r"\bwss?://[^\s│┃]+", status)
        if len(urls) != 1 or "online" not in status:
            raise RuntimeError("Could not resolve an online Sherpa host")
        self.config["host"]["server_url"] = urls[0]
        self.report["host"] = dict(self.config["host"])
        self.report["host"]["cli_version"] = self.command(
            "host-cli-version", [self.config["host"]["sherpa"], "--version"],
            self.config["timeouts"]["command"])[1].strip()
        (self.directory / "runner-config.toml").write_text(encode_toml(self.config))
        self.report["candidate"]["commit"] = self.command(
            "source-commit", ["git", "-C", str(ROOT), "rev-parse", "HEAD"],
            self.config["timeouts"]["command"])[1].strip()
        self.report["candidate"]["dirty"] = bool(self.command(
            "source-status", ["git", "-C", str(ROOT), "status", "--porcelain"],
            self.config["timeouts"]["command"])[1].strip())
        for name in ("sherpa_install.sh", "sherpa_uninstall.sh"):
            source = ROOT / candidate["scripts_directory"] / name
            (self.directory / name).write_bytes(source.read_bytes())
            self.report["scripts"][name.replace(".", "_")] = sha256(self.directory / name)
        for name in ("vm_release_test.py", "vm_release_guest.py"):
            path = self.directory / name
            path.write_bytes(RUNNER_SOURCE if name == "vm_release_test.py" else (ROOT / "scripts" / name).read_bytes())
            self.report["scripts"][name.replace(".", "_")] = sha256(path)
        for binary in ("sherpad", "sherpa"):
            asset = f"{binary}-{candidate['target']}.tar.gz"
            path = self.directory / asset
            if candidate["artifact_directory"]:
                source = ROOT / candidate["artifact_directory"] / asset
                path.write_bytes(source.read_bytes())
            else:
                url = f"{candidate['download_url']}/{candidate['version']}/{asset}"
                self.command(f"download-{binary}", ["curl", "-fSL", "--connect-timeout",
                             str(self.config["timeouts"]["connect"]), "--max-time",
                             str(self.config["timeouts"]["download"]), "-o", str(path), url],
                             self.config["timeouts"]["download"])
            self.report["artifacts"][f"{binary}_archive_sha256"] = sha256(path)
            self.report["artifacts"][f"{binary}_binary_sha256"] = archive_binary_hash(path, binary)
        self.save()

    def provision(self):
        manifest = tomllib.loads((ROOT / self.config["vm"]["manifest"]).read_text())
        if len(manifest["nodes"]) != 1 or manifest["nodes"][0]["model"] != "ubuntu_linux":
            raise ValueError("The runner requires a dedicated single-Ubuntu manifest")
        node = dict(manifest["nodes"][0])
        node["name"] = self.node_name
        text = f"name = {value_toml(self.lab_name)}\nready_timeout = {manifest['ready_timeout']}\n\n[[nodes]]\n"
        text += "\n".join(f"{key} = {value_toml(value)}" for key, value in node.items()) + "\n"
        (self.directory / "manifest.toml").write_text(text)
        self.report["vm"] = node
        self.sherpa("validate", ["validate"], self.config["timeouts"]["command"])
        # A failed up call can leave resources before lab-info.toml is written.
        self.report["run"]["retained"] = True
        self.report["run"]["resource_state"] = "provisioning"
        self.save()
        print(f"[provision] Creating fresh lab {self.lab_name}", flush=True)
        try:
            self.sherpa("up", ["up"], self.config["timeouts"]["provision"])
        finally:
            info_path = self.directory / "lab-info.toml"
            if info_path.exists():
                info = tomllib.loads(info_path.read_text())
                if info["name"] != self.lab_name:
                    raise ValueError("Unexpected lab returned by Sherpa")
                self.lab_id = info["id"]
                self.report["run"]["lab_id"] = self.lab_id
                self.report["run"]["retained"] = True
                self.report["run"]["resource_state"] = "allocated"
                self.save()
        _, expanded = self.command("ssh-settings", ["ssh", "-G", "-F",
                       str(self.directory / "sherpa_ssh_config"), f"{self.node_name}.{self.lab_id}"],
                       self.config["timeouts"]["command"])
        settings = dict(line.split(" ", 1) for line in expanded.splitlines() if " " in line)
        self.jump_host = settings["proxyjump"]
        if self.jump_host == "none" or "," in self.jump_host:
            raise ValueError("A single Sherpa SSH jump host is required")
        self.guest_uuid = self.host("domain-uuid", ["virsh", "-c", self.config["host"]["libvirt_uri"],
                                    "domuuid", f"{self.node_name}-{self.lab_id}"])[1].strip().lower()
        uuid.UUID(self.guest_uuid)
        actual_hash = self.host("base-image", ["sha256sum", self.config["image"]["path"]])[1].split()[0]
        if actual_hash != self.config["image"]["sha256"]:
            raise ValueError("The server's base image checksum differs from the configured build")
        self.report["run"]["guest_uuid"] = self.guest_uuid
        self.report["run"]["jump_host"] = self.jump_host
        domain = f"{self.node_name}-{self.lab_id}"
        _, disks = self.host("domain-disks", ["virsh", "-c", self.config["host"]["libvirt_uri"],
                                            "domblklist", domain, "--details"])
        sources = [line.split()[3] for line in disks.splitlines()
                   if len(line.split()) == 4 and line.split()[0] == "file"]
        if not sources or any(not Path(path).name.startswith((domain + "-", domain + ".")) for path in sources):
            raise ValueError("Unexpected disk ownership in the test domain")
        self.report["run"]["owned_disks"] = sources
        self.save()

    def transfer(self):
        remote = self.config["guest"]["workspace_prefix"] + "-" + self.run_id
        guard = ("from pathlib import Path; import socket; "
                 f"assert Path('/sys/class/dmi/id/product_uuid').read_text().strip().lower() == {self.guest_uuid!r}; "
                 f"assert socket.gethostname().split('.')[0] == {self.node_name!r}; "
                 f"Path({remote!r}).mkdir(mode=0o700)")
        self.ssh("create-workspace", ["sudo", "-n", "python3", "-c", guard],
                 self.config["timeouts"]["command"])
        # The copied workspace belongs to the SSH login user, not to the service installation.
        self.ssh("workspace-owner", ["sudo", "-n", "chown", "--reference=.", remote],
                 self.config["timeouts"]["command"])
        database_password = "Aa1!" + secrets.token_hex(24)
        admin_password = "Bb2!" + secrets.token_hex(24)
        self.secrets.extend([database_password, admin_password])
        payload = {
            "identity": {"uuid": self.guest_uuid, "hostname": self.node_name, "nonce": self.run_id,
                         "workspace": remote},
            "candidate": {**self.config["candidate"], **self.report["artifacts"]},
            "guest": dict(self.config["guest"]),
            "vm": dict(self.report["vm"]),
            "timeouts": dict(self.config["timeouts"]),
            "credentials": {"db_password": database_password, "admin_password": admin_password,
                            "admin_username": self.config["guest"]["admin_prefix"] + self.run_id[:8]},
        }
        payload_path = self.directory / "guest.toml"
        payload_path.write_text(encode_toml(payload))
        payload_path.chmod(0o600)
        files = [str(payload_path), str(self.directory / "vm_release_guest.py"),
                 str(self.directory / "sherpa_install.sh"), str(self.directory / "sherpa_uninstall.sh")]
        if self.config["candidate"]["artifact_directory"]:
            files.extend(str(path) for path in self.directory.glob("*.tar.gz"))
        self.command("upload", ["scp", "-F", str(self.directory / "sherpa_ssh_config"), "-o",
                               "BatchMode=yes", *files, f"{self.node_name}.{self.lab_id}:{remote}/"],
                     self.config["timeouts"]["download"])
        self.remote_directory = remote

    def step(self, name):
        print(f"[test] {name}", flush=True)
        timeout = self.config["timeouts"]["command"]
        if name in ("install", "reinstall", "restore", "new-database"):
            timeout = self.config["timeouts"]["install"]
        elif name in ("initialize", "baseline"):
            timeout = self.config["timeouts"]["initialize"]
        elif name == "authenticate":
            timeout = self.config["timeouts"]["startup"]
        command = ["sudo", "-n", "timeout", "--signal=TERM",
                   f"--kill-after={self.config['timeouts']['terminate']}", str(timeout),
                   "python3", f"{self.remote_directory}/vm_release_guest.py",
                   f"{self.remote_directory}/guest.toml", name]
        self.ssh(name, command, timeout + self.config["timeouts"]["terminate"])
        if name != "diagnostics":
            self.report["run"]["completed_steps"].append(name)
        self.save()
        print(f"[pass] {name}", flush=True)

    def reboot(self):
        old_boot = self.ssh("boot-before", ["cat", "/proc/sys/kernel/random/boot_id"],
                            self.config["timeouts"]["command"])[1].strip()
        self.step("reboot")
        deadline = time.monotonic() + self.config["timeouts"]["reboot"]
        while time.monotonic() < deadline:
            code, output = self.ssh("boot-after", ["cat", "/proc/sys/kernel/random/boot_id"],
                                    self.config["timeouts"]["connect"] + self.config["timeouts"]["terminate"],
                                    check=False)
            if code == 0 and output.strip() != old_boot:
                return
            time.sleep(self.config["timeouts"]["poll"])
        raise TimeoutError("Guest did not return with a new boot ID")

    def collect(self):
        if not self.guest_uuid or not hasattr(self, "remote_directory"):
            return
        try:
            self.step("diagnostics")
        except (RuntimeError, subprocess.TimeoutExpired):
            print("[diagnostics] Guest collection failed; provisioning and step logs are retained", flush=True)
        self.sherpa("inspect-final", ["inspect"], self.config["timeouts"]["command"])

    def cleanup(self):
        if not self.lab_id or not self.guest_uuid:
            raise ValueError("Cannot verify provisioned lab ownership; retain it for inspection")
        info = tomllib.loads((self.directory / "lab-info.toml").read_text())
        check_lab(info, self.lab_name, self.lab_id)
        manifest = tomllib.loads((self.directory / "manifest.toml").read_text())
        if manifest["name"] != self.lab_name or manifest["nodes"][0]["name"] != self.node_name:
            raise ValueError("Refusing cleanup: manifest ownership changed")
        actual = self.host("cleanup-uuid", ["virsh", "-c", self.config["host"]["libvirt_uri"],
                           "domuuid", f"{self.node_name}-{self.lab_id}"])[1].strip().lower()
        if actual != self.guest_uuid:
            raise ValueError("Refusing cleanup: domain identity changed")
        print(f"[cleanup] Destroying owned lab {self.lab_name}", flush=True)
        self.sherpa("destroy", ["destroy", "--yes"], self.config["timeouts"]["cleanup"])
        _, names = self.host("cleanup-domains", ["virsh", "-c", self.config["host"]["libvirt_uri"],
                                               "list", "--all", "--name"])
        if f"{self.node_name}-{self.lab_id}" in names.splitlines():
            raise RuntimeError("Owned VM remains after cleanup")
        disk_check = ("from pathlib import Path; "
                      f"paths = {self.report['run']['owned_disks']!r}; "
                      "remaining = [path for path in paths if Path(path).exists()]; "
                      "print('Remaining owned disks:', remaining); "
                      "raise SystemExit(bool(remaining))")
        self.host("cleanup-disks", ["python3", "-c", disk_check])
        actual_hash = self.host("base-after-cleanup", ["sha256sum", self.config["image"]["path"]])[1].split()[0]
        if actual_hash != self.config["image"]["sha256"]:
            raise RuntimeError("Base image changed during the run")
        self.report["run"]["retained"] = False
        self.report["run"]["resource_state"] = "removed"
        # Delete only local test credentials, retaining logs, inputs and the run receipt.
        for name in ("guest.toml", "sherpa_ssh_key"):
            (self.directory / name).unlink(missing_ok=True)

    def run(self, scenario):
        success = False
        required = STEPS if scenario == "lifecycle" else STEPS[:2]
        self.report["run"]["scenario"] = scenario
        try:
            self.inputs()
            self.provision()
            self.transfer()
            for step in required:
                if step == "reboot":
                    self.reboot()
                else:
                    self.step(step)
            if tuple(self.report["run"]["completed_steps"]) != required:
                raise RuntimeError("Required steps are missing")
            success = True
        except (Exception, KeyboardInterrupt) as error:
            self.report["run"]["error"] = self.redact(str(error))
            print(f"[fail] {self.redact(str(error))}", flush=True)
        finally:
            try:
                self.collect()
                retain = self.config["runner"]["retain_successful" if success else "retain_failed"]
                if self.lab_id and not (retain or self.keep_vm):
                    self.cleanup()
            except (Exception, KeyboardInterrupt) as error:
                success = False
                self.report["run"]["cleanup_error"] = self.redact(str(error))
                print(f"[fail] Evidence collection or cleanup failed: {error}", flush=True)
            self.report["run"]["status"] = "passed" if success else "failed"
            self.save()
            print(f"[result] {self.report['run']['status']}: {self.directory / 'result.toml'}", flush=True)
        return 0 if success else 1


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--config", type=Path, default=ROOT / "dev/release-test/config.toml")
    parser.add_argument("--scenario", choices=("lifecycle", "preflight"), default="lifecycle")
    parser.add_argument("--version", help="Override the explicit candidate version in TOML")
    parser.add_argument("--artifact-dir", help="Use local release archives instead of GitHub downloads")
    parser.add_argument("--keep-vm", action="store_true", help="Retain the run's VM even after success")
    parser.add_argument("--self-test", action="store_true", help="Run host-independent harness regression tests")
    args = parser.parse_args()
    if args.self_test:
        unittest.main(argv=[sys.argv[0]])
        return 0
    config = load_config(args.config)
    if args.version:
        config["candidate"]["version"] = args.version
    if args.artifact_dir:
        config["candidate"]["artifact_directory"] = str(Path(args.artifact_dir).resolve())
    validate_config(config)
    return Runner(config, args.keep_vm).run(args.scenario)


class HarnessTests(unittest.TestCase):
    def test_partial_provisioning_is_not_reported_as_clean(self):
        config = load_config(ROOT / "dev/release-test/config.toml")
        with tempfile.TemporaryDirectory() as directory:
            config["runner"]["output_directory"] = directory
            runner = Runner(config)
            def fail_up(label, arguments, timeout):
                if label == "up":
                    raise RuntimeError("interrupted provisioning")
                return 0, ""
            with patch.object(runner, "sherpa", side_effect=fail_up), redirect_stdout(io.StringIO()):
                with self.assertRaises(RuntimeError):
                    runner.provision()
            receipt = tomllib.loads((runner.directory / "result.toml").read_text())
            self.assertTrue(receipt["run"]["retained"])

    def test_custom_output_directory_does_not_expose_credentials_to_git(self):
        config = load_config(ROOT / "dev/release-test/config.toml")
        with tempfile.TemporaryDirectory() as directory:
            subprocess.run(["git", "init", "-q", directory], check=True, capture_output=True)
            config["runner"]["output_directory"] = str(Path(directory) / "results")
            runner = Runner(config)
            key = runner.directory / "guest.toml"
            key.write_text("disposable credentials")
            result = subprocess.run(["git", "-C", directory, "check-ignore", "-q", str(key)])
            self.assertEqual(result.returncode, 0)

    def test_uninstall_removes_only_owned_binary_symlinks(self):
        source = (ROOT / "scripts/sherpa_uninstall.sh").read_text().removesuffix('main "$@"\n')
        with tempfile.TemporaryDirectory() as directory:
            base = Path(directory)
            owned = base / "owned"
            other = base / "other"
            regular = base / "regular"
            owned.symlink_to(base / "missing-owned-binary")
            other.symlink_to(base / "another-installation")
            regular.write_text("another installation")
            code = source + "\n" + "\n".join(
                f"remove_binary_symlink {shlex.quote(str(path))} {shlex.quote(str(base / 'missing-owned-binary'))}"
                for path in (owned, other, regular))
            result = subprocess.run(["bash", "-c", code], capture_output=True, text=True, timeout=5)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertFalse(owned.is_symlink())
            self.assertTrue(other.is_symlink())
            self.assertEqual(regular.read_text(), "another installation")

    def test_installer_selects_concrete_qemu_package(self):
        source = (ROOT / "scripts/sherpa_install.sh").read_text().removesuffix('main "$@"\n')
        code = source + """
apt-get() { printf '%s\\n' "$*"; }
install_system_packages
"""
        result = subprocess.run(["bash", "-c", code], capture_output=True, text=True, timeout=5)
        self.assertEqual(result.returncode, 0)
        self.assertIn("qemu-system-x86", result.stdout)
        self.assertNotIn(" qemu-kvm ", result.stdout)

    def test_jump_host_brackets_and_ports(self):
        self.assertEqual(jump_host_arguments("bradmin@[10.100.58.10]"), ["-l", "bradmin", "10.100.58.10"])
        self.assertEqual(jump_host_arguments("user@[2001:db8::1]:2222"),
                         ["-p", "2222", "-l", "user", "2001:db8::1"])

    def test_missing_version_is_rejected_before_host_changes(self):
        result = subprocess.run(["bash", str(ROOT / "scripts/sherpa_install.sh"), "--version"],
                                capture_output=True, text=True, timeout=5)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("--version requires a version", result.stderr)

    def test_missing_local_artifact_is_rejected_before_host_changes(self):
        with tempfile.TemporaryDirectory() as directory:
            env = dict(os.environ, SHERPA_ARTIFACT_DIR=directory)
            result = subprocess.run(["bash", str(ROOT / "scripts/sherpa_install.sh"),
                                     "--version", "v0.3.79"], env=env,
                                    capture_output=True, text=True, timeout=5)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("artifact not found", result.stderr)

    def test_setup_failure_produces_failed_result_and_nonzero_exit(self):
        config = load_config(ROOT / "dev/release-test/config.toml")
        with tempfile.TemporaryDirectory() as directory:
            config["runner"]["output_directory"] = directory
            runner = Runner(config)
            with patch.object(runner, "inputs", side_effect=RuntimeError("setup failed")), \
                 patch.object(runner, "collect"):
                with redirect_stdout(io.StringIO()):
                    self.assertEqual(runner.run("lifecycle"), 1)
            self.assertEqual(tomllib.loads((runner.directory / "result.toml").read_text())["run"]["status"], "failed")

    def test_command_timeout_is_bounded_and_logged(self):
        config = load_config(ROOT / "dev/release-test/config.toml")
        with tempfile.TemporaryDirectory() as directory:
            config["runner"]["output_directory"] = directory
            runner = Runner(config)
            with self.assertRaises(subprocess.TimeoutExpired):
                runner.command("timeout", [sys.executable, "-c", "import time; time.sleep(30)"], 1)
            self.assertTrue((runner.directory / "01-timeout.log").is_file())

    def test_rejects_archive_traversal_and_symlinks(self):
        for name, kind in (("../sherpad", tarfile.REGTYPE), ("sherpad", tarfile.SYMTYPE)):
            stream = io.BytesIO()
            with tarfile.open(fileobj=stream, mode="w:gz") as archive:
                member = tarfile.TarInfo(name)
                member.type = kind
                member.linkname = "/etc/passwd" if kind == tarfile.SYMTYPE else ""
                member.size = 1 if kind == tarfile.REGTYPE else 0
                archive.addfile(member, io.BytesIO(b"x"))
            with tempfile.TemporaryDirectory() as directory:
                path = Path(directory) / "asset.tar.gz"
                path.write_bytes(stream.getvalue())
                with self.assertRaises(ValueError):
                    archive_binary_hash(path, "sherpad")

    def test_hashes_binary_contents_not_archive(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "asset.tar.gz"
            with tarfile.open(path, "w:gz") as archive:
                member = tarfile.TarInfo("./sherpad")
                member.size = 6
                archive.addfile(member, io.BytesIO(b"binary"))
            self.assertEqual(archive_binary_hash(path, "sherpad"), hashlib.sha256(b"binary").hexdigest())

    def test_report_is_valid_toml(self):
        data = {"run": {"status": "failed", "detail": "quote\"\nline", "retained": True},
                "checks": {"steps": ["baseline", "install"], "count": 2}}
        self.assertEqual(tomllib.loads(encode_toml(data)), data)

    def test_cleanup_rejects_different_lab(self):
        with self.assertRaises(ValueError):
            check_lab({"name": "production", "id": "12345678"}, "release-test", "12345678")
        with self.assertRaises(ValueError):
            check_lab({"name": "release-test", "id": "87654321"}, "release-test", "12345678")


if __name__ == "__main__":
    sys.exit(main())
