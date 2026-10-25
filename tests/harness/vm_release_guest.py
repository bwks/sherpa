#!/usr/bin/env python3
"""Guest-only checks. Never invoke this helper on the Sherpa host."""

import fcntl
from contextlib import suppress
import hashlib
import json
import os
from pathlib import Path
import pty
import pwd
import re
import select
import shutil
import signal
import socket
import ssl
import subprocess
import sys
import tarfile
import tempfile
import time
import tomllib
import unittest
from unittest.mock import patch
import urllib.error
import urllib.request
import xml.etree.ElementTree as ET


def require(condition, message):
    if not condition:
        raise RuntimeError(message)


def validate_identity(expected_uuid, expected_hostname, actual_uuid, actual_hostname):
    require(actual_uuid.lower() == expected_uuid.lower(), "Target VM UUID does not match")
    require(actual_hostname.split(".")[0] == expected_hostname, "Target VM hostname does not match")


def digest(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def environment_fingerprint(path):
    values = {}
    for line in path.read_text().splitlines():
        line = line.strip()
        if not line or line.startswith("#"):
            continue
        require("=" in line, "Malformed installer environment assignment")
        key, value = line.split("=", 1)
        values[key.strip()] = value.strip()
    require(bool(values), "Installer environment settings are missing")
    return hashlib.sha256(json.dumps(values, sort_keys=True).encode()).hexdigest()


def validate_binary_version(binary, version, output):
    require(output.split() == [binary, version.removeprefix("v")],
            f"Unexpected {binary} version")


TEST_SUMMARY = re.compile(r"^test result: (?:ok|FAILED)\. (\d+) passed; (\d+) failed; (\d+) ignored", re.M)
FAILED_TEST = re.compile(r"^test (\S+) \.\.\. FAILED$", re.M)
# Tests that return early when their host requirement is missing; in a test environment
# the requirement is guaranteed, so the skip means the test did not run.
REQUIRED_RUN_MARKERS = ("Skipping VM test",)


def summarize_tests(output):
    totals = {"binaries": 0, "passed": 0, "failed": 0, "ignored": 0}
    for passed, failed, ignored in TEST_SUMMARY.findall(output):
        totals["binaries"] += 1
        totals["passed"] += int(passed)
        totals["failed"] += int(failed)
        totals["ignored"] += int(ignored)
    totals["failures"] = sorted(set(FAILED_TEST.findall(output)))
    totals["skips"] = [line.strip() for line in output.splitlines() if line.lstrip().startswith("Skipping")]
    return totals


def test_problems(code, summary):
    problems = []
    if code:
        problems.append(f"cargo exited {code}")
    if not summary["binaries"]:
        problems.append("no test results were reported")
    if summary["failed"] or summary["failures"]:
        problems.append(f"{max(summary['failed'], len(summary['failures']))} failed")
    problems.extend(f"required test skipped: {line}" for line in summary["skips"]
                    if line.startswith(REQUIRED_RUN_MARKERS))
    return problems


def leftover_resources(before, after):
    leftovers = {}
    for kind, names in after.items():
        added = sorted(set(names) - set(before.get(kind, [])))
        if added:
            leftovers[kind] = added
    return leftovers


def toml_document(sections):
    lines = []
    for section, values in sections.items():
        lines.append(f"[{section}]")
        lines.extend(f"{key} = {json.dumps(value, ensure_ascii=False)}" for key, value in values.items())
        lines.append("")
    return "\n".join(lines)


def validate_retained_resources(before, after):
    require(set(before) == set(after), "Resource inventory categories changed")
    for category, items in before.items():
        require(items == after[category], f"Uninstall changed retained {category}")


def validate_upgrade_state(before, after):
    required = {"config", "ssh_key", "environment", "certificate", "users"}
    require(set(before) == required and set(after) == required, "Incomplete upgrade state evidence")
    for key in required - {"users"}:
        require(isinstance(before[key], str) and re.fullmatch(r"[0-9a-f]{64}", before[key]),
                f"Missing {key} fingerprint")
    require(isinstance(before["users"], list) and len(before["users"]) == 2,
            "Both persistent test users must be recorded")
    for key in required:
        require(before[key] == after[key], f"Upgrade changed retained {key}")


def validate_upgrade_evidence(evidence, baseline, candidate):
    phases = {"before", "after", "reboot", "baseline-binaries", "candidate-binaries", "reboot-binaries"}
    require(set(evidence) == phases, "Incomplete upgrade phase evidence")
    for phase in ("after", "reboot"):
        validate_upgrade_state(evidence["before"], evidence[phase])
    for phase, inputs in (("baseline-binaries", baseline), ("candidate-binaries", candidate),
                          ("reboot-binaries", candidate)):
        for binary in ("sherpa", "sherpad"):
            key = binary + "_binary_sha256"
            require(evidence[phase].get(key) == inputs[key], f"Incorrect {phase} {binary} bytes")


def ubuntu_version(version):
    """Map a Sherpa image version such as 24.04-20260926 to its Ubuntu VERSION_ID."""
    return version.split("-", 1)[0]


def check_bare_ubuntu(command, vm, initialize_timeout):
    """Check a fresh Sherpa-built Ubuntu guest against its manifest node settings."""
    command(["cloud-init", "status", "--wait", "--long"], timeout=initialize_timeout)
    release = dict(line.split("=", 1) for line in Path("/etc/os-release").read_text().splitlines()
                   if "=" in line)
    require(release["ID"].strip('"') == "ubuntu"
            and release["VERSION_ID"].strip('"') == ubuntu_version(vm["version"]),
            "Guest operating system differs from the manifest")
    print("OS: " + release["PRETTY_NAME"])
    require(os.cpu_count() == vm["cpu_count"], "Guest vCPU count differs from the manifest")
    _, disk = command(["lsblk", "-bdno", "SIZE", "/dev/vda"])
    require(int(disk.strip()) == vm["boot_disk_size"] * 1024**3,
            "Guest disk size differs from the manifest")
    for binary in ("sherpa", "sherpad", "docker", "dockerd", "virsh", "surreal"):
        code, _ = command(["bash", "-c", 'command -v "$1"', "bash", binary], check=False)
        require(code != 0, f"Preinstalled runtime command: {binary}")
    require(not Path("/opt/sherpa").exists(), "/opt/sherpa already exists")
    with open("/dev/kvm", "rb", buffering=0) as device:
        require(fcntl.ioctl(device.fileno(), 0xAE00, 0) == 12, "KVM API unavailable")
        descriptor = fcntl.ioctl(device.fileno(), 0xAE01, 0)
        os.close(descriptor)
    require(pwd.getpwnam("sherpa").pw_uid >= 1000, "Expected Sherpa's cloud-init login account")


def interrupted(signum, _frame):
    raise KeyboardInterrupt(f"Guest interrupted by {signal.Signals(signum).name}")


class Guest:
    def __init__(self, config_path):
        self.config = tomllib.loads(config_path.read_text())
        self.workspace = config_path.parent
        self.settings = self.config["guest"]
        self.timeouts = self.config["timeouts"]
        self.credentials = self.config["credentials"]
        self.marker = self.workspace / ".owner"
        self.base = Path("/opt/sherpa")
        self.passwords = [self.credentials["db_password"], self.credentials["admin_password"]]

    def guard(self, baseline=False):
        identity = self.config["identity"]
        require(os.geteuid() == 0, "Guest checks require sudo")
        require(str(self.workspace) == identity["workspace"], "Unexpected guest workspace")
        require(self.workspace.name.endswith(identity["nonce"]), "Workspace nonce does not match")
        validate_identity(identity["uuid"], identity["hostname"],
                          Path("/sys/class/dmi/id/product_uuid").read_text().strip(),
                          socket.gethostname())
        if not baseline:
            require(self.marker.is_file() and self.marker.stat().st_uid == 0,
                    "Missing root-owned test marker")
            require(self.marker.read_text().strip() == identity["nonce"], "Test marker does not match")

    def redact(self, output):
        for secret in self.passwords:
            output = output.replace(secret, "[REDACTED]")
        return output

    def command(self, command, timeout=None, env=None, check=True, display=True, cwd=None):
        process = subprocess.Popen(command, cwd=cwd or self.workspace, stdin=subprocess.DEVNULL,
                                   stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                                   text=True, env=env, start_new_session=True)
        try:
            output, _ = process.communicate(timeout=timeout or self.timeouts["command"])
        except (subprocess.TimeoutExpired, KeyboardInterrupt) as error:
            with suppress(ProcessLookupError):
                os.killpg(process.pid, signal.SIGTERM)
            try:
                output, _ = process.communicate(timeout=self.timeouts["terminate"])
            except subprocess.TimeoutExpired:
                with suppress(ProcessLookupError):
                    os.killpg(process.pid, signal.SIGKILL)
                output, _ = process.communicate()
            print(self.redact(output), flush=True)
            if isinstance(error, KeyboardInterrupt):
                raise
            raise TimeoutError(f"Command timed out: {command[0]}")
        if display:
            print(self.redact(output), end="", flush=True)
        if check:
            require(process.returncode == 0, f"Command failed ({process.returncode}): {command[0]}")
        return process.returncode, output

    def environment(self, selection="candidate"):
        env = dict(os.environ)
        env.update(SHERPA_DB_PASSWORD=self.credentials["db_password"],
                   SHERPA_SERVER_IPV4=self.settings["listen_ip"],
                   SHERPA_SERVER_WS_PORT=str(self.settings["ws_port"]),
                   SHERPA_SERVER_HTTP_PORT=str(self.settings["http_port"]),
                   SHERPA_DB_PORT=str(self.settings["db_port"]),
                   DEBIAN_FRONTEND="noninteractive")
        env.pop("SHERPA_ARTIFACT_DIR", None)
        if selection == "baseline":
            env["SHERPA_ARTIFACT_DIR"] = str(self.workspace / "baseline")
        elif self.config[selection]["artifact_directory"]:
            env["SHERPA_ARTIFACT_DIR"] = str(self.workspace)
        return env

    def installer(self, selection="candidate"):
        return ["bash", str(self.workspace / "sherpa_install.sh"),
                "--version", self.config[selection]["version"]]

    def baseline(self):
        check_bare_ubuntu(self.command, self.config["vm"], self.timeouts["initialize"])
        self.command(["curl", "--version"])
        self.marker.write_text(self.config["identity"]["nonce"])
        self.marker.chmod(0o600)
        print("PASS bare baseline, cloud-init, nested KVM and guest identity")

    def preflight(self):
        for script in ("sherpa_install.sh", "sherpa_uninstall.sh"):
            code, output = self.command(["bash", str(self.workspace / script), "--help"])
            require("Usage:" in output, "Missing help output")
            code, output = self.command(["bash", str(self.workspace / script), "--bogus"], check=False)
            require(code != 0 and "Unknown option" in output, "Invalid argument accepted")
        code, output = self.command(["sudo", "-u", "sherpa", *self.installer()],
                                    env=self.environment(), check=False)
        require(code != 0 and "must be run as root" in output, "Non-root installer accepted")
        env = self.environment()
        env["SHERPA_DB_PASSWORD"] = "short"
        code, output = self.command(self.installer(), env=env, check=False)
        require(code != 0 and "at least 8" in output, "Short password accepted")
        env = self.environment()
        env["SHERPA_SERVER_IPV4"] = "invalid"
        code, output = self.command(self.installer(), env=env, check=False)
        require(code != 0 and "Invalid IPv4" in output, "Invalid address accepted")
        with socket.socket() as listener:
            listener.bind((self.settings["listen_ip"], self.settings["db_port"]))
            listener.listen()
            code, output = self.command(self.installer(), env=self.environment(), check=False)
            require(code != 0 and "already in use" in output, "Occupied DB port accepted")
        env = self.environment()
        env["SHERPA_ARTIFACT_DIR"] = str(self.workspace / "missing")
        code, output = self.command(self.installer(), env=env, check=False)
        require(code != 0 and "artifact" in output.lower(), "Unavailable artifacts accepted")
        require(not self.base.exists(), "Preflight failures changed the installation")

    def check_runtimes(self):
        """Docker and libvirt answer requests. libvirtd is socket-activated and exits when
        idle, so an inactive libvirtd.service is normal; the socket must accept a connection."""
        self.command(["systemctl", "is-active", "docker"])
        self.command(["systemctl", "is-active", "libvirtd.socket"])
        self.command(["virsh", "-c", "qemu:///system", "version"])

    def install(self, selection="candidate"):
        self.command(self.installer(selection), timeout=self.timeouts["install"], env=self.environment(selection))
        self.verify_installation(selection)

    def verify_installation(self, selection="candidate"):
        for binary in ("sherpa", "sherpad"):
            path = self.base / "bin" / binary
            require(digest(path) == self.config[selection][f"{binary}_binary_sha256"],
                    f"Installed {binary} does not match the {selection} artifact")
            require(path.stat().st_mode & 0o777 == 0o755, f"Invalid {binary} permissions")
            require(path.stat().st_uid == pwd.getpwnam("sherpa").pw_uid, f"Invalid {binary} owner")
            link = Path("/usr/local/bin") / binary
            require(link.is_symlink() and link.resolve() == path, f"Invalid {binary} symlink")
            _, output = self.command([str(path), "--version"])
            validate_binary_version(binary, self.config[selection]["version"], output)
        for directory, permissions in (("", 0o775), ("db", 0o775), ("config", 0o775), ("env", 0o750)):
            path = self.base / directory
            require(path.is_dir() and path.stat().st_mode & 0o777 == permissions,
                    f"Invalid permissions for {path}")
            require(path.stat().st_uid == pwd.getpwnam("sherpa").pw_uid, f"Invalid owner: {path}")
        env_file = self.base / "env/sherpa.env"
        require(env_file.stat().st_mode & 0o777 == 0o640, "Environment permissions are not 640")
        require(env_file.stat().st_uid == pwd.getpwnam("sherpa").pw_uid, "Invalid environment owner")
        env_values = dict(line.split("=", 1) for line in env_file.read_text().splitlines()
                          if line and not line.startswith("#") and "=" in line)
        for key, value in self.environment().items():
            if key.startswith("SHERPA_") and key != "SHERPA_ARTIFACT_DIR":
                require(env_values.get(key) == value, f"Environment setting mismatch: {key}")
        _, groups = self.command(["id", "-nG", "sherpa"])
        require({"libvirt", "docker", "kvm"}.issubset(groups.split()), "Missing runtime groups")
        self.check_runtimes()
        self.command(["systemctl", "is-enabled", "sherpad"])
        unit = Path("/etc/systemd/system/sherpad.service").read_text()
        require("ExecStart=/opt/sherpa/bin/sherpad start --foreground" in unit, "Incorrect service command")
        require("Requires=docker.service libvirtd.service" in unit, "Missing service dependencies")
        require("RuntimeDirectory=sherpa\nRuntimeDirectoryMode=0700\nRuntimeDirectoryPreserve=yes" in unit,
                "Missing private /run/sherpa for container secrets")
        require(Path("/etc/logrotate.d/sherpad").is_file(), "Missing logrotate configuration")
        _, image = self.command(["docker", "inspect", "--format",
                                "{{.Config.Image}} {{.State.Running}} {{.HostConfig.RestartPolicy.Name}}",
                                "sherpa-db"])
        require(image.strip() == self.settings["surrealdb_image"] + " true unless-stopped",
                "Incorrect database image, state or restart policy")
        self.command(["docker", "image", "inspect", "--format", "{{.Id}}", self.settings["router_image"]])
        self.command(["curl", "-fsS", f"http://{self.settings['client_host']}:{self.settings['db_port']}/health"])
        require(any((self.base / "db").iterdir()), "Database storage is empty")

    def initialize(self):
        command = ["sudo", "-u", "sherpa", "-H", str(self.base / "bin/sherpad"), "init"]
        # The built-in admin account, then the first user account, a personal administrator.
        prompts = [(b"Password for admin: ", self.credentials["admin_password"]),
                   (b"Confirm password: ", self.credentials["admin_password"]),
                   (b"Username: ", self.credentials["admin_username"]),
                   (f"Password for {self.credentials['admin_username']}: ".encode(),
                    self.credentials["admin_password"]),
                   (b"Confirm password: ", self.credentials["admin_password"]),
                   (b"Make this user an administrator? [Y/n]: ", "y"),
                   (b"(file path or key, Enter to skip): ", "")]
        pid, descriptor = pty.fork()
        if pid == 0:
            os.chdir(self.base)
            os.execvp(command[0], command)
        deadline = time.monotonic() + self.timeouts["initialize"]
        output = bytearray()
        pending = bytearray()
        status = None
        waited = 0
        try:
            while time.monotonic() < deadline:
                ready, _, _ = select.select([descriptor], [], [], self.timeouts["poll"])
                if ready:
                    try:
                        chunk = os.read(descriptor, 65536)
                    except OSError:
                        break
                    if not chunk:
                        break
                    output.extend(chunk)
                    pending.extend(chunk)
                    if prompts and prompts[0][0] in pending:
                        _, answer = prompts.pop(0)
                        os.write(descriptor, (answer + "\n").encode())
                        pending.clear()
                waited, status = os.waitpid(pid, os.WNOHANG)
                if waited:
                    break
            if waited == 0:
                waited, status = os.waitpid(pid, os.WNOHANG)
                # EOF on the PTY can precede sudo's actual process exit.
                while not waited and time.monotonic() < deadline:
                    time.sleep(self.timeouts["poll"])
                    waited, status = os.waitpid(pid, os.WNOHANG)
                if not waited:
                    os.killpg(pid, signal.SIGKILL)
                    waited, status = os.waitpid(pid, 0)
                    raise TimeoutError("Server initialization timed out")
            require(os.waitstatus_to_exitcode(status) == 0 and not prompts, "Server initialization failed")
        except (Exception, KeyboardInterrupt):
            if not waited:
                with suppress(ProcessLookupError):
                    os.killpg(pid, signal.SIGKILL)
                os.waitpid(pid, 0)
            raise
        finally:
            os.close(descriptor)
            print(self.redact(output.decode(errors="replace")), flush=True)
        self.snapshot()
        self.command(["systemctl", "start", "sherpad"])

    def snapshot(self):
        hashes = {"config": digest(self.base / "config/sherpa.toml"),
                  "ssh_key": digest(self.base / "ssh/sherpa_ssh_key")}
        (self.workspace / "retained.toml").write_text("\n".join(f'{key} = "{value}"' for key, value in hashes.items()))

    def verify_retained(self):
        hashes = tomllib.loads((self.workspace / "retained.toml").read_text())
        require(digest(self.base / "config/sherpa.toml") == hashes["config"], "Server config changed")
        require(digest(self.base / "ssh/sherpa_ssh_key") == hashes["ssh_key"], "SSH identity changed")

    def request(self, path, payload=None, token=None):
        certificate = self.base / ".certs/server.crt"
        context = ssl.create_default_context(cafile=str(certificate))
        headers = {"Content-Type": "application/json"}
        if token:
            headers["Authorization"] = f"Bearer {token}"
        url = f"https://{self.settings['client_host']}:{self.settings['ws_port']}{path}"
        data = json.dumps(payload).encode() if payload is not None else None
        with urllib.request.urlopen(urllib.request.Request(url, data=data, headers=headers),
                                    context=context, timeout=self.timeouts["connect"]) as response:
            return json.load(response)

    def authenticate(self):
        deadline = time.monotonic() + self.timeouts["startup"]
        last_error = ""
        while time.monotonic() < deadline:
            try:
                self.command(["systemctl", "is-active", "sherpad"])
                login = self.request("/api/v1/auth/login",
                                     {"username": self.credentials["admin_username"],
                                      "password": self.credentials["admin_password"]})
                require(login["is_admin"] and login["username"] == self.credentials["admin_username"],
                        "Incorrect authenticated admin")
                token = login["token"]
                self.passwords.append(token)
                users = self.request("/api/v1/admin/users", token=token)["users"]
                require(any(user["username"] == self.credentials["admin_username"] for user in users),
                        "Persistent test user missing")
                require(any(user["username"] == "admin" and user["is_admin"] for user in users),
                        "Built-in admin account missing")
                client_base = Path.home() / ".sherpa"
                (client_base / "config").mkdir(parents=True, exist_ok=True, mode=0o700)
                (client_base / "token").write_text(token)
                (client_base / "token").chmod(0o600)
                url = f"wss://{self.settings['client_host']}:{self.settings['ws_port']}/ws"
                (client_base / "config/sherpa.toml").write_text(
                    "[server_connection]\n"
                    f"url = {json.dumps(url)}\n"
                    f"timeout_secs = {self.timeouts['connect']}\n"
                    "validate_certs = true\ninsecure = false\n"
                    f"ca_cert_path = {json.dumps(str(self.base / '.certs/server.crt'))}\n")
                _, output = self.command([str(self.base / "bin/sherpa"), "whoami", "--server-url", url])
                require("Authenticated" in output and self.credentials["admin_username"] in output,
                        "CLI did not validate the test admin")
                self.verify_retained()
                print("PASS authenticated API, CLI and persistent admin record")
                return
            except (RuntimeError, OSError, urllib.error.URLError) as error:
                last_error = str(error)
                time.sleep(self.timeouts["poll"])
        raise TimeoutError(f"Server authentication did not become ready: {last_error}")

    def reboot(self):
        self.command(["systemd-run", "--unit=sherpa-release-reboot",
                      f"--on-active={self.timeouts['poll']}", "systemctl", "reboot"])

    def login(self, username):
        login = self.request("/api/v1/auth/login", {"username": username,
                             "password": self.credentials["admin_password"]})
        require(login["username"] == username, "Incorrect persistent user login")
        self.passwords.append(login["token"])
        return login

    def upgrade_state(self):
        self.verify_retained()
        state = {"config": digest(self.base / "config/sherpa.toml"),
                 "ssh_key": digest(self.base / "ssh/sherpa_ssh_key"),
                 "environment": environment_fingerprint(self.base / "env/sherpa.env"),
                 "certificate": digest(self.base / ".certs/server.crt")}
        admin = self.login(self.credentials["admin_username"])
        require(admin["is_admin"], "Persistent admin lost privileges")
        username = "persist-" + self.config["identity"]["nonce"][:8]
        user = self.login(username)
        require(not user["is_admin"], "Persistent non-admin gained privileges")
        users = self.request("/api/v1/admin/users", token=admin["token"])["users"]
        selected = sorted((entry for entry in users
                           if entry["username"] in (self.credentials["admin_username"], username)),
                          key=lambda entry: entry["username"])
        require(len(selected) == 2, "Persistent test users missing")
        state["users"] = [json.dumps(entry, sort_keys=True) for entry in selected]
        return state

    def seed_upgrade(self):
        admin = self.login(self.credentials["admin_username"])
        username = "persist-" + self.config["identity"]["nonce"][:8]
        created = self.request("/api/v1/admin/users",
                               {"username": username, "password": self.credentials["admin_password"],
                                "is_admin": False, "token": admin["token"]}, token=admin["token"])
        require(created["success"] and created["username"] == username and not created["is_admin"],
                "Persistent test user was not created")
        self.write_resources(self.workspace / "upgrade-state.toml",
                             {"before": self.upgrade_state(), "baseline-binaries": self.installed_hashes()})
        print("PASS baseline persistent users, settings and identity recorded")

    def upgrade(self):
        evidence = tomllib.loads((self.workspace / "upgrade-state.toml").read_text())
        validate_upgrade_state(evidence["before"], self.upgrade_state())
        self.reinstall()

    def verify_upgrade(self, phase):
        self.verify_installation()
        evidence = tomllib.loads((self.workspace / "upgrade-state.toml").read_text())
        state = self.upgrade_state()
        validate_upgrade_state(evidence["before"], state)
        evidence[phase] = state
        evidence["candidate-binaries" if phase == "after" else "reboot-binaries"] = self.installed_hashes()
        if phase == "reboot":
            validate_upgrade_evidence(evidence, self.config["baseline"], self.config["candidate"])
        self.write_resources(self.workspace / "upgrade-state.toml", evidence)
        print(f"PASS {phase}: candidate binaries and unchanged users, settings and identity")

    def installed_hashes(self):
        return {binary + "_binary_sha256": digest(self.base / "bin" / binary)
                for binary in ("sherpa", "sherpad")}

    def reinstall(self):
        self.install()
        self.verify_retained()
        self.command(["systemctl", "start", "sherpad"])

    def uninstall(self, mode):
        before = self.resources()
        evidence = self.workspace / ("resources-" + mode.removeprefix("--") + ".toml")
        self.write_resources(evidence, {"before": before})
        self.command(["bash", str(self.workspace / "sherpa_uninstall.sh"), mode, "--force"])
        after = self.resources()
        self.write_resources(evidence, {"before": before, "after": after})
        validate_retained_resources(before, after)
        print("PASS retained packages, users/groups, images, volumes and libvirt resources")
        require(not Path("/etc/systemd/system/sherpad.service").exists(), "Service unit remains")
        require(not Path("/etc/logrotate.d/sherpad").exists(), "Logrotate entry remains")
        binaries = ("sherpa", "sherpad") if mode == "--remove-all" else ("sherpad",)
        for binary in binaries:
            require(not (self.base / "bin" / binary).exists(), f"{binary} binary remains")
            link = Path("/usr/local/bin") / binary
            require(not link.exists() and not link.is_symlink(), f"{binary} symlink remains")
        code, _ = self.command(["docker", "inspect", "--format", "{{.Id}}", "sherpa-db"], check=False)
        require(code != 0, "Database container remains")
        code, _ = self.command(["systemctl", "is-active", "--quiet", "sherpad"], check=False)
        require(code != 0, "Server still running")
        self.check_runtimes()
        self.command(["id", "sherpa"])
        if mode == "--remove-all":
            require(not self.base.exists(), "Installation directory remains")
        else:
            client = self.base / "bin/sherpa"
            require(digest(client) == self.config["candidate"]["sherpa_binary_sha256"],
                    "Retained CLI differs from the candidate")
            require((Path("/usr/local/bin") / "sherpa").resolve() == client, "Retained CLI link is broken")
            self.verify_retained()
            require((self.base / "db").is_dir(), "Database directory removed")
            files = list((self.base / "db").iterdir())
            require(bool(files) if mode == "--keep-data" else not files, "Incorrect database retention")

    def write_resources(self, path, sections):
        lines = []
        for section, values in sections.items():
            lines.append(f"[{section}]")
            lines.extend(f"{key} = {json.dumps(value)}" for key, value in values.items())
        path.write_text("\n".join(lines) + "\n")

    def resources(self):
        def query(command):
            return self.command(command, display=False)[1].strip()
        inventory = {
            "packages": sorted(query(["dpkg-query", "-W", "-f=${binary:Package}\t${db:Status-Status}\t${Version}\n"]).splitlines()),
            "users": sorted(query(["getent", "passwd"]).splitlines()),
            "groups": sorted(query(["getent", "group"]).splitlines()),
            "images": sorted(query(["docker", "image", "ls", "--no-trunc", "--format",
                                    "{{.Repository}}:{{.Tag}} {{.ID}}"] ).splitlines()),
            "volumes": sorted(query(["docker", "volume", "ls", "--format", "{{.Name}}"] ).splitlines()),
            "containers": sorted(line for line in query(["docker", "ps", "-a", "--no-trunc", "--format",
                                "{{.Names}} {{.ID}} {{.Image}} {{.State}}"] ).splitlines()
                                 if line.split()[0] != "sherpa-db"),
        }
        for kind, category in (("net", "networks"), ("pool", "pools"), ("dom", "domains")):
            list_command = "list" if kind == "dom" else kind + "-list"
            names = query(["virsh", "-c", "qemu:///system", list_command, "--all", "--name"]).split()
            entries = []
            for name in sorted(names):
                xml = query(["virsh", "-c", "qemu:///system", kind + "-dumpxml" if kind != "dom" else "dumpxml", name])
                root = ET.fromstring(xml)
                for field in ("capacity", "allocation", "available"):
                    element = root.find(field)
                    if element is not None:
                        root.remove(element)
                identity = hashlib.sha256(ET.tostring(root)).hexdigest()
                info = query(["virsh", "-c", "qemu:///system", kind + "info" if kind == "dom" else kind + "-info", name])
                state = [line.strip() for line in info.splitlines()
                         if line.split(":", 1)[0].strip() in ("State", "Active", "Persistent", "Autostart")]
                entries.append(name + " " + identity + " " + " ".join(state))
            inventory[category] = entries
        print("Resource inventory: " + ", ".join(f"{key}={len(value)}" for key, value in inventory.items()))
        return inventory

    def diagnostics(self, required=False):
        print("Collecting server/database logs, systemd status and boot diagnostics", flush=True)
        commands = (["systemctl", "status", "--no-pager", "sherpad", "docker", "libvirtd"],
                    ["journalctl", "--no-pager", "-u", "sherpad", "-n", "200"],
                    ["journalctl", "--no-pager", "-b", "-p", "warning", "-n", "100"],
                    ["docker", "logs", "--tail", "100", "sherpa-db"],
                    ["docker", "ps", "-a", "--format", "{{.Names}} {{.Status}}"],
                    ["cloud-init", "status", "--long"],
                    ["df", "-h"])
        for command in commands:
            try:
                code, output = self.command(command, check=False)
                if required and command[:2] == ["docker", "logs"]:
                    require(code == 0, "Database logs unavailable before uninstall")
                if required and command[0] == "journalctl" and "sherpad" in command:
                    require(code == 0 and output.strip() and output.strip() != "-- No entries --",
                            "Server journal unavailable before uninstall")
            except FileNotFoundError:
                print(f"Diagnostic tool not installed: {command[0]}")
        log = self.base / "logs/sherpad.log"
        if log.is_file():
            self.command(["tail", "-n", "200", str(log)])
        if required:
            print("PASS live server and database logs captured before uninstall")

    def login_user(self):
        name = os.environ.get("SUDO_USER", "")
        require(name and name != "root", "Run code steps through sudo from the guest login account")
        return pwd.getpwnam(name)

    def cargo_environment(self, root):
        user = self.login_user()
        name = "root" if root else user.pw_name
        environment = {"PATH": f"{user.pw_dir}/.cargo/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin",
                       "HOME": "/root" if root else user.pw_dir,
                       # Code under test reads the current user from these, as in a login session.
                       "USER": name, "LOGNAME": name,
                       "CARGO_HOME": f"{user.pw_dir}/.cargo", "RUSTUP_HOME": f"{user.pw_dir}/.rustup",
                       # Keep build output outside the source tree so a rerun can replace the source.
                       "CARGO_TARGET_DIR": str(self.workspace / "target"),
                       "CARGO_TERM_COLOR": "never", "LANG": "C.UTF-8"}
        if root:
            environment.update(SHERPA_DB_PASSWORD=self.credentials["test_db_password"],
                               SHERPA_DEV_DB_PORT=str(self.config["tests"]["test_db_port"]),
                               SHERPA_GUEST_TEST_KEY=str(self.workspace / "guest-test-key"),
                               SHERPA_GUEST_TEST_ROTATED_KEY=str(self.workspace / "guest-test-key-rotated"))
        return environment

    def as_user(self, command):
        user = self.login_user()
        environment = [f"{key}={value}" for key, value in self.cargo_environment(root=False).items()]
        return ["sudo", "-u", user.pw_name, "-H", "env", "-i", *environment, *command]

    def logged(self, name, command, timeout, env=None):
        logs = self.workspace / "logs"
        logs.mkdir(exist_ok=True)
        code, output = self.command(command, timeout=timeout, env=env, check=False, display=False,
                                    cwd=self.workspace / "source")
        (logs / f"{name}.log").write_text(self.redact(output))
        print(self.redact("\n".join(output.splitlines()[-40:])), flush=True)
        print(f"{name}: exit {code}, full log {logs / (name + '.log')}", flush=True)
        return code, output

    def record_results(self, section, values):
        path = self.workspace / "test-results.toml"
        results = tomllib.loads(path.read_text()) if path.exists() else {}
        results[section] = values
        path.write_text(toml_document(results))
        path.chmod(0o644)

    def source(self):
        archive = self.workspace / "source.tar.gz"
        require(digest(archive) == self.config["tests"]["source_sha256"],
                "Source archive differs from the recorded snapshot")
        destination = self.workspace / "source"
        if destination.exists():
            shutil.rmtree(destination)
        with tarfile.open(archive, "r:gz") as stream:
            stream.extractall(destination, filter="data")
        user = self.login_user()
        self.command(["chown", "-R", f"{user.pw_name}:", str(destination)])
        require((destination / "Cargo.lock").is_file(), "Source snapshot has no Cargo.lock")
        print(f"PASS source snapshot {self.config['tests']['source_sha256']}")

    def toolchain(self):
        environment = dict(os.environ, DEBIAN_FRONTEND="noninteractive")
        self.command(["apt-get", "update"], timeout=self.timeouts["install"], env=environment)
        self.command(["apt-get", "install", "-y", "--no-install-recommends", "build-essential",
                      "pkg-config", "libvirt-dev", "ca-certificates", "curl"],
                     timeout=self.timeouts["install"], env=environment)
        user = self.login_user()
        if not Path(user.pw_dir, ".cargo/bin/rustup").exists():
            installer = self.workspace / "rustup-init.sh"
            self.command(["curl", "-fsSL", "--proto", "=https", "--tlsv1.2", "-o", str(installer),
                          "https://sh.rustup.rs"], timeout=self.timeouts["download"])
            self.command(self.as_user(["sh", str(installer), "-y", "--profile", "minimal",
                                       "--default-toolchain", "none", "--no-modify-path"]),
                         timeout=self.timeouts["install"])
        toolchain = tomllib.loads((self.workspace / "source/rust-toolchain.toml").read_text())["toolchain"]
        self.command(self.as_user(["rustup", "toolchain", "install", toolchain["channel"], "--profile", "minimal"]),
                     timeout=self.timeouts["install"])
        _, version = self.command(self.as_user(["rustc", "--version"]), cwd=self.workspace / "source")
        self.record_results("toolchain", {"rustc": version.strip()})

    def test_images(self):
        tests = self.config["tests"]
        digests = {}
        for image in tests["container_images"]:
            self.command(["docker", "pull", "--quiet", image], timeout=self.timeouts["images"])
            _, output = self.command(["docker", "image", "inspect", "--format", "{{json .RepoDigests}}", image])
            digests[image] = json.loads(output)
        images = self.base / "images"
        directory = images / tests["vm_image_model"] / tests["vm_image_version"]
        path = directory / "virtioa.qcow2"
        if not path.is_file() or digest(path) != tests["vm_image_sha256"]:
            directory.mkdir(parents=True, exist_ok=True)
            partial = directory / "virtioa.qcow2.partial"
            self.command(["curl", "-fSL", "--connect-timeout", str(self.timeouts["connect"]), "-o", str(partial),
                          tests["vm_image_url"]], timeout=self.timeouts["images"])
            require(digest(partial) == tests["vm_image_sha256"], "Test VM image checksum mismatch")
            partial.replace(path)
        self.command(["chown", "-R", "--reference", str(images), str(images / tests["vm_image_model"])])
        self.record_results("images", {"container_digests": [f"{image} {' '.join(found)}"
                                                             for image, found in digests.items()],
                                       "vm_image": str(path), "vm_image_sha256": tests["vm_image_sha256"]})
        print("PASS test images")

    def build_tests(self):
        timeout = self.timeouts["build"]
        builds = [("build-workspace", ["cargo", "test", "--workspace", "--locked", "--no-run"])]
        builds.extend((f"build-{crate}", ["cargo", "test", "-p", crate, "--locked", "--no-run"])
                      for crate in self.config["tests"]["integration_crates"])
        for name, command in builds:
            code, _ = self.logged(name, self.as_user(command), timeout)
            require(code == 0, f"{name} failed")
        print("PASS test build")

    def unit_tests(self):
        command = ["cargo", "test", "--workspace", "--locked", "--offline", "--no-fail-fast"]
        code, output = self.logged("unit", self.as_user(command), self.timeouts["tests"])
        summary = summarize_tests(output)
        problems = test_problems(code, summary)
        self.record_results("unit", {**summary, "status": "failed" if problems else "passed",
                                     "problems": problems})
        require(not problems, "Unit tests: " + "; ".join(problems))
        print(f"PASS unit tests: {summary['passed']} passed, {summary['ignored']} ignored")

    def inventory(self):
        queries = {"containers": ["docker", "ps", "-a", "--format", "{{.Names}}"],
                   "docker_networks": ["docker", "network", "ls", "--format", "{{.Name}}"],
                   "docker_volumes": ["docker", "volume", "ls", "--format", "{{.Name}}"],
                   "domains": ["virsh", "-c", "qemu:///system", "list", "--all", "--name"],
                   "libvirt_networks": ["virsh", "-c", "qemu:///system", "net-list", "--all", "--name"],
                   "storage_pools": ["virsh", "-c", "qemu:///system", "pool-list", "--all", "--name"]}
        found = {}
        for kind, command in queries.items():
            _, output = self.command(command, display=False)
            found[kind] = sorted({line.strip() for line in output.splitlines() if line.strip()})
        _, links = self.command(["ip", "-j", "link", "show"], display=False)
        found["links"] = sorted(link["ifname"] for link in json.loads(links))
        return found

    def wait_test_database(self):
        deadline = time.monotonic() + self.timeouts["startup"]
        while time.monotonic() < deadline:
            with suppress(OSError), socket.create_connection(("127.0.0.1", self.config["tests"]["test_db_port"]),
                                                             timeout=self.timeouts["connect"]):
                return
            time.sleep(self.timeouts["poll"])
        raise TimeoutError("Test database did not start")

    def integration_tests(self):
        tests = self.config["tests"]
        # The tests start their own server; the installed service must not compete for resources.
        self.command(["systemctl", "stop", "sherpad"])
        for name in ("guest-test-key", "guest-test-key-rotated"):
            if not (self.workspace / name).exists():
                self.command(["ssh-keygen", "-q", "-t", "ed25519", "-N", "", "-C", name,
                              "-f", str(self.workspace / name)])
        environment = self.cargo_environment(root=True)
        environment.update(assignment.split("=", 1) for assignment in tests["environment"])
        skips = [argument for name in tests["skip_tests"] for argument in ("--skip", name)]
        self.record_results("configuration", {"skip_tests": tests["skip_tests"],
                                              "environment": tests["environment"]})
        self.command(["bash", str(self.workspace / "source/dev/testdb"), "restart"],
                     env={"PATH": environment["PATH"], "SHERPA_DEV_DB_USER": tests["test_db_user"],
                          "SHERPA_DEV_DB_PASS": self.credentials["test_db_password"],
                          "SHERPA_DEV_DB_PORT": str(tests["test_db_port"])},
                     cwd=self.workspace / "source")
        self.wait_test_database()
        before = self.inventory()
        failed = []
        for crate in tests["integration_crates"]:
            code, output = self.logged(f"integration-{crate}",
                                       ["cargo", "test", "-p", crate, "--locked", "--offline", "--no-fail-fast",
                                        "--", "--ignored", "--test-threads=1", "--show-output", *skips],
                                       self.timeouts["tests"], env=environment)
            summary = summarize_tests(output)
            problems = test_problems(code, summary)
            self.record_results(f"integration-{crate}", {**summary, "status": "failed" if problems else "passed",
                                                          "problems": problems})
            print(f"{'FAIL' if problems else 'PASS'} {crate}: {summary['passed']} passed, "
                  f"{summary['failed']} failed{': ' + '; '.join(problems) if problems else ''}", flush=True)
            if problems:
                failed.append(crate)
        leftovers = leftover_resources(before, self.inventory())
        self.record_results("leftovers", {kind: names for kind, names in leftovers.items()})
        for kind, names in leftovers.items():
            print(f"FAIL leftover {kind}: {', '.join(names)}", flush=True)
        require(not failed and not leftovers,
                f"Integration tests failed: {', '.join(failed) or 'none'}; "
                f"leftover resources: {', '.join(leftovers) or 'none'}")
        print("PASS integration tests with no leftover resources")

    def fault_wait(self):
        code = ("from pathlib import Path; import os, time; "
                f"Path({str(self.workspace / 'fault-child.pid')!r}).write_text(str(os.getpid())); "
                "print('Fault probe running', flush=True); "
                f"time.sleep({self.config['faults']['wait']})")
        self.command([sys.executable, "-c", code], timeout=self.config["faults"]["wait"])

    def cancel(self):
        active = self.workspace / "active.pid"
        if active.exists():
            require(active.stat().st_uid == 0, "Active PID marker must be root-owned")
            pid = int(active.read_text())
            process = Path(f"/proc/{pid}")
            if process.exists():
                args = (process / "cmdline").read_bytes().split(b"\0")
                require(str(self.workspace / "vm_release_guest.py").encode() in args
                        and str(self.workspace / "guest.toml").encode() in args,
                        "Refusing to signal an unrelated process")
                os.kill(pid, signal.SIGTERM)
                deadline = time.monotonic() + self.timeouts["terminate"]
                while process.exists() and time.monotonic() < deadline:
                    time.sleep(0.1)
                require(not process.exists(), "Guest helper did not stop after cancellation")
        child = self.workspace / "fault-child.pid"
        if child.exists():
            require(not Path(f"/proc/{int(child.read_text())}").exists(), "Fault command survived cancellation")
        print("PASS guest command stopped; no probe process remains")

    def run(self, step):
        self.guard(baseline=step in ("baseline", "diagnostics", "cancel"))
        actions = {"baseline": self.baseline, "preflight": self.preflight,
                   "install": self.install, "initialize": self.initialize,
                   "authenticate": self.authenticate, "reboot": self.reboot,
                   "reinstall": self.reinstall, "restore": self.reinstall,
                   "keep-data": lambda: self.uninstall("--keep-data"),
                   "remove-data": lambda: self.uninstall("--remove-data"),
                   "remove-all": lambda: self.uninstall("--remove-all"),
                   "repeat-uninstall": lambda: self.uninstall("--remove-all"),
                   "diagnostics": self.diagnostics,
                   "diagnostics-live": lambda: self.diagnostics(required=True),
                   "fault-wait": self.fault_wait, "cancel": self.cancel,
                   "install-baseline": lambda: self.install("baseline"),
                   "seed-upgrade": self.seed_upgrade, "upgrade": self.upgrade,
                   "verify-upgrade": lambda: self.verify_upgrade("after"),
                   "verify-upgrade-reboot": lambda: self.verify_upgrade("reboot"),
                   "new-database": lambda: (self.install(), self.initialize()),
                   "source": self.source, "toolchain": self.toolchain,
                   "test-images": self.test_images, "build-tests": self.build_tests,
                   "unit-tests": self.unit_tests, "integration-tests": self.integration_tests}
        require(step in actions, "Unknown guest step")
        if step == "cancel":
            self.cancel()
            return
        active = self.workspace / "active.pid"
        with active.open("x") as stream:
            stream.write(str(os.getpid()))
        active.chmod(0o600)
        previous = signal.signal(signal.SIGTERM, interrupted)
        try:
            actions[step]()
        finally:
            active.unlink(missing_ok=True)
            signal.signal(signal.SIGTERM, previous)


class GuestTests(unittest.TestCase):
    def test_environment_fingerprint_ignores_comments_but_detects_changed_settings(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "environment"
            path.write_text("# Generated yesterday\nSHERPA_DB_PASSWORD=test-only\nSHERPA_DB_PORT=8000\n")
            before = environment_fingerprint(path)
            path.write_text("# Generated today\n\nSHERPA_DB_PORT=8000\nSHERPA_DB_PASSWORD=test-only\n")
            self.assertEqual(before, environment_fingerprint(path))
            path.write_text("SHERPA_DB_PORT=8001\nSHERPA_DB_PASSWORD=test-only\n")
            self.assertNotEqual(before, environment_fingerprint(path))

    def test_upgrade_evidence_rejects_missing_phases_and_wrong_installed_bytes(self):
        state = {key: "a" * 64 for key in ("config", "ssh_key", "environment", "certificate")}
        state["users"] = ['{"username":"admin","is_admin":true}',
                          '{"username":"persisted","is_admin":false}']
        baseline = {f"{binary}_binary_sha256": "b" * 64 for binary in ("sherpa", "sherpad")}
        candidate = {f"{binary}_binary_sha256": "c" * 64 for binary in ("sherpa", "sherpad")}
        evidence = {"before": state, "after": state, "reboot": state,
                    "baseline-binaries": baseline, "candidate-binaries": candidate,
                    "reboot-binaries": candidate}
        validate_upgrade_evidence(evidence, baseline, candidate)
        for phase in evidence:
            incomplete = dict(evidence)
            incomplete.pop(phase)
            with self.subTest(phase=phase), self.assertRaises(RuntimeError):
                validate_upgrade_evidence(incomplete, baseline, candidate)
        changed = {**evidence, "candidate-binaries": baseline}
        with self.assertRaises(RuntimeError):
            validate_upgrade_evidence(changed, baseline, candidate)

    def test_upgrade_state_requires_unchanged_settings_identity_and_records(self):
        before = {key: "a" * 64 for key in ("config", "ssh_key", "environment", "certificate")}
        before["users"] = ['{"username":"admin","is_admin":true}',
                           '{"username":"persisted","is_admin":false}']
        validate_upgrade_state(before, before)
        for key in before:
            changed = dict(before, **{key: [] if key == "users" else "b" * 64})
            with self.subTest(key=key), self.assertRaises(RuntimeError):
                validate_upgrade_state(before, changed)
        with self.assertRaises(RuntimeError):
            validate_upgrade_state({}, {})

    def test_baseline_install_uses_separate_verified_inputs(self):
        with tempfile.TemporaryDirectory() as directory:
            workspace = Path(directory)
            payload = {"candidate": {"version": "v0.3.80", "artifact_directory": "/candidate"},
                       "baseline": {"version": "v0.3.79", "artifact_directory": "/baseline"},
                       "guest": {"listen_ip": "127.0.0.1", "ws_port": 3030,
                                 "http_port": 3031, "db_port": 8000},
                       "timeouts": {}, "credentials": {"db_password": "test-db", "admin_password": "test-admin"}}
            lines = []
            for section, values in payload.items():
                lines.append(f"[{section}]")
                lines.extend(f"{key} = {json.dumps(value)}" for key, value in values.items())
            path = workspace / "guest.toml"
            path.write_text("\n".join(lines))
            guest = Guest(path)
            self.assertEqual(guest.installer("baseline")[-1], "v0.3.79")
            self.assertEqual(guest.environment("baseline")["SHERPA_ARTIFACT_DIR"], str(workspace / "baseline"))
            self.assertEqual(guest.installer()[-1], "v0.3.80")

    def test_initialization_allows_child_to_exit_after_terminal_closes(self):
        guest = Guest.__new__(Guest)
        guest.base = Path("/unused-test-guest")
        guest.credentials = {"admin_username": "test-admin", "admin_password": "test-only"}
        guest.timeouts = {"initialize": 1, "poll": 1}
        guest.passwords = []
        guest.snapshot = lambda: None
        guest.command = lambda *args: (0, "")
        with patch.object(pty, "fork", return_value=(1234, 99)), \
             patch.object(time, "monotonic", return_value=0), patch.object(time, "sleep"), \
             patch.object(select, "select", return_value=([99], [], [])), \
             patch.object(os, "read", side_effect=[b"Password for admin: ", b"Confirm password: ",
                                                    b"Username: ", b"Password for test-admin: ",
                                                    b"Confirm password: ",
                                                    b"Make this user an administrator? [Y/n]: ",
                                                    b"SSH public key for test-admin (file path or key, "
                                                    b"Enter to skip): ", OSError("PTY closed")]), \
             patch.object(os, "write"), \
             patch.object(os, "waitpid", side_effect=[(0, 0)] * 8 + [(1234, 0)]), \
             patch.object(os, "killpg") as terminate, patch.object(os, "close"):
            guest.initialize()
            terminate.assert_not_called()

    def test_initialization_timeout_reports_timeout_after_reaping_child(self):
        guest = Guest.__new__(Guest)
        guest.base = Path("/unused-test-guest")
        guest.credentials = {"admin_username": "test-admin", "admin_password": "test-only"}
        guest.timeouts = {"initialize": 1, "poll": 1}
        guest.passwords = []
        with patch.object(pty, "fork", return_value=(1234, 99)), \
             patch.object(time, "monotonic", side_effect=[0, 2, 2]), \
             patch.object(os, "waitpid", side_effect=[(0, 0), (1234, 9), ChildProcessError()]), \
             patch.object(os, "killpg"), patch.object(os, "close"):
            with self.assertRaises(TimeoutError):
                guest.initialize()

    def test_systemd_journal_satisfies_server_logging_without_a_log_file(self):
        guest = Guest.__new__(Guest)
        guest.base = Path("/nonexistent-sherpa-test-installation")
        guest.command = lambda *args, **kwargs: (0, "server/database log entry\n")
        guest.diagnostics(required=True)

    def test_retention_checks_detect_removed_or_changed_resources(self):
        before = {"packages": ["docker:1", "libvirt:2"], "users": ["sherpa:1000"],
                  "groups": ["docker:999:sherpa"], "images": ["sha256:db", "sha256:router"],
                  "networks": ["sherpa-bridge:uuid"], "pools": ["sherpa-pool:uuid"],
                  "domains": [], "volumes": [], "containers": []}
        validate_retained_resources(before, before)
        for category in before:
            after = dict(before, **{category: ["unexpected"]})
            with self.subTest(category=category), self.assertRaises(RuntimeError):
                validate_retained_resources(before, after)

    def test_live_diagnostics_require_both_log_sources(self):
        guest = Guest.__new__(Guest)
        guest.base = Path("/nonexistent-sherpa-test-installation")
        guest.command = lambda *args, **kwargs: (1, "missing")
        with self.assertRaises(RuntimeError):
            guest.diagnostics(required=True)

    def test_test_summary_counts_every_binary_and_names_failures(self):
        output = ("running 2 tests\ntest a::ok ... ok\ntest a::bad ... FAILED\n"
                  "test result: FAILED. 1 passed; 1 failed; 3 ignored; 0 measured\n"
                  "test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured\n")
        summary = summarize_tests(output)
        self.assertEqual((summary["binaries"], summary["passed"], summary["failed"], summary["ignored"]),
                         (2, 5, 1, 3))
        self.assertEqual(summary["failures"], ["a::bad"])
        self.assertEqual(test_problems(101, summary), ["cargo exited 101", "1 failed"])

    def test_missing_results_and_required_skips_are_failures(self):
        self.assertEqual(test_problems(0, summarize_tests("error: could not compile\n")),
                         ["no test results were reported"])
        skipped = summarize_tests("Skipping VM test: /dev/kvm not available\n"
                                  "test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured\n")
        self.assertEqual(test_problems(0, skipped),
                         ["required test skipped: Skipping VM test: /dev/kvm not available"])
        optional = summarize_tests("Skipping TAP test on kernel 6.8\n"
                                   "test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured\n")
        self.assertEqual(test_problems(0, optional), [])
        self.assertEqual(optional["skips"], ["Skipping TAP test on kernel 6.8"])

    def test_leftovers_report_only_new_resources(self):
        before = {"containers": ["sherpa-db"], "links": ["lo", "eth0"]}
        after = {"containers": ["sherpa-db", "srl1-tc000001"], "links": ["eth0", "lo"], "domains": ["vm1-x"]}
        self.assertEqual(leftover_resources(before, after),
                         {"containers": ["srl1-tc000001"], "domains": ["vm1-x"]})

    def test_results_document_is_valid_toml(self):
        document = toml_document({"integration-db": {"status": "passed", "passed": 3, "failures": [],
                                                      "skips": ['quote " and \\ slash']}})
        self.assertEqual(tomllib.loads(document)["integration-db"]["skips"], ['quote " and \\ slash'])

    def test_cargo_environment_names_the_running_user(self):
        guest = Guest.__new__(Guest)
        guest.workspace = Path("/var/tmp/workspace")
        guest.config = {"tests": {"test_db_port": 42069}}
        guest.credentials = {"test_db_password": "secret"}
        account = pwd.struct_passwd(("tester", "x", 1000, 1000, "", "/home/tester", "/bin/bash"))
        with patch.dict(os.environ, {"SUDO_USER": "tester"}), \
             patch.object(pwd, "getpwnam", return_value=account):
            user = guest.cargo_environment(root=False)
            root = guest.cargo_environment(root=True)
        self.assertEqual((user["USER"], user["LOGNAME"], user["HOME"]), ("tester", "tester", "/home/tester"))
        self.assertEqual((root["USER"], root["HOME"]), ("root", "/root"))
        self.assertEqual(root["CARGO_HOME"], "/home/tester/.cargo")
        self.assertNotIn("SHERPA_DB_PASSWORD", user)

    def test_ubuntu_version_ignores_build_suffix(self):
        self.assertEqual(ubuntu_version("26.04"), "26.04")
        self.assertEqual(ubuntu_version("24.04-20260926"), "24.04")

    def test_binary_version_requires_exact_match(self):
        validate_binary_version("sherpad", "v0.3.79", "sherpad 0.3.79\n")
        for output in ("sherpad 0.3.790", "other 0.3.79"):
            with self.assertRaises(RuntimeError):
                validate_binary_version("sherpad", "v0.3.79", output)

    def test_refuses_host_or_another_vm(self):
        for actual_uuid, actual_hostname in (("other", "test"), ("expected", "production")):
            with self.assertRaises(RuntimeError):
                validate_identity("expected", "test", actual_uuid, actual_hostname)

    def test_guest_uuid_and_fqdn_match(self):
        validate_identity("abCD", "test", "ABcd", "test.sherpa.lab.local")

    def test_failed_assertion_is_a_failure(self):
        with self.assertRaises(RuntimeError):
            require(False, "Required check failed")


if __name__ == "__main__":
    if sys.argv[1:] == ["--self-test"]:
        unittest.main(argv=[sys.argv[0]])
    else:
        try:
            Guest(Path(sys.argv[1])).run(sys.argv[2])
        except (Exception, KeyboardInterrupt) as error:
            print(f"FAIL: {error}", file=sys.stderr)
            sys.exit(1)
