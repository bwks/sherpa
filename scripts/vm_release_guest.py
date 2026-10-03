#!/usr/bin/env python3
"""Guest-only checks. Never invoke this helper on the Sherpa host."""

import fcntl
import hashlib
import json
import os
from pathlib import Path
import pty
import pwd
import select
import signal
import socket
import ssl
import subprocess
import sys
import time
import tomllib
import unittest
import urllib.error
import urllib.request


def require(condition, message):
    if not condition:
        raise RuntimeError(message)


def validate_identity(expected_uuid, expected_hostname, actual_uuid, actual_hostname):
    require(actual_uuid.lower() == expected_uuid.lower(), "Target VM UUID does not match")
    require(actual_hostname.split(".")[0] == expected_hostname, "Target VM hostname does not match")


def digest(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def validate_binary_version(binary, version, output):
    require(output.split() == [binary, version.removeprefix("v")],
            f"Unexpected {binary} version")


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

    def command(self, command, timeout=None, env=None, check=True):
        process = subprocess.Popen(command, cwd=self.workspace, stdin=subprocess.DEVNULL,
                                   stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                                   text=True, env=env, start_new_session=True)
        try:
            output, _ = process.communicate(timeout=timeout or self.timeouts["command"])
        except subprocess.TimeoutExpired:
            os.killpg(process.pid, signal.SIGTERM)
            try:
                output, _ = process.communicate(timeout=self.timeouts["terminate"])
            except subprocess.TimeoutExpired:
                os.killpg(process.pid, signal.SIGKILL)
                output, _ = process.communicate()
            print(self.redact(output), flush=True)
            raise TimeoutError(f"Command timed out: {command[0]}")
        print(self.redact(output), end="", flush=True)
        if check:
            require(process.returncode == 0, f"Command failed ({process.returncode}): {command[0]}")
        return process.returncode, output

    def environment(self):
        env = dict(os.environ)
        env.update(SHERPA_DB_PASSWORD=self.credentials["db_password"],
                   SHERPA_SERVER_IPV4=self.settings["listen_ip"],
                   SHERPA_SERVER_WS_PORT=str(self.settings["ws_port"]),
                   SHERPA_SERVER_HTTP_PORT=str(self.settings["http_port"]),
                   SHERPA_DB_PORT=str(self.settings["db_port"]),
                   DEBIAN_FRONTEND="noninteractive")
        if self.config["candidate"]["artifact_directory"]:
            env["SHERPA_ARTIFACT_DIR"] = str(self.workspace)
        return env

    def installer(self):
        return ["bash", str(self.workspace / "sherpa_install.sh"),
                "--version", self.config["candidate"]["version"]]

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

    def install(self):
        self.command(self.installer(), timeout=self.timeouts["install"], env=self.environment())
        self.verify_installation()

    def verify_installation(self):
        for binary in ("sherpa", "sherpad"):
            path = self.base / "bin" / binary
            require(digest(path) == self.config["candidate"][f"{binary}_binary_sha256"],
                    f"Installed {binary} does not match the candidate artifact")
            require(path.stat().st_mode & 0o777 == 0o755, f"Invalid {binary} permissions")
            require(path.stat().st_uid == pwd.getpwnam("sherpa").pw_uid, f"Invalid {binary} owner")
            link = Path("/usr/local/bin") / binary
            require(link.is_symlink() and link.resolve() == path, f"Invalid {binary} symlink")
            _, output = self.command([str(path), "--version"])
            validate_binary_version(binary, self.config["candidate"]["version"], output)
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
                if not waited:
                    os.killpg(pid, signal.SIGKILL)
                    os.waitpid(pid, 0)
                    raise TimeoutError("Server initialization timed out")
            require(os.waitstatus_to_exitcode(status) == 0 and not prompts, "Server initialization failed")
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

    def reinstall(self):
        self.install()
        self.verify_retained()
        self.command(["systemctl", "start", "sherpad"])

    def uninstall(self, mode):
        self.command(["bash", str(self.workspace / "sherpa_uninstall.sh"), mode, "--force"])
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

    def diagnostics(self):
        commands = (["systemctl", "status", "--no-pager", "sherpad", "docker", "libvirtd"],
                    ["journalctl", "--no-pager", "-u", "sherpad", "-n", "200"],
                    ["journalctl", "--no-pager", "-b", "-p", "warning", "-n", "100"],
                    ["docker", "logs", "--tail", "100", "sherpa-db"],
                    ["docker", "ps", "-a", "--format", "{{.Names}} {{.Status}}"],
                    ["cloud-init", "status", "--long"],
                    ["df", "-h"])
        for command in commands:
            try:
                self.command(command, check=False)
            except FileNotFoundError:
                print(f"Diagnostic tool not installed: {command[0]}")
        log = self.base / "logs/sherpad.log"
        if log.is_file():
            self.command(["tail", "-n", "200", str(log)])

    def run(self, step):
        self.guard(baseline=step == "baseline")
        actions = {"baseline": self.baseline, "preflight": self.preflight,
                   "install": self.install, "initialize": self.initialize,
                   "authenticate": self.authenticate, "reboot": self.reboot,
                   "reinstall": self.reinstall, "restore": self.reinstall,
                   "keep-data": lambda: self.uninstall("--keep-data"),
                   "remove-data": lambda: self.uninstall("--remove-data"),
                   "remove-all": lambda: self.uninstall("--remove-all"),
                   "repeat-uninstall": lambda: self.uninstall("--remove-all"),
                   "diagnostics": self.diagnostics,
                   "new-database": lambda: (self.install(), self.initialize())}
        require(step in actions, "Unknown guest step")
        actions[step]()


class GuestTests(unittest.TestCase):
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
        except Exception as error:
            print(f"FAIL: {error}", file=sys.stderr)
            sys.exit(1)
