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
import signal
import socket
import ssl
import subprocess
import sys
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

    def command(self, command, timeout=None, env=None, check=True, display=True):
        process = subprocess.Popen(command, cwd=self.workspace, stdin=subprocess.DEVNULL,
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
        self.command(["cloud-init", "status", "--wait", "--long"],
                     timeout=self.timeouts["initialize"])
        release = dict(line.split("=", 1) for line in Path("/etc/os-release").read_text().splitlines()
                       if "=" in line)
        require(release["ID"].strip('"') == "ubuntu"
                and release["VERSION_ID"].strip('"') == self.config["vm"]["version"],
                "Guest operating system differs from the manifest")
        print("OS: " + release["PRETTY_NAME"])
        require(os.cpu_count() == self.config["vm"]["cpu_count"], "Guest vCPU count differs from the manifest")
        _, disk = self.command(["lsblk", "-bdno", "SIZE", "/dev/vda"])
        require(int(disk.strip()) == self.config["vm"]["boot_disk_size"] * 1024**3,
                "Guest disk size differs from the manifest")
        for binary in ("sherpa", "sherpad", "docker", "dockerd", "virsh", "surreal"):
            code, _ = self.command(["bash", "-c", 'command -v "$1"', "bash", binary], check=False)
            require(code != 0, f"Preinstalled runtime command: {binary}")
        require(not self.base.exists(), "/opt/sherpa already exists")
        with open("/dev/kvm", "rb", buffering=0) as device:
            require(fcntl.ioctl(device.fileno(), 0xAE00, 0) == 12, "KVM API unavailable")
            descriptor = fcntl.ioctl(device.fileno(), 0xAE01, 0)
            os.close(descriptor)
        require(pwd.getpwnam("sherpa").pw_uid >= 1000, "Expected Sherpa's cloud-init login account")
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
        for service in ("docker", "libvirtd"):
            self.command(["systemctl", "is-active", service])
        self.command(["systemctl", "is-enabled", "sherpad"])
        unit = Path("/etc/systemd/system/sherpad.service").read_text()
        require("ExecStart=/opt/sherpa/bin/sherpad start --foreground" in unit, "Incorrect service command")
        require("Requires=docker.service libvirtd.service" in unit, "Missing service dependencies")
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
        prompts = [(b"Admin username: ", self.credentials["admin_username"]),
                   (f"Password for {self.credentials['admin_username']}: ".encode(),
                    self.credentials["admin_password"]),
                   (b"Confirm password: ", self.credentials["admin_password"])]
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
        for service in ("docker", "libvirtd"):
            self.command(["systemctl", "is-active", service])
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
                   "new-database": lambda: (self.install(), self.initialize())}
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
             patch.object(os, "read", side_effect=[b"Admin username: ", b"Password for test-admin: ",
                                                    b"Confirm password: ", OSError("PTY closed")]), \
             patch.object(os, "write"), \
             patch.object(os, "waitpid", side_effect=[(0, 0)] * 4 + [(1234, 0)]), \
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
