#!/usr/bin/env python3
"""Local Sherpa VM release-test runner (Python 3.11+, standard library only)."""

import argparse
from contextlib import redirect_stdout
from contextlib import suppress
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

from vm_release_guest import validate_upgrade_evidence


ROOT = Path(__file__).resolve().parent.parent
RUNNER_SOURCE = Path(__file__).read_bytes()
STEPS = ("baseline", "preflight", "install", "initialize", "authenticate",
         "reboot", "authenticate", "reinstall", "authenticate",
         "keep-data", "restore", "authenticate", "remove-data",
         "new-database", "authenticate", "remove-all", "repeat-uninstall")
UPGRADE_STEPS = ("baseline", "install-baseline", "initialize", "authenticate",
                 "seed-upgrade", "upgrade", "authenticate", "verify-upgrade",
                 "reboot", "authenticate", "verify-upgrade-reboot")


def validate_upgrade_config(config):
    baseline = config.get("baseline", {})
    if set(baseline) != {"version", "artifact_directory"}:
        raise ValueError("Upgrade requires an explicit baseline version and artifact selection")
    if not re.fullmatch(r"v\d+\.\d+\.\d+(?:[-.][A-Za-z0-9.]+)?", baseline["version"]):
        raise ValueError("Select an explicit baseline release version")
    if baseline["version"] == config["candidate"]["version"]:
        raise ValueError("Upgrade requires distinct baseline and candidate versions")
    if not config["candidate"]["artifact_directory"]:
        raise ValueError("Upgrade requires supplied local candidate archives")


def validate_upgrade_artifacts(baseline, candidate):
    for binary in ("sherpa", "sherpad"):
        if baseline[f"{binary}_binary_sha256"] == candidate[f"{binary}_binary_sha256"]:
            raise ValueError(f"Upgrade requires different {binary} baseline and candidate bytes")


def interrupted(signum, _frame):
    raise KeyboardInterrupt(f"Runner interrupted by {signal.Signals(signum).name}")


def inspection_identity(output, name):
    match = re.search(r"Sherpa Environment - " + re.escape(name) + r"-([0-9a-f]{8})\b", output)
    if not match:
        raise ValueError("Cannot resolve the generated lab identity from Sherpa")
    return match[1]


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


def ssh_jump_options(destination):
    if not destination:
        return []
    jump_host_arguments(destination)
    return ["-o", "ProxyJump=" + destination]


def load_config(path):
    config = tomllib.loads(path.read_text())
    return validate_config(config)


def validate_config(config):
    required = {
        "host": ("sherpa", "server_url", "libvirt_uri", "ssh_destination"),
        "vm": ("manifest", "lab_prefix", "node_prefix"),
        "image": ("path", "build", "sha256"),
        "candidate": ("version", "artifact_directory", "target", "download_url", "scripts_directory"),
        "runner": ("output_directory", "retain_failed", "retain_successful"),
        "guest": ("workspace_prefix", "listen_ip", "client_host", "ws_port", "http_port", "db_port",
                  "admin_prefix", "surrealdb_image", "router_image"),
        "timeouts": ("provision", "connect", "command", "download", "install", "initialize",
                     "startup", "reboot", "cleanup", "poll", "terminate"),
        "faults": ("wait", "command_timeout", "ready_timeout", "ready_port", "trigger_timeout"),
    }
    if "baseline" in config:
        required["baseline"] = ("version", "artifact_directory")
    if set(config) != set(required):
        raise ValueError("Unknown or missing configuration section")
    for section, keys in required.items():
        if set(config[section]) != set(keys):
            raise ValueError(f"Unknown or missing settings in [{section}]")
    for key, value in config["timeouts"].items():
        if type(value) is not int or value <= 0:
            raise ValueError(f"Timeout {key} must be a positive integer")
    for key, value in config["faults"].items():
        if type(value) is not int or value <= 0:
            raise ValueError(f"Fault setting {key} must be a positive integer")
    if config["faults"]["ready_port"] > 65535:
        raise ValueError("Invalid fault readiness port")
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
    def __init__(self, config, keep_vm=False, result_file=None):
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
        self.result_file = result_file.resolve() if result_file else None
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

    @classmethod
    def resume(cls, directory):
        directory = directory.resolve()
        report = tomllib.loads((directory / "result.toml").read_text())
        run = report["run"]
        if run["id"] != directory.name or not re.fullmatch(r"[0-9a-f]{32}", run["id"]):
            raise ValueError("Refusing cleanup: result directory identity changed")
        config = load_config(directory / "runner-config.toml")
        if (run["lab_name"] != config["vm"]["lab_prefix"] + run["id"][:8]
                or run["node_name"] != config["vm"]["node_prefix"] + run["id"][:8]):
            raise ValueError("Refusing cleanup: generated resource names changed")
        runner = cls.__new__(cls)
        runner.config, runner.report, runner.directory = config, report, directory
        runner.run_id, runner.lab_name, runner.node_name = run["id"], run["lab_name"], run["node_name"]
        runner.lab_id, runner.guest_uuid, runner.jump_host = run.get("lab_id", ""), run.get("guest_uuid", ""), run.get("jump_host", "")
        runner.keep_vm, runner.secrets = False, []
        runner.sequence = max((int(path.name.split("-", 1)[0]) for path in directory.glob("[0-9]*-*.log")), default=0)
        payload = directory / "guest.toml"
        if payload.exists():
            data = tomllib.loads(payload.read_text())
            validate = data["identity"]
            if validate["uuid"] != runner.guest_uuid or validate["nonce"] != runner.run_id:
                raise ValueError("Refusing cleanup: guest payload identity changed")
            runner.remote_directory = validate["workspace"]
            runner.secrets = [data["credentials"][key] for key in ("db_password", "admin_password")]
        return runner

    def remove_retained(self):
        try:
            if self.report["run"].get("resource_state") == "removed":
                raise ValueError("This run is already recorded as removed")
            if hasattr(self, "remote_directory"):
                self.step("cancel")
            self.cleanup()
            self.report["run"]["cleanup_verified"] = True
            self.save()
            print(f"[cleanup] Verified removal: {self.directory / 'result.toml'}", flush=True)
            return 0
        except (Exception, KeyboardInterrupt) as error:
            self.report["run"]["cleanup_error"] = self.redact(str(error))
            self.save()
            print(f"[fail] Cleanup refused or failed: {error}", flush=True)
            return 1

    def redact(self, output):
        for secret in self.secrets:
            output = output.replace(secret, "[REDACTED]")
        return output

    def save(self):
        temporary = self.directory / ".result.toml.tmp"
        temporary.write_text(encode_toml(self.report))
        temporary.replace(self.directory / "result.toml")
        if getattr(self, "result_file", None):
            self.result_file.parent.mkdir(parents=True, exist_ok=True)
            reference = {"result": {"receipt": str(self.directory / "result.toml"),
                                    "run_id": self.run_id, "status": self.report["run"]["status"]}}
            temporary = self.result_file.with_name("." + self.result_file.name + ".tmp")
            temporary.write_text(encode_toml(reference))
            temporary.replace(self.result_file)

    def command(self, label, command, timeout, cwd=None, input_text=None, check=True):
        self.sequence += 1
        log = self.directory / f"{self.sequence:02d}-{label}.log"
        process = subprocess.Popen(command, cwd=cwd, stdin=subprocess.PIPE,
                                   stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                                   text=True, start_new_session=True)
        try:
            output, _ = process.communicate(input_text, timeout=timeout)
        except (subprocess.TimeoutExpired, KeyboardInterrupt):
            with suppress(ProcessLookupError):
                os.killpg(process.pid, signal.SIGTERM)
            try:
                output, _ = process.communicate(timeout=self.config["timeouts"]["terminate"])
            except subprocess.TimeoutExpired:
                with suppress(ProcessLookupError):
                    os.killpg(process.pid, signal.SIGKILL)
                output, _ = process.communicate()
            log.write_text(self.redact(output))
            raise
        log.write_text(self.redact(output))
        if check and process.returncode:
            raise RuntimeError(f"{label} exited {process.returncode}; see {log}")
        return process.returncode, output

    def sherpa(self, label, arguments, timeout, check=True):
        command = [self.config["host"]["sherpa"], *arguments]
        if self.config["host"]["server_url"]:
            command.extend(["--server-url", self.config["host"]["server_url"]])
        return self.command(label, command, timeout, cwd=self.directory, check=check)

    def ssh(self, label, command, timeout, check=True):
        return self.command(label, ["ssh", "-F", str(self.directory / "sherpa_ssh_config"),
                                  *ssh_jump_options(self.config["host"]["ssh_destination"]),
                                  "-o", "BatchMode=yes", "-o",
                                  f"ConnectTimeout={self.config['timeouts']['connect']}",
                                  f"{self.node_name}.{self.lab_id}", shlex.join(command)],
                            timeout, check=check)

    def host(self, label, command, check=True):
        return self.command(label, ["ssh", "-o", "BatchMode=yes", "-o",
                                   f"ConnectTimeout={self.config['timeouts']['connect']}",
                                   *jump_host_arguments(self.jump_host), shlex.join(command)],
                            self.config["timeouts"]["command"], check=check)

    def inputs(self):
        candidate = self.config["candidate"]
        _, status = self.sherpa("host-status", ["server", "status"], self.config["timeouts"]["command"])
        urls = re.findall(r"\bwss?://[^\s│┃]+", status)
        if len(urls) != 1 or "online" not in status:
            raise RuntimeError("Could not resolve an online Sherpa host")
        self.config["host"]["server_url"] = urls[0]
        self.jump_host = self.config["host"]["ssh_destination"] or urlsplit(urls[0]).hostname
        if not self.jump_host:
            raise ValueError("Cannot resolve the Sherpa SSH host")
        self.report["run"]["jump_host"] = self.jump_host
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
        self.report["artifacts"] = self.prepare_artifacts(candidate, self.directory)
        if self.report["run"]["scenario"] == "upgrade":
            baseline = {**candidate, **self.config["baseline"]}
            baseline_directory = self.directory / "baseline"
            baseline_directory.mkdir(mode=0o700)
            self.report["baseline"] = {**baseline,
                "installer_sha256": self.report["scripts"]["sherpa_install_sh"],
                "installer_source": "recorded-checkout",
                **self.prepare_artifacts(baseline, baseline_directory)}
            validate_upgrade_artifacts(self.report["baseline"], self.report["artifacts"])
        self.save()

    def prepare_artifacts(self, candidate, directory):
        artifacts = {}
        for binary in ("sherpad", "sherpa"):
            asset = f"{binary}-{candidate['target']}.tar.gz"
            path = directory / asset
            if candidate["artifact_directory"]:
                source = ROOT / candidate["artifact_directory"] / asset
                path.write_bytes(source.read_bytes())
            else:
                url = f"{candidate['download_url']}/{candidate['version']}/{asset}"
                self.command(f"download-{candidate['version']}-{binary}", ["curl", "-fSL", "--connect-timeout",
                             str(self.config["timeouts"]["connect"]), "--max-time",
                             str(self.config["timeouts"]["download"]), "-o", str(path), url],
                             self.config["timeouts"]["download"])
            artifacts[f"{binary}_archive_sha256"] = sha256(path)
            artifacts[f"{binary}_binary_sha256"] = archive_binary_hash(path, binary)
        return artifacts

    def provision(self):
        manifest = tomllib.loads((ROOT / self.config["vm"]["manifest"]).read_text())
        if len(manifest["nodes"]) != 1 or manifest["nodes"][0]["model"] != "ubuntu_linux":
            raise ValueError("The runner requires a dedicated single-Ubuntu manifest")
        node = dict(manifest["nodes"][0])
        node["name"] = self.node_name
        if self.report["run"].get("scenario") == "fault-provision":
            manifest["ready_timeout"] = self.config["faults"]["ready_timeout"]
            node["ready_port"] = self.config["faults"]["ready_port"]
        text = f"name = {value_toml(self.lab_name)}\nready_timeout = {manifest['ready_timeout']}\n\n[[nodes]]\n"
        text += "\n".join(f"{key} = {value_toml(value)}" for key, value in node.items()) + "\n"
        (self.directory / "manifest.toml").write_text(text)
        self.report["vm"] = node
        self.sherpa("validate", ["validate"], self.config["timeouts"]["command"])
        code, output = self.sherpa("inspect-before", ["inspect"],
                                   self.config["timeouts"]["command"], check=False)
        if code == 0:
            raise ValueError("Refusing to provision: generated lab already exists")
        self.lab_id = inspection_identity(output, self.lab_name)
        self.report["run"]["lab_id"] = self.lab_id
        _, names = self.host("domains-before", ["virsh", "-c", self.config["host"]["libvirt_uri"],
                                              "list", "--all", "--name"])
        if f"{self.node_name}-{self.lab_id}" in names.splitlines():
            raise ValueError("Refusing to provision: generated domain already exists")
        # A failed up call can leave resources before lab-info.toml is written.
        self.report["run"]["retained"] = True
        self.report["run"]["resource_state"] = "provisioning"
        self.save()
        print(f"[provision] Creating fresh lab {self.lab_name}", flush=True)
        try:
            _, up_output = self.sherpa("up", ["up"], self.config["timeouts"]["provision"])
            if self.report["run"].get("scenario") == "fault-provision":
                self.report["run"]["readiness_timed_out"] = "[NodeReadiness]" in up_output
                self.save()
        finally:
            info_path = self.directory / "lab-info.toml"
            if info_path.exists():
                info = tomllib.loads(info_path.read_text())
                check_lab(info, self.lab_name, self.lab_id)
                self.lab_id = info["id"]
                self.report["run"]["lab_id"] = self.lab_id
                self.report["run"]["retained"] = True
                self.report["run"]["resource_state"] = "allocated"
                self.save()
            # A failed up can have created the domain without writing local SSH files.
            self.bind_vm(required=False)
        _, expanded = self.command("ssh-settings", ["ssh", "-G", "-F",
                       str(self.directory / "sherpa_ssh_config"), f"{self.node_name}.{self.lab_id}"],
                       self.config["timeouts"]["command"])
        settings = dict(line.split(" ", 1) for line in expanded.splitlines() if " " in line)
        self.jump_host = self.config["host"]["ssh_destination"] or settings["proxyjump"]
        if self.jump_host == "none" or "," in self.jump_host:
            raise ValueError("A single Sherpa SSH jump host is required")
        self.bind_vm()

    def bind_vm(self, required=True):
        code, output = self.host("domain-uuid", ["virsh", "-c", self.config["host"]["libvirt_uri"],
                                    "domuuid", f"{self.node_name}-{self.lab_id}"], check=required)
        if code:
            return
        actual_uuid = output.strip().lower()
        if self.guest_uuid and actual_uuid != self.guest_uuid:
            raise ValueError("Refusing to bind a changed domain identity")
        self.guest_uuid = actual_uuid
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
        self.report["run"]["resource_state"] = "allocated"
        self.save()

    def wait_guest(self):
        deadline = time.monotonic() + self.config["timeouts"]["startup"]
        while time.monotonic() < deadline:
            code, _ = self.ssh("guest-ssh-ready", ["true"],
                               self.config["timeouts"]["connect"] + self.config["timeouts"]["terminate"],
                               check=False)
            if code == 0:
                return
            time.sleep(self.config["timeouts"]["poll"])
        raise TimeoutError("Guest SSH did not become ready before workspace creation")

    def transfer(self):
        if self.report["run"]["scenario"] == "upgrade":
            self.wait_guest()
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
            "faults": dict(self.config["faults"]),
            "credentials": {"db_password": database_password, "admin_password": admin_password,
                            "admin_username": self.config["guest"]["admin_prefix"] + self.run_id[:8]},
        }
        if self.report["run"]["scenario"] == "upgrade":
            payload["baseline"] = dict(self.report["baseline"])
        payload_path = self.directory / "guest.toml"
        payload_path.write_text(encode_toml(payload))
        payload_path.chmod(0o600)
        files = [str(payload_path), str(self.directory / "vm_release_guest.py"),
                 str(self.directory / "sherpa_install.sh"), str(self.directory / "sherpa_uninstall.sh")]
        if self.config["candidate"]["artifact_directory"]:
            files.extend(str(path) for path in self.directory.glob("*.tar.gz"))
        if "baseline" in payload:
            files.append(str(self.directory / "baseline"))
        self.command("upload", ["scp", "-r", "-F", str(self.directory / "sherpa_ssh_config"), "-o",
                               "BatchMode=yes", *ssh_jump_options(self.config["host"]["ssh_destination"]),
                               *files, f"{self.node_name}.{self.lab_id}:{remote}/"],
                     self.config["timeouts"]["download"])
        self.remote_directory = remote

    def step(self, name, label=None):
        label = label or name
        print(f"[test] {label}", flush=True)
        self.report["run"]["active_step"] = name
        self.save()
        timeout = self.config["timeouts"]["command"]
        if name in ("install", "reinstall", "restore", "new-database", "install-baseline", "upgrade"):
            timeout = self.config["timeouts"]["install"]
        elif name in ("initialize", "baseline"):
            timeout = self.config["timeouts"]["initialize"]
        elif name in ("authenticate", "seed-upgrade", "verify-upgrade", "verify-upgrade-reboot"):
            timeout = self.config["timeouts"]["startup"]
        elif name == "fault-wait" and self.report["run"]["scenario"] == "fault-timeout":
            timeout = self.config["faults"]["command_timeout"]
        command = ["sudo", "-n", "timeout", "--signal=TERM",
                   f"--kill-after={self.config['timeouts']['terminate']}", str(timeout),
                   "python3", f"{self.remote_directory}/vm_release_guest.py",
                   f"{self.remote_directory}/guest.toml", name]
        code, _ = self.ssh(label, command, timeout + self.config["timeouts"]["terminate"], check=False)
        if code:
            if code == 124:
                raise TimeoutError(f"{label} timed out; see {self.directory}")
            raise RuntimeError(f"{label} exited {code}; see {self.directory}")
        if name not in ("diagnostics", "diagnostics-live", "cancel"):
            self.report["run"]["completed_steps"].append(name)
        if name == "diagnostics-live":
            self.report["run"].setdefault("diagnostics_before", []).append(label.removeprefix("before-"))
        self.report["run"].pop("active_step", None)
        self.save()
        print(f"[pass] {label}", flush=True)

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
        if self.guest_uuid and hasattr(self, "remote_directory"):
            try:
                self.step("diagnostics")
                if self.report["run"]["scenario"] == "upgrade":
                    self.command("upgrade-evidence", ["scp", "-F", str(self.directory / "sherpa_ssh_config"),
                                 "-o", "BatchMode=yes", *ssh_jump_options(self.config["host"]["ssh_destination"]),
                                 f"{self.node_name}.{self.lab_id}:{self.remote_directory}/upgrade-state.toml",
                                 str(self.directory)], self.config["timeouts"]["command"])
                    evidence = tomllib.loads((self.directory / "upgrade-state.toml").read_text())
                    if tuple(self.report["run"]["completed_steps"]) == UPGRADE_STEPS:
                        validate_upgrade_evidence(evidence, self.report["baseline"], self.report["artifacts"])
                    self.report["run"]["upgrade_evidence_sha256"] = sha256(self.directory / "upgrade-state.toml")
                for mode in ("keep-data", "remove-data", "remove-all"):
                    if mode in self.report["run"]["completed_steps"]:
                        self.command("inventory-" + mode, ["scp", "-F", str(self.directory / "sherpa_ssh_config"),
                                     "-o", "BatchMode=yes", *ssh_jump_options(self.config["host"]["ssh_destination"]),
                                     f"{self.node_name}.{self.lab_id}:"
                                     f"{self.remote_directory}/resources-{mode}.toml", str(self.directory)],
                                     self.config["timeouts"]["command"])
            except (RuntimeError, subprocess.TimeoutExpired):
                print("[diagnostics] Guest collection failed; provisioning and step logs are retained", flush=True)
                if tuple(self.report["run"]["completed_steps"]) in (STEPS, UPGRADE_STEPS):
                    raise
        if self.lab_id:
            self.sherpa("inspect-final", ["inspect"], self.config["timeouts"]["command"])

    def cleanup(self):
        if not self.lab_id or not self.guest_uuid:
            raise ValueError("Cannot verify provisioned lab ownership; retain it for inspection")
        info_path = self.directory / "lab-info.toml"
        if info_path.exists():
            check_lab(tomllib.loads(info_path.read_text()), self.lab_name, self.lab_id)
        else:
            _, output = self.sherpa("inspect-partial", ["inspect"], self.config["timeouts"]["command"])
            check_lab({"name": self.lab_name, "id": inspection_identity(output, self.lab_name)},
                      self.lab_name, self.lab_id)
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
        for label, command, targets in (
            ("libvirt-networks", ["virsh", "-c", self.config["host"]["libvirt_uri"], "net-list", "--all", "--name"],
             [f"sherpa-management-{self.lab_id}", f"sherpa-isolated-{self.node_name}-{self.lab_id}"]),
            ("docker-networks", ["docker", "network", "ls", "--format", "{{.Name}}"],
             [f"sherpa-management-{self.lab_id}"]),
            ("router", ["docker", "ps", "-a", "--format", "{{.Names}}"], [f"sherpa-router-{self.lab_id}"]),
        ):
            _, names = self.host("cleanup-" + label, command)
            if any(target in names.splitlines() for target in targets):
                raise RuntimeError(f"Owned {label} remain after cleanup")
        actual_hash = self.host("base-after-cleanup", ["sha256sum", self.config["image"]["path"]])[1].split()[0]
        if actual_hash != self.config["image"]["sha256"]:
            raise RuntimeError("Base image changed during the run")
        self.report["run"]["retained"] = False
        self.report["run"]["resource_state"] = "removed"
        self.report["run"]["cleanup_verified"] = True
        # Delete only local test credentials, retaining logs, inputs and the run receipt.
        for name in ("guest.toml", "sherpa_ssh_key"):
            (self.directory / name).unlink(missing_ok=True)

    def run(self, scenario):
        success = False
        required = STEPS if scenario == "lifecycle" else STEPS[:2]
        if scenario == "upgrade":
            validate_upgrade_config(self.config)
            required = UPGRADE_STEPS
        if scenario in ("fault-timeout", "fault-interrupt"):
            required = ("baseline", "fault-wait")
        self.report["run"]["scenario"] = scenario
        previous = signal.signal(signal.SIGTERM, interrupted)
        try:
            self.inputs()
            self.provision()
            self.transfer()
            for step in required:
                if step == "reboot":
                    self.reboot()
                else:
                    if step in ("keep-data", "remove-data", "remove-all"):
                        self.step("diagnostics-live", label="before-" + step)
                    self.step(step)
            if tuple(self.report["run"]["completed_steps"]) != required:
                raise RuntimeError("Required steps are missing")
            success = True
        except (Exception, KeyboardInterrupt) as error:
            self.report["run"]["failure_kind"] = ("interrupted" if isinstance(error, KeyboardInterrupt)
                                                  else "timeout" if isinstance(error, (TimeoutError, subprocess.TimeoutExpired))
                                                  else "error")
            if isinstance(error, KeyboardInterrupt):
                signal.signal(signal.SIGTERM, signal.SIG_IGN)
            self.report["run"]["error"] = self.redact(str(error))
            self.report["run"]["status"] = "failed"
            self.save()
            print(f"[fail] {self.redact(str(error))}", flush=True)
        finally:
            if not success and hasattr(self, "remote_directory"):
                try:
                    self.step("cancel")
                    self.report["run"]["command_stopped"] = True
                except (Exception, KeyboardInterrupt) as error:
                    self.report["run"]["cancel_error"] = self.redact(str(error))
                    print(f"[fail] Guest cancellation failed: {self.redact(str(error))}", flush=True)
            try:
                self.collect()
            except (Exception, KeyboardInterrupt) as error:
                success = False
                self.report["run"]["evidence_error"] = self.redact(str(error))
                print(f"[fail] Evidence collection failed: {self.redact(str(error))}", flush=True)
            retain = self.config["runner"]["retain_successful" if success else "retain_failed"]
            if self.lab_id and not (retain or self.keep_vm):
                try:
                    self.cleanup()
                except (Exception, KeyboardInterrupt) as error:
                    success = False
                    self.report["run"]["cleanup_error"] = self.redact(str(error))
                    print(f"[fail] Cleanup failed: {self.redact(str(error))}", flush=True)
            self.report["run"]["status"] = "passed" if success else "failed"
            self.save()
            signal.signal(signal.SIGTERM, previous)
            print(f"[result] {self.report['run']['status']}: {self.directory / 'result.toml'}", flush=True)
        return 0 if success else 1


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--config", type=Path, default=ROOT / "dev/release-test/config.toml")
    parser.add_argument("--scenario", choices=("lifecycle", "upgrade", "preflight", "failure-checks", "fault-timeout",
                                               "fault-interrupt", "fault-provision"), default="lifecycle")
    parser.add_argument("--version", help="Override the explicit candidate version in TOML")
    parser.add_argument("--artifact-dir", help="Use local release archives instead of GitHub downloads")
    parser.add_argument("--baseline-version", help="Explicit previous release for the upgrade scenario")
    parser.add_argument("--baseline-artifact-dir", help="Optional local previous-release archives")
    parser.add_argument("--keep-vm", action="store_true", help="Retain the run's VM even after success")
    parser.add_argument("--cleanup-run", type=Path, help="Verify identity and remove a retained run's lab")
    parser.add_argument("--result-file", type=Path, help="Write a nonsecret TOML reference to this run's receipt")
    parser.add_argument("--self-test", action="store_true", help="Run host-independent harness regression tests")
    args = parser.parse_args()
    if args.cleanup_run:
        return Runner.resume(args.cleanup_run).remove_retained()
    if args.self_test:
        unittest.main(argv=[sys.argv[0]])
        return 0
    if args.scenario == "failure-checks":
        command = [sys.executable, str(ROOT / "scripts/vm_release_faults.py"), "--config", str(args.config)]
        if args.version:
            command.extend(["--version", args.version])
        if args.artifact_dir:
            command.extend(["--artifact-dir", args.artifact_dir])
        os.execv(sys.executable, command)
    config = load_config(args.config)
    if args.version:
        config["candidate"]["version"] = args.version
    if args.artifact_dir:
        config["candidate"]["artifact_directory"] = str(Path(args.artifact_dir).resolve())
    if args.baseline_version:
        config["baseline"] = {"version": args.baseline_version,
                              "artifact_directory": str(Path(args.baseline_artifact_dir).resolve())
                              if args.baseline_artifact_dir else ""}
    elif args.baseline_artifact_dir:
        parser.error("--baseline-artifact-dir requires --baseline-version")
    if args.scenario != "upgrade" and (args.baseline_version or args.baseline_artifact_dir):
        parser.error("Baseline inputs require --scenario upgrade")
    validate_config(config)
    return Runner(config, args.keep_vm, args.result_file).run(args.scenario)


class HarnessTests(unittest.TestCase):
    def test_upgrade_waits_for_transient_guest_ssh_failure_without_mutating_it(self):
        config = self.upgrade_config()
        with tempfile.TemporaryDirectory() as directory:
            config["runner"]["output_directory"] = directory
            runner = Runner(config)
            with patch.object(runner, "ssh", side_effect=[(255, "refused"), (0, "")]) as ssh, \
                 patch.object(time, "sleep"):
                runner.wait_guest()
                self.assertEqual(ssh.call_count, 2)
                for call in ssh.call_args_list:
                    self.assertEqual(call.args[1], ["true"])

    def test_upgrade_guest_ssh_wait_is_bounded(self):
        config = self.upgrade_config()
        with tempfile.TemporaryDirectory() as directory:
            config["runner"]["output_directory"] = directory
            runner = Runner(config)
            with patch.object(runner, "ssh", return_value=(255, "refused")), \
                 patch.object(time, "monotonic", side_effect=[0, 0, 1000]), \
                 patch.object(time, "sleep"), self.assertRaises(TimeoutError):
                runner.wait_guest()

    def upgrade_config(self):
        config = load_config(ROOT / "dev/release-test/config.toml")
        config["candidate"].update(version="v0.3.80", artifact_directory="/candidate")
        config["baseline"] = {"version": "v0.3.79", "artifact_directory": ""}
        return config

    def test_upgrade_requires_distinct_versions_and_local_candidate(self):
        validate_upgrade_config(self.upgrade_config())
        for field, value in (("version", "v0.3.79"), ("artifact_directory", "")):
            config = self.upgrade_config()
            config["candidate"][field] = value
            with self.subTest(field=field), self.assertRaises(ValueError):
                validate_upgrade_config(config)

    def test_upgrade_refuses_identical_baseline_and_candidate_bytes(self):
        baseline = {f"{binary}_binary_sha256": "a" * 64 for binary in ("sherpa", "sherpad")}
        candidate = {f"{binary}_binary_sha256": "b" * 64 for binary in ("sherpa", "sherpad")}
        validate_upgrade_artifacts(baseline, candidate)
        for binary in ("sherpa", "sherpad"):
            changed = dict(candidate)
            changed[f"{binary}_binary_sha256"] = baseline[f"{binary}_binary_sha256"]
            with self.subTest(binary=binary), self.assertRaises(ValueError):
                validate_upgrade_artifacts(baseline, changed)

    def test_upgrade_runs_both_versions_and_rechecks_state_after_reboot(self):
        config = self.upgrade_config()
        with tempfile.TemporaryDirectory() as directory:
            config["runner"]["output_directory"] = directory
            runner = Runner(config)
            def step(name, label=None):
                runner.report["run"]["completed_steps"].append(name)
            with patch.object(runner, "inputs"), patch.object(runner, "provision"), \
                 patch.object(runner, "transfer"), patch.object(runner, "collect"), \
                 patch.object(runner, "step", side_effect=step), \
                 patch.object(runner, "reboot", side_effect=lambda: step("reboot")), \
                 redirect_stdout(io.StringIO()):
                self.assertEqual(runner.run("upgrade"), 0)
            self.assertEqual(runner.report["run"]["completed_steps"],
                             ["baseline", "install-baseline", "initialize", "authenticate",
                              "seed-upgrade", "upgrade", "authenticate", "verify-upgrade",
                              "reboot", "authenticate", "verify-upgrade-reboot"])

    def test_upgrade_failure_keeps_failed_receipt_and_verifies_targeted_cleanup(self):
        config = self.upgrade_config()
        config["runner"]["retain_failed"] = False
        with tempfile.TemporaryDirectory() as directory:
            config["runner"]["output_directory"] = directory
            runner = Runner(config)
            runner.lab_id = "12345678"
            def step(name, label=None):
                if name == "upgrade":
                    raise TimeoutError("upgrade timed out")
                runner.report["run"]["completed_steps"].append(name)
            with patch.object(runner, "inputs"), patch.object(runner, "provision"), \
                 patch.object(runner, "transfer"), patch.object(runner, "collect") as collect, \
                 patch.object(runner, "step", side_effect=step), \
                 patch.object(runner, "cleanup") as cleanup, redirect_stdout(io.StringIO()):
                self.assertEqual(runner.run("upgrade"), 1)
                cleanup.assert_called_once()
                collect.assert_called_once()
            report = tomllib.loads((runner.directory / "result.toml").read_text())
            self.assertEqual(report["run"]["failure_kind"], "timeout")
            self.assertNotIn("verify-upgrade", report["run"]["completed_steps"])

    def test_completed_upgrade_fails_when_evidence_collection_fails(self):
        config = self.upgrade_config()
        with tempfile.TemporaryDirectory() as directory:
            config["runner"]["output_directory"] = directory
            runner = Runner(config)
            runner.guest_uuid = "expected"
            runner.remote_directory = "/unused-test-workspace"
            runner.report["run"].update(scenario="upgrade", completed_steps=list(UPGRADE_STEPS))
            with patch.object(runner, "step"), \
                 patch.object(runner, "command", side_effect=RuntimeError("missing upgrade evidence")), \
                 redirect_stdout(io.StringIO()), self.assertRaises(RuntimeError):
                runner.collect()

    def test_configured_jump_host_is_used_for_guest_commands(self):
        config = load_config(ROOT / "dev/release-test/config.toml")
        config["host"]["ssh_destination"] = "remote-user@example.test"
        with tempfile.TemporaryDirectory() as directory:
            config["runner"]["output_directory"] = directory
            runner = Runner(config)
            runner.lab_id = "12345678"
            with patch.object(runner, "command", return_value=(0, "")) as command:
                runner.ssh("probe", ["true"], 1)
            self.assertIn("ProxyJump=remote-user@example.test", command.call_args.args[1])

    def test_result_reference_tracks_status_without_credentials(self):
        config = load_config(ROOT / "dev/release-test/config.toml")
        with tempfile.TemporaryDirectory() as directory:
            config["runner"]["output_directory"] = directory
            reference = Path(directory) / "reference.toml"
            runner = Runner(config, result_file=reference)
            runner.secrets = ["private-password"]
            runner.report["run"]["status"] = "failed"
            runner.save()
            data = tomllib.loads(reference.read_text())["result"]
            self.assertEqual(data, {"receipt": str(runner.directory / "result.toml"),
                                    "run_id": runner.run_id, "status": "failed"})
            self.assertNotIn("private-password", reference.read_text())

    def test_cleanup_refuses_a_replaced_domain_before_destroy(self):
        config = load_config(ROOT / "dev/release-test/config.toml")
        with tempfile.TemporaryDirectory() as directory:
            config["runner"]["output_directory"] = directory
            runner = Runner(config)
            runner.lab_id, runner.guest_uuid = "12345678", str(uuid.uuid4())
            (runner.directory / "lab-info.toml").write_text(
                f'id = "12345678"\nname = "{runner.lab_name}"\n')
            (runner.directory / "manifest.toml").write_text(
                f'name = "{runner.lab_name}"\n[[nodes]]\nname = "{runner.node_name}"\n')
            with patch.object(runner, "host", return_value=(0, str(uuid.uuid4()))), \
                 patch.object(runner, "sherpa") as destroy:
                with self.assertRaises(ValueError):
                    runner.cleanup()
                destroy.assert_not_called()

    def test_resuming_cleanup_rejects_tampered_resource_names(self):
        config = load_config(ROOT / "dev/release-test/config.toml")
        with tempfile.TemporaryDirectory() as directory:
            config["runner"]["output_directory"] = directory
            runner = Runner(config)
            (runner.directory / "runner-config.toml").write_text(encode_toml(config))
            runner.report["run"]["node_name"] = "another-lab"
            runner.save()
            with self.assertRaises(ValueError):
                Runner.resume(runner.directory)

    def test_logs_are_collected_before_each_uninstall(self):
        config = load_config(ROOT / "dev/release-test/config.toml")
        with tempfile.TemporaryDirectory() as directory:
            config["runner"]["output_directory"] = directory
            runner = Runner(config)
            events = []
            def step(name, label=None):
                events.append(label or name)
                if name not in ("diagnostics", "diagnostics-live"):
                    runner.report["run"]["completed_steps"].append(name)
            with patch.object(runner, "inputs"), patch.object(runner, "provision"), \
                 patch.object(runner, "transfer"), patch.object(runner, "collect"), \
                 patch.object(runner, "step", side_effect=step), \
                 patch.object(runner, "reboot", side_effect=lambda: step("reboot")), \
                 redirect_stdout(io.StringIO()):
                self.assertEqual(runner.run("lifecycle"), 0)
            for mode in ("keep-data", "remove-data", "remove-all"):
                index = events.index(mode)
                self.assertEqual(events[index - 1], "before-" + mode)

    def test_sigterm_records_failure_and_stops_the_active_command(self):
        with tempfile.TemporaryDirectory() as directory:
            code = f"""
import sys, time
from pathlib import Path
sys.path.insert(0, {str(ROOT / 'scripts')!r})
from vm_release_test import Runner, load_config, ROOT
config = load_config(ROOT / 'dev/release-test/config.toml')
config['runner']['output_directory'] = {directory!r}
runner = Runner(config)
runner.inputs = lambda: None
runner.provision = lambda: None
runner.transfer = lambda: None
runner.collect = lambda: None
def step(name):
    runner.command('active', [sys.executable, '-c', "from pathlib import Path; import os, time; Path({str(Path(directory) / 'active')!r}).write_text(str(os.getpid())); print('started', flush=True); time.sleep(60)"], 60)
runner.step = step
sys.exit(runner.run('preflight'))
"""
            process = subprocess.Popen([sys.executable, "-c", code], stdout=subprocess.PIPE,
                                       stderr=subprocess.STDOUT, text=True)
            try:
                ready = Path(directory) / "active"
                deadline = time.monotonic() + 10
                while not ready.exists() and process.poll() is None and time.monotonic() < deadline:
                    time.sleep(0.05)
                self.assertTrue(ready.exists())
                child = int(ready.read_text())
                process.send_signal(signal.SIGTERM)
                output, _ = process.communicate(timeout=15)
                self.assertNotEqual(process.returncode, 0, output)
                receipt = tomllib.loads(next(Path(directory).glob("*/result.toml")).read_text())
                self.assertEqual(receipt["run"]["status"], "failed")
                self.assertEqual(receipt["run"]["failure_kind"], "interrupted")
                with self.assertRaises(ProcessLookupError):
                    os.kill(child, 0)
            finally:
                if process.poll() is None:
                    process.kill()
                    process.communicate()
                if (Path(directory) / "active").exists():
                    with suppress(ProcessLookupError):
                        os.killpg(int((Path(directory) / "active").read_text()), signal.SIGKILL)

    def test_partial_provisioning_is_not_reported_as_clean(self):
        config = load_config(ROOT / "dev/release-test/config.toml")
        with tempfile.TemporaryDirectory() as directory:
            config["runner"]["output_directory"] = directory
            runner = Runner(config)
            def fail_up(label, arguments, timeout, check=True):
                if label == "up":
                    raise RuntimeError("interrupted provisioning")
                if label == "inspect-before":
                    return 1, f"Sherpa Environment - {runner.lab_name}-12345678"
                return 0, ""
            with patch.object(runner, "sherpa", side_effect=fail_up), \
                 patch.object(runner, "host", return_value=(1, "")), redirect_stdout(io.StringIO()):
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
        self.assertEqual(jump_host_arguments("user@[192.0.2.10]"), ["-l", "user", "192.0.2.10"])
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
