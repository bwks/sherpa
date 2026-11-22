#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.15"
# dependencies = []
# ///
"""Guest-only checks (Python 3.15+, standard library only). The harness runs this helper on the
Python 3.15 that it installs on the guest with uv, never on the guest's system Python. Never invoke
this helper on the Sherpa host."""

import fcntl
from contextlib import redirect_stdout, suppress
import grp
import hashlib
import io
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


def check_cases(cases):
    """Run each case's check, given as (case ID, check), and report it by its ID with PASS or FAIL;
    a failed case does not stop the next. The step then fails naming every case that failed."""
    failed = []
    for case, check in cases:
        try:
            check()
        except TimeoutError:
            print(f"FAIL {case}: timed out", flush=True)
            raise
        except (RuntimeError, OSError, KeyError) as error:
            failed.append(case)
            print(f"FAIL {case}: {error}", flush=True)
        else:
            print(f"PASS {case}", flush=True)
    require(not failed, "Failed cases: " + ", ".join(failed))


def authenticate_case(previous_step, upgrade):
    """The case an authenticate step completes after the step before it, or None. In the upgrade
    scenario the verify steps report the update's cases, and the server first set up is the
    earlier release's."""
    return None if upgrade else AUTHENTICATE_CASES.get(previous_step)


def account_exists(name):
    try:
        pwd.getpwnam(name)
    except KeyError:
        return False
    return True


def check_mode(path, mode, owner=None, group=None):
    """`path` has `mode`, special bits included, and the owner and group given."""
    status = path.stat()
    require(status.st_mode & 0o7777 == mode, f"{path} is {status.st_mode & 0o7777:o}, not {mode:o}")
    if owner:
        require(status.st_uid == pwd.getpwnam(owner).pw_uid, f"{path} is not owned by {owner}")
    if group:
        require(status.st_gid == grp.getgrnam(group).gr_gid, f"{path} is not in the {group} group")


def blkid(path, tag):
    return subprocess.run(["blkid", "-p", "-o", "value", "-s", tag, str(path)],
                          capture_output=True, text=True, check=False).stdout.strip()


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
THP_ENABLED = Path("/sys/kernel/mm/transparent_hugepage/enabled")
# Each integration crate's test executables, written by the test build for the test runs.
TEST_EXECUTABLES_FILE = "test-executables.toml"
# sccache's configuration, naming the host's build cache; written only while the cache is usable.
SCCACHE_CONFIG_FILE = "sccache.toml"
SCCACHE_BINARY = "tools/sccache"
# Build output, outside the source tree so a rerun can replace the source, and at the same path
# in every run: crates that generate code embed their output path, and a path that changes
# with the run changes their cache keys and those of every crate depending on them.
CARGO_TARGET_DIR = "/var/tmp/sherpa-release-test-target"
# The live tailnet tests join a real tailnet, so they run only when the host passed a key. Their
# gateway gets a disposable Docker network, created and removed around the integration tests.
TAILNET_TESTS = [
    "tailscale::tests::tailnet_join_003_a_gateway_enrolled_with_a_valid_key_reports_connected",
    "tailscale::tests::tailnet_suspend_003_a_stopped_gateway_reports_stopped_and_enrolled_again_connected",
]
TAILNET_SUBNET = "172.31.250.0/24"
TAILNET_GATEWAY_IPV4 = "172.31.250.10"
THP_BOOT_RULE = Path("/etc/tmpfiles.d/sherpa-thp.conf")
# The case an authenticate step completes, by the step before it (see authenticate_case).
AUTHENTICATE_CASES = {"initialize": "server_init_001", "reboot": "install_host_008",
                      "reinstall": "install_host_009", "restore": "uninstall_script_002",
                      "new-database": "uninstall_script_004"}
# The steps after which the guest records no previous step: they test nothing of Sherpa.
UNRECORDED_STEPS = ("diagnostics", "diagnostics-live", "fault-wait", "cancel")
PREVIOUS_STEP_FILE = "previous-step"
# Each blank disk template as (name, bytes, file system, label, FAT version): what the ZTP code in
# crates/server expects. None is none.
BLANK_DISKS = (("base.img", 67108864, None, None, None),
               ("fat16.img", 67108864, "vfat", None, "FAT16"),
               ("fat32.img", 67108864, "vfat", None, "FAT32"),
               ("junos.img", 67108864, "vfat", "vmm-data", "FAT16"),
               ("iosv.img", 67092480, "vfat", None, "FAT16"),
               ("ise.img", 10485760, "ext4", "ISE-ZTP", None),
               ("ext4-100mb.img", 104857600, "ext4", "data-disk", None),
               ("ext4-200mb.img", 209715200, "ext4", "data-disk", None),
               ("ext4-500mb.img", 524288000, "ext4", "data-disk", None),
               ("ext4-1g.img", 1073741824, "ext4", "data-disk", None),
               ("ext4-2g.img", 2147483648, "ext4", "data-disk", None),
               ("ext4-3g.img", 3221225472, "ext4", "data-disk", None),
               ("ext4-4g.img", 4294967296, "ext4", "data-disk", None),
               ("ext4-5g.img", 5368709120, "ext4", "data-disk", None))


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


def package_name(package_id):
    """The package name in a Cargo package ID, such as `path+file:///src/crates/server#sherpad@0.5.12`.
    Cargo leaves the name out when it is the directory's name: `path+file:///src/crates/db#0.5.12`."""
    location, _, fragment = package_id.partition("#")
    if "@" in fragment:
        return fragment.split("@", 1)[0]
    return location.rstrip("/").rsplit("/", 1)[-1]


def test_executables(build_output, crates):
    """Each crate's test executables, from `cargo test --no-run --message-format=json` output:
    `{crate: {"paths": [executable], "directories": [package directory]}}` in build order."""
    executables = {crate: {"paths": [], "directories": []} for crate in crates}
    for line in build_output.splitlines():
        if not line.startswith("{"):
            continue
        message = json.loads(line)
        if message.get("reason") != "compiler-artifact" or not message.get("executable"):
            continue
        if not message["profile"]["test"]:
            continue
        crate = package_name(message["package_id"])
        if crate in executables:
            executables[crate]["paths"].append(message["executable"])
            executables[crate]["directories"].append(str(Path(message["manifest_path"]).parent))
    return executables


def test_problems(code, summary):
    problems = []
    if code:
        problems.append(f"cargo exited {code}")
    if not summary["binaries"]:
        problems.append("no test results were reported")
    if summary["failed"] or summary["failures"]:
        problems.append(f"{max(summary['failed'], len(summary['failures']))} failed")
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
    required = {"config", "environment", "server_key", "users"}
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


# Where the update script keeps its backups of the database and its key.
UPDATE_BACKUP_ROOT = Path("/var/backups/sherpa")
# What `sherpa up` says when the lab owner has no device password.
DEVICE_PASSWORD_REFUSAL = "has no device password, and labs need one"
# The first release whose installer makes the database directory 0750; older ones made it 0775.
PRIVATE_DATABASE_DIRECTORY_VERSION = (0, 5, 13)
# The first release whose installer makes the config folder 0750 and the env file 0600; older
# ones made them 0775 and 0640.
PRIVATE_CONFIG_VERSION = (0, 6, 6)
# The first release whose sherpad makes the database key, at its first start.
DATABASE_KEY_VERSION = (0, 6, 4)
# What the server keeps across a reboot, a reinstall, an update and an uninstall that keeps
# /opt/sherpa, by its name in retained.toml: its path under /opt/sherpa, and the first release
# that makes it.
RETAINED_FILES = {"config": ("config/sherpa.toml", (0, 5, 0)),
                  "server_key": (".certs/server.key", (0, 5, 0)),
                  "database_key": (".secret/db.key", DATABASE_KEY_VERSION)}
RETAINED_FILE = "retained.toml"
# The settings the harness gives the installer, which it writes to sherpa.env.
INSTALLER_SETTINGS = ("SHERPA_SERVER_IPV4", "SHERPA_SERVER_WS_PORT", "SHERPA_SERVER_HTTP_PORT")


def release_numbers(version):
    """The numbers of a Sherpa release such as v0.5.0, for comparing releases."""
    return tuple(int(part) for part in version.removeprefix("v").split("-", 1)[0].split("."))


def database_directory_mode(version):
    """The mode a Sherpa release's installer gives /opt/sherpa/db."""
    return 0o750 if release_numbers(version) >= PRIVATE_DATABASE_DIRECTORY_VERSION else 0o775


def base_layout(selection):
    """The mode and owner of /opt/sherpa and the binaries that an install or update of
    `selection` leaves: the checkout's scripts install the candidate, root's and 0755; the
    baseline's own installer left them sherpa's, and the base folder 0775."""
    return (0o755, "root") if selection == "candidate" else (0o775, "sherpa")


def config_modes(version):
    """The modes a Sherpa release's installer gives /opt/sherpa/config and env/sherpa.env."""
    if release_numbers(version) >= PRIVATE_CONFIG_VERSION:
        return 0o750, 0o600
    return 0o775, 0o640


def tailnet_test_environment(credentials, network):
    """The live tailnet test's settings, or None when the run has no tailnet key."""
    key = credentials.get("tailnet_auth_key")
    if not key:
        return None
    return {"SHERPA_TEST_TAILSCALE_AUTH_KEY": key, "SHERPA_TEST_TAILSCALE_NETWORK": network,
            "SHERPA_TEST_TAILSCALE_IPV4": TAILNET_GATEWAY_IPV4, "SHERPA_TEST_TAILSCALE_ROUTES": TAILNET_SUBNET}


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
    require(os.cpu_count() == vm["cpu"]["count"], "Guest vCPU count differs from the manifest")
    _, disk = command(["lsblk", "-bdno", "SIZE", "/dev/vda"])
    require(int(disk.strip()) == vm["boot_disk"]["size"] * 1024**3,
            "Guest disk size differs from the manifest")
    for binary in ("sherpa", "sherpad", "docker", "dockerd", "virsh"):
        code, _ = command(["bash", "-c", 'command -v "$1"', "bash", binary], check=False)
        require(code != 0, f"Preinstalled runtime command: {binary}")
    require(not Path("/opt/sherpa").exists(), "/opt/sherpa already exists")
    with open("/dev/kvm", "rb", buffering=0) as device:
        require(fcntl.ioctl(device.fileno(), 0xAE00, 0) == 12, "KVM API unavailable")
        descriptor = fcntl.ioctl(device.fileno(), 0xAE01, 0)
        os.close(descriptor)
    # The node's only login is its lab owner's; the installer makes the `sherpa` service account.
    login = os.environ.get("SUDO_USER", "")
    require(login and login != "root" and pwd.getpwnam(login).pw_uid >= 1000,
            "Expected the lab owner's login account")
    with suppress(KeyError):
        pwd.getpwnam("sherpa")
        raise RuntimeError("The node has a sherpa account: Sherpa gives nodes only the lab owner's login")


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
        self.passwords = [self.credentials["admin_password"]]
        for key in ("device_password", "build_cache_password", "tailnet_auth_key"):
            if key in self.credentials:
                self.passwords.append(self.credentials[key])

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
        env.update(SHERPA_SERVER_IPV4=self.settings["listen_ip"],
                   SHERPA_SERVER_WS_PORT=str(self.settings["ws_port"]),
                   SHERPA_SERVER_HTTP_PORT=str(self.settings["http_port"]),
                   DEBIAN_FRONTEND="noninteractive")
        env.pop("SHERPA_ARTIFACT_DIR", None)
        if selection == "baseline":
            env["SHERPA_ARTIFACT_DIR"] = str(self.workspace / "baseline")
        elif self.config[selection]["artifact_directory"]:
            env["SHERPA_ARTIFACT_DIR"] = str(self.workspace)
        return env

    def installer(self, selection="candidate"):
        # The runner ships the baseline release's own installer beside its archives.
        directory = self.workspace / "baseline" if selection == "baseline" else self.workspace
        return ["bash", str(directory / "sherpa_install.sh"),
                "--version", self.config[selection]["version"]]

    def updater(self):
        return ["bash", str(self.workspace / "sherpa_update.sh"),
                "--version", self.config["candidate"]["version"]]

    def baseline(self):
        check_bare_ubuntu(self.command, self.config["vm"], self.timeouts["initialize"])
        self.command(["curl", "--version"])
        self.marker.write_text(self.config["identity"]["nonce"])
        self.marker.chmod(0o600)
        print("PASS bare baseline, cloud-init, nested KVM and guest identity")

    def preflight(self):
        check_cases([("install_preflight_010", self.preflight_refusals)])

    def preflight_refusals(self):
        """Each refusal of the install script a bare host can reach leaves no installation."""
        code, output = self.command(["bash", str(self.workspace / "sherpa_install.sh"), "--bogus"], check=False)
        require(code != 0 and "Unknown option" in output, "Invalid argument accepted")
        code, output = self.command(["sudo", "-u", self.login_user().pw_name, *self.installer()],
                                    env=self.environment(), check=False)
        require(code != 0 and "must be run as root" in output, "Non-root installer accepted")
        env = self.environment()
        env["SHERPA_SERVER_IPV4"] = "invalid"
        code, output = self.command(self.installer(), env=env, check=False)
        require(code != 0 and "Invalid IPv4" in output, "Invalid address accepted")
        env = self.environment()
        env["SHERPA_ARTIFACT_DIR"] = str(self.workspace / "missing")
        code, output = self.command(self.installer(), env=env, check=False)
        require(code != 0 and "artifact" in output.lower(), "Unavailable artifacts accepted")
        require(not self.base.exists(), "Preflight failures changed the installation")
        require(not account_exists("sherpa"), "Preflight failures created the sherpa account")

    def check_runtimes(self):
        """Docker and libvirt answer requests. libvirtd is socket-activated and exits when
        idle, so an inactive libvirtd.service is normal; the socket must accept a connection."""
        self.command(["systemctl", "is-active", "docker"])
        self.command(["systemctl", "is-active", "libvirtd.socket"])
        self.command(["virsh", "-c", "qemu:///system", "version"])

    def install(self, selection="candidate", fresh=False):
        """Install the release. `fresh` is the first install on a bare host: it reports the cases
        of install_host and blank_disks it shows."""
        self.command(self.installer(selection), timeout=self.timeouts["install"], env=self.environment(selection))
        if fresh:
            check_cases([("install_host_001", lambda: self.verify_binaries(selection)),
                         ("install_host_002", self.verify_accounts),
                         ("install_host_003", self.verify_service),
                         ("install_host_004", lambda: self.verify_environment(selection)),
                         ("install_host_005", self.verify_router_image),
                         ("install_host_006", self.verify_hugepages),
                         ("blank_disks_001", self.verify_blank_disks)])
        else:
            self.verify_installation(selection)

    def verify_installation(self, selection="candidate"):
        """What an install, a reinstall or an update leaves, the release's own modes included."""
        self.verify_binaries(selection)
        database_mode = database_directory_mode(self.config[selection]["version"])
        config_mode, env_file_mode = config_modes(self.config[selection]["version"])
        base_mode, base_owner = base_layout(selection)
        for directory, permissions, owner in (("", base_mode, base_owner), ("db", database_mode, "sherpa"),
                                              ("config", config_mode, "sherpa"), ("env", 0o750, "sherpa")):
            path = self.base / directory
            require(path.is_dir() and path.stat().st_mode & 0o777 == permissions,
                    f"Invalid permissions for {path}")
            require(path.stat().st_uid == pwd.getpwnam(owner).pw_uid, f"Invalid owner: {path}")
        env_file = self.base / "env/sherpa.env"
        require(env_file.stat().st_mode & 0o777 == env_file_mode,
                f"Environment permissions are not {env_file_mode:o}")
        require(env_file.stat().st_uid == pwd.getpwnam("sherpa").pw_uid, "Invalid environment owner")
        self.verify_environment(selection)
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
        if selection == "candidate":
            self.verify_hugepages()
        self.verify_router_image()

    def verify_binaries(self, selection):
        for binary in ("sherpa", "sherpad"):
            path = self.base / "bin" / binary
            require(digest(path) == self.config[selection][f"{binary}_binary_sha256"],
                    f"Installed {binary} does not match the {selection} artifact")
            require(path.stat().st_mode & 0o777 == 0o755, f"Invalid {binary} permissions")
            require(path.stat().st_uid == pwd.getpwnam(base_layout(selection)[1]).pw_uid,
                    f"Invalid {binary} owner")
            link = Path("/usr/local/bin") / binary
            require(link.is_symlink() and link.resolve() == path, f"Invalid {binary} symlink")
            _, output = self.command([str(path), "--version"])
            validate_binary_version(binary, self.config[selection]["version"], output)

    def verify_accounts(self):
        """Docker and libvirt run; `sherpa` cannot log in and is in their groups; the user who
        ran sudo is in the `sherpa` group."""
        self.check_runtimes()
        account = pwd.getpwnam("sherpa")
        require(account.pw_shell == "/usr/sbin/nologin", f"The sherpa account's shell is {account.pw_shell}")
        _, groups = self.command(["id", "-nG", "sherpa"])
        require({"libvirt", "docker", "kvm"}.issubset(groups.split()), "Missing runtime groups")
        installer = self.login_user().pw_name
        require(installer in grp.getgrnam("sherpa").gr_mem, f"{installer} is not in the sherpa group")

    def verify_service(self):
        """The unit is installed and enabled but not started, runs sherpad as `sherpa` with its
        capabilities, needs Docker and libvirt and keeps /run/sherpa; the log is rotated daily."""
        self.command(["systemctl", "is-enabled", "sherpad"])
        code, _ = self.command(["systemctl", "is-active", "--quiet", "sherpad"], check=False)
        require(code != 0, "The install started sherpad")
        unit = Path("/etc/systemd/system/sherpad.service").read_text().splitlines()
        for line in ("ExecStart=/opt/sherpa/bin/sherpad start --foreground", "User=sherpa",
                     "Requires=docker.service libvirtd.service",
                     "AmbientCapabilities=CAP_NET_ADMIN CAP_NET_RAW CAP_BPF CAP_PERFMON",
                     "RuntimeDirectory=sherpa", "RuntimeDirectoryMode=0700", "RuntimeDirectoryPreserve=yes"):
            require(line in unit, f"The service unit lacks {line}")
        rotation = Path("/etc/logrotate.d/sherpad").read_text()
        require("/opt/sherpa/logs/sherpad.log" in rotation and "daily" in rotation.split(),
                "sherpad.log is not rotated daily")

    def verify_environment(self, selection):
        """sherpa.env holds the address and ports the installer was given."""
        env_file = self.base / "env/sherpa.env"
        env_values = dict(line.split("=", 1) for line in env_file.read_text().splitlines()
                          if line and not line.startswith("#") and "=" in line)
        given = self.environment(selection)
        for key in INSTALLER_SETTINGS:
            require(env_values.get(key) == given[key], f"Environment setting mismatch: {key}")

    def verify_router_image(self):
        self.command(["docker", "image", "inspect", "--format", "{{.Id}}", self.settings["router_image"]])

    def verify_hugepages(self):
        thp = THP_ENABLED.read_text()
        require("[madvise]" in thp or "[always]" in thp, "Transparent hugepages are not madvise")
        if "[always]" not in thp:
            require(f"w {THP_ENABLED} - - - - madvise" in THP_BOOT_RULE.read_text(),
                    "Missing transparent hugepages boot rule")

    def verify_blank_disks(self):
        """Each blank disk template has its size, file system and label, 644 sherpa:sherpa, in a
        folder 775."""
        folder = self.base / "images/blank_disk"
        check_mode(folder, 0o775)
        for name, size, kind, label, version in BLANK_DISKS:
            disk = folder / name
            require(disk.is_file() and not disk.is_symlink(), f"Blank disk {name} is missing")
            require(disk.stat().st_size == size, f"Blank disk {name} is not {size} bytes")
            check_mode(disk, 0o644, "sherpa", "sherpa")
            require(blkid(disk, "TYPE") == (kind or ""), f"Blank disk {name} has the wrong file system")
            require(blkid(disk, "LABEL") == (label or ""), f"Blank disk {name} has the wrong label")
            if version:
                require(blkid(disk, "VERSION") == version, f"Blank disk {name} is not {version}")

    def verify_embedded_database(self):
        """The database is the file sherpad embeds."""
        require((self.base / "db/sherpa.sqlite").is_file(), "Embedded database file missing")

    def init_command(self):
        """sherpad init with the address and ports the installer was given, as the installer
        tells users to run it."""
        return ["sudo", "-u", "sherpa", "-H", str(self.base / "bin/sherpad"), "init",
                "--server-ipv4", self.settings["listen_ip"],
                "--ws-port", str(self.settings["ws_port"]),
                "--http-port", str(self.settings["http_port"])]

    def initialize(self):
        command = self.init_command()
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
        self.record_retained(("config",))
        self.command(["systemctl", "start", "sherpad"])

    def installed_version(self):
        """The version the installed sherpad reports, such as 0.7.1."""
        _, output = self.command([str(self.base / "bin/sherpad"), "--version"], display=False)
        return output.split()[1]

    def record_retained(self, names):
        """Record the digest of each file of `names` in RETAINED_FILES that the installed release
        makes and retained.toml has none for yet. The config is there after `sherpad init`, the
        certificate and the database key after sherpad first starts."""
        path = self.workspace / RETAINED_FILE
        hashes = tomllib.loads(path.read_text()) if path.exists() else {}
        release = release_numbers(self.installed_version())
        for name in names:
            relative, since = RETAINED_FILES[name]
            if name not in hashes and release >= since:
                hashes[name] = digest(self.base / relative)
        path.write_text("".join(f'{key} = "{value}"\n' for key, value in hashes.items()))

    def verify_retained(self):
        """Each file retained.toml records is as it was."""
        hashes = tomllib.loads((self.workspace / RETAINED_FILE).read_text())
        for name, value in hashes.items():
            relative = RETAINED_FILES[name][0]
            require(digest(self.base / relative) == value, f"{self.base / relative} changed")

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

    def write_client_files(self, home, token):
        """The CLI's token and server settings under `home`; returns the server URL."""
        client_base = home / ".sherpa"
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
        return url

    def previous_step(self):
        path = self.workspace / PREVIOUS_STEP_FILE
        return path.read_text().strip() if path.exists() else None

    def authenticate(self):
        """Wait for sherpad and log in. After a step whose case needs a running server, report that
        case; after the first set-up, the modes of the server's files too."""
        case = authenticate_case(self.previous_step(), "baseline" in self.config)
        if case is None:
            self.wait_authenticated()
            return
        cases = [(case, self.wait_authenticated)]
        if case == "server_init_001":
            cases += [("server_file_modes_001", self.verify_server_file_modes),
                      ("server_file_modes_002", self.verify_secret_readers),
                      ("server_file_modes_019", self.verify_writers)]
        check_cases(cases)

    def verify_server_file_modes(self):
        """Each path of the spec's table of the server's files has its mode and owner."""
        for path, mode, owner, group in (
                ("", 0o755, "root", "root"), ("bin", 0o755, "root", "root"),
                ("bin/sherpad", 0o755, "root", "root"), ("bin/sherpa", 0o755, "root", "root"),
                ("config", 0o750, "sherpa", "sherpa"), ("config/sherpa.toml", 0o600, "sherpa", "sherpa"),
                ("env", 0o750, "sherpa", "sherpa"), ("env/sherpa.env", 0o600, "sherpa", "sherpa"),
                ("env/sherpa.env.example", 0o600, "sherpa", "sherpa"),
                (".secret", 0o700, "sherpa", "sherpa"), (".certs", 0o700, "sherpa", "sherpa"),
                (".certs/server.key", 0o600, None, None),
                ("db", 0o750, "sherpa", "sherpa"), ("db/backups", 0o700, "sherpa", "sherpa"),
                ("logs", 0o750, "sherpa", "sherpa"),
                ("labs", 0o2750, "sherpa", "sherpa"), ("libvirt", 0o755, "root", "root"),
                ("libvirt/images", 0o2750, "sherpa", "kvm"), ("images", 0o775, "sherpa", "sherpa"),
                ("images/blank_disk", 0o775, "sherpa", "sherpa"), ("containers", 0o755, "sherpa", "sherpa"),
                ("bins", 0o755, "sherpa", "sherpa"), ("run", 0o755, "sherpa", "sherpa")):
            check_mode(self.base / path, mode, owner, group)
        for folder in (".secret", "db"):
            for file in (self.base / folder).iterdir():
                if file.is_file():
                    check_mode(file, 0o600, "sherpa", "sherpa")
        log = self.base / "logs/sherpad.log"
        if log.exists():
            check_mode(log, 0o640)
        for disk in (self.base / "images/blank_disk").iterdir():
            check_mode(disk, 0o644, "sherpa", "sherpa")
        for path in (Path("/etc/systemd/system/sherpad.service"), Path("/etc/logrotate.d/sherpad"), THP_BOOT_RULE):
            if path != THP_BOOT_RULE or path.exists():
                check_mode(path, 0o644, "root")

    def verify_secret_readers(self):
        """sherpa.toml and the env files are read by `sherpa` and by no one else, the user who
        installed, in the `sherpa` group, included."""
        for file in ("config/sherpa.toml", "env/sherpa.env", "env/sherpa.env.example"):
            path = str(self.base / file)
            self.command(["runuser", "-u", "sherpa", "--", "test", "-r", path])
            for account in (self.login_user().pw_name, "nobody"):
                code, _ = self.command(["runuser", "-u", account, "--", "test", "-r", path], check=False)
                require(code != 0, f"{account} can read {path}")

    def verify_writers(self):
        """The user who installed, in the `sherpa` group, can write nothing in /opt/sherpa but the
        images and blank disk folders, and `sherpa` cannot write what systemd and the scripts run
        as root."""
        installer = self.login_user().pw_name
        writable = {self.base / "images", self.base / "images/blank_disk"}
        for path in [self.base, *self.base.rglob("*")]:
            if path.is_symlink() or path in writable:
                continue
            code, _ = self.command(["runuser", "-u", installer, "--", "test", "-w", str(path)],
                                   check=False, display=False)
            require(code != 0, f"{installer}, in the sherpa group, can write {path}")
        for path in (self.base, self.base / "bin", self.base / "bin/sherpad", self.base / "bin/sherpa",
                     self.base / "libvirt", Path("/etc/systemd/system/sherpad.service")):
            code, _ = self.command(["runuser", "-u", "sherpa", "--", "test", "-w", str(path)],
                                   check=False, display=False)
            require(code != 0, f"sherpa can write {path}")

    def wait_authenticated(self):
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
                url = self.write_client_files(Path.home(), token)
                _, output = self.command([str(self.base / "bin/sherpa"), "whoami", "--server-url", url])
                require("Authenticated" in output and self.credentials["admin_username"] in output,
                        "CLI did not validate the test admin")
                self.record_retained(RETAINED_FILES)
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
                 "environment": environment_fingerprint(self.base / "env/sherpa.env"),
                 "server_key": digest(self.base / ".certs/server.key")}
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
        # The device password state is new in the candidate, so a baseline record lacks it; the
        # `device-password` step checks it. Everything else must be unchanged.
        state["users"] = [json.dumps({key: value for key, value in entry.items()
                                      if key != "device_password_set"}, sort_keys=True)
                          for entry in selected]
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
        print("PASS baseline persistent users, settings and server key recorded")

    def upgrade(self):
        """Upgrade with the update script, as users do."""
        evidence = tomllib.loads((self.workspace / "upgrade-state.toml").read_text())
        validate_upgrade_state(evidence["before"], self.upgrade_state())
        _, output = self.command(self.updater(), timeout=self.timeouts["install"], env=self.environment())
        check_cases([("update_script_025", lambda: self.verify_update_backup(output))])
        self.verify_installation()
        self.verify_retained()
        self.command(["systemctl", "start", "sherpad"])

    def verify_update_backup(self, output):
        """The backup folder the update names holds the database, root's only, and the key, when
        the server had one, is the server's."""
        found = re.findall(r"Backup of the database and its key: (\S+)", output)
        require(found, "The update named no backup of the database")
        folder = Path(found[-1])
        require(folder.parent == UPDATE_BACKUP_ROOT, f"The backup is not in {UPDATE_BACKUP_ROOT}: {folder}")
        check_mode(folder, 0o700, "root", "root")
        check_mode(folder / "sherpa.sqlite", 0o600, "root", "root")
        key = folder / "db.key"
        if key.exists():
            check_mode(key, 0o600, "root", "root")
            require(key.read_bytes() == (self.base / ".secret/db.key").read_bytes(),
                    "The backed-up key is not the server's")

    def verify_upgrade(self, phase):
        case = "update_script_001" if phase == "after" else "update_script_002"
        check_cases([(case, lambda: self.verify_upgraded(phase))])

    def verify_upgraded(self, phase):
        """This release's binaries, and the users, settings and server key the server held."""
        self.verify_installation()
        self.verify_embedded_database()
        evidence = tomllib.loads((self.workspace / "upgrade-state.toml").read_text())
        state = self.upgrade_state()
        validate_upgrade_state(evidence["before"], state)
        evidence[phase] = state
        evidence["candidate-binaries" if phase == "after" else "reboot-binaries"] = self.installed_hashes()
        if phase == "reboot":
            validate_upgrade_evidence(evidence, self.config["baseline"], self.config["candidate"])
        self.write_resources(self.workspace / "upgrade-state.toml", evidence)
        print(f"PASS {phase}: candidate binaries and unchanged users, settings and server key")

    def device_password_state(self, token, username):
        """Whether `username` has set a device password, read from their own record with their
        token: listing every user is for admins only."""
        return self.request(f"/api/v1/admin/users/{username}", token=token)["user"].get("device_password_set")

    def lab_up_as(self, home, directory):
        """`sherpa up` for a one-node lab as the user whose CLI is under `home`; returns the exit
        code and output. It never gets as far as a VM: no node image is imported on the guest."""
        directory.mkdir(exist_ok=True)
        (directory / "manifest.toml").write_text(
            f'name = "{directory.name}"\n\n[[nodes]]\nname = "n1"\nmodel = "ubuntu_linux"\n'
            f'version = "{self.config["vm"]["version"]}"\n')
        env = {**os.environ, "HOME": str(home)}
        return self.command([str(self.base / "bin/sherpa"), "up"], env=env, check=False,
                            cwd=directory, timeout=self.timeouts["command"])

    def device_password(self):
        """After the update, the upgraded user's labs are refused until they set a device
        password, and are not refused for one after. It runs after the other upgrade checks, as
        it changes the user's record."""
        username = "persist-" + self.config["identity"]["nonce"][:8]
        device_password = self.credentials["device_password"]
        user = self.login(username)
        token = user["token"]
        require(self.device_password_state(token, username) is False,
                "The upgraded user has a device password they never set")
        # A lab needs the owner's SSH key before it needs the device password.
        key = self.workspace / "persist-key"
        self.command(["ssh-keygen", "-q", "-t", "ed25519", "-N", "", "-C", username, "-f", str(key)])
        added = self.request(f"/api/v1/admin/users/{username}/ssh-keys",
                             {"username": username, "ssh_key": key.with_suffix(".pub").read_text().strip(),
                              "token": token}, token=token)
        require(added, "The upgraded user's SSH key was not added")
        home = self.workspace / "persist-home"
        home.mkdir(exist_ok=True, mode=0o700)
        self.write_client_files(home, token)
        directory = self.workspace / ("persist-lab-" + self.config["identity"]["nonce"][:8])
        code, output = self.lab_up_as(home, directory)
        require(code != 0 and DEVICE_PASSWORD_REFUSAL in output,
                f"Labs were not refused for the missing device password (exit {code})")
        require(f"'{username}'" in output and "sherpa server user device-password" in output,
                "The refusal does not name the user or the command that fixes it")
        response = self.request(f"/api/v1/admin/users/{username}/device-password",
                                {"username": username, "login_password": self.credentials["admin_password"],
                                 "device_password": device_password, "token": token}, token=token)
        require(response["device_password_set"] is True, "The device password was not set")
        require(self.device_password_state(token, username) is True, "The user record has no device password")
        code, output = self.lab_up_as(home, directory)
        require(DEVICE_PASSWORD_REFUSAL not in output,
                "Labs are still refused for a device password after it was set")
        if code == 0:
            # No image should have let a lab start, but never leave one behind.
            self.command([str(self.base / "bin/sherpa"), "destroy", "--yes"],
                         env={**os.environ, "HOME": str(home)}, cwd=directory, check=False)
        print("PASS upgraded user: labs refused without a device password, accepted once set")

    def installed_hashes(self):
        return {binary + "_binary_sha256": digest(self.base / "bin" / binary)
                for binary in ("sherpa", "sherpad")}

    def reinstall(self):
        self.install()
        self.verify_retained()
        self.command(["systemctl", "start", "sherpad"])

    def uninstall(self, mode):
        """Uninstall in `mode`, and report its case and the resources every mode leaves alone."""
        before = self.resources()
        evidence = self.workspace / ("resources-" + mode.removeprefix("--") + ".toml")
        self.write_resources(evidence, {"before": before})
        self.command(["bash", str(self.workspace / "sherpa_uninstall.sh"), mode, "--force"])
        after = self.resources()
        self.write_resources(evidence, {"before": before, "after": after})
        case = {"--keep-data": "uninstall_script_001", "--remove-data": "uninstall_script_003",
                "--remove-all": "uninstall_script_005"}[mode]
        check_cases([(case, lambda: self.verify_uninstalled(mode)),
                     ("uninstall_script_006", lambda: self.verify_left_alone(before, after))])

    def verify_uninstalled(self, mode):
        """The service and what the install put outside /opt/sherpa are gone, and the data is
        kept or removed as `mode` says."""
        code, _ = self.command(["systemctl", "is-active", "--quiet", "sherpad"], check=False)
        require(code != 0, "Server still running")
        require(not Path("/etc/systemd/system/sherpad.service").exists(), "Service unit remains")
        require(not Path("/etc/logrotate.d/sherpad").exists(), "Logrotate entry remains")
        require(not THP_BOOT_RULE.exists(), "Transparent hugepages boot rule remains")
        binaries = ("sherpa", "sherpad") if mode == "--remove-all" else ("sherpad",)
        for binary in binaries:
            require(not (self.base / "bin" / binary).exists(), f"{binary} binary remains")
            link = Path("/usr/local/bin") / binary
            require(not link.exists() and not link.is_symlink(), f"{binary} symlink remains")
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

    def verify_left_alone(self, before, after):
        """Packages, accounts and groups, Docker's and libvirt's resources are as they were, and
        Docker and libvirt still run."""
        validate_retained_resources(before, after)
        self.check_runtimes()
        self.command(["id", "sherpa"])

    def repeat_uninstall(self):
        """The uninstall on a host where nothing is left exits 0."""
        check_cases([("uninstall_script_007", lambda: self.command(
            ["bash", str(self.workspace / "sherpa_uninstall.sh"), "--remove-all", "--force"]))])

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
            "containers": sorted(query(["docker", "ps", "-a", "--no-trunc", "--format",
                                        "{{.Names}} {{.ID}} {{.Image}} {{.State}}"] ).splitlines()),
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
        print("Collecting server logs, systemd status and boot diagnostics", flush=True)
        commands = (["systemctl", "status", "--no-pager", "sherpad", "docker", "libvirtd"],
                    ["journalctl", "--no-pager", "-u", "sherpad", "-n", "200"],
                    ["journalctl", "--no-pager", "-b", "-p", "warning", "-n", "100"],
                    ["docker", "ps", "-a", "--format", "{{.Names}} {{.Status}}"],
                    ["cloud-init", "status", "--long"],
                    ["df", "-h"])
        for command in commands:
            try:
                code, output = self.command(command, check=False)
                if required and command[0] == "journalctl" and "sherpad" in command:
                    require(code == 0 and output.strip() and output.strip() != "-- No entries --",
                            "Server journal unavailable before uninstall")
            except FileNotFoundError:
                print(f"Diagnostic tool not installed: {command[0]}")
        log = self.base / "logs/sherpad.log"
        if log.is_file():
            self.command(["tail", "-n", "200", str(log)])
        if required:
            print("PASS live server logs captured before uninstall")

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
                       "CARGO_TARGET_DIR": CARGO_TARGET_DIR,
                       "CARGO_TERM_COLOR": "never", "LANG": "C.UTF-8"}
        sccache_config = self.workspace / SCCACHE_CONFIG_FILE
        if not root and sccache_config.is_file():
            # The same for every cargo command, or cargo rebuilds what the test build made.
            # sccache cannot cache incremental compilations.
            environment.update(RUSTC_WRAPPER=str(self.workspace / SCCACHE_BINARY),
                               SCCACHE_CONF=str(sccache_config), CARGO_INCREMENTAL="0")
        return environment

    def as_user(self, command):
        user = self.login_user()
        environment = [f"{key}={value}" for key, value in self.cargo_environment(root=False).items()]
        return ["sudo", "-u", user.pw_name, "-H", "env", "-i", *environment, *command]

    def logged(self, name, command, timeout, env=None, cwd=None):
        logs = self.workspace / "logs"
        logs.mkdir(exist_ok=True)
        code, output = self.command(command, timeout=timeout, env=env, check=False, display=False,
                                    cwd=cwd or self.workspace / "source")
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

    def build_cache(self):
        """Use the host's build cache when the run has one and sccache can reach it.

        sccache refuses to start when it cannot read its storage, and every compilation then
        fails, so the check is starting it: a cache that is down, or a wrong password, gives an
        uncached build instead of a failed one."""
        sccache_config = self.workspace / SCCACHE_CONFIG_FILE
        sccache_config.unlink(missing_ok=True)
        if "build_cache" not in self.config:
            self.record_results("build_cache", {"status": "off"})
            print("Build cache: off")
            return
        cache = self.config["build_cache"]
        binary = self.workspace / SCCACHE_BINARY
        if not binary.is_file():
            archive = self.workspace / "sccache.tar.gz"
            self.command(["curl", "-fsSL", "--proto", "=https", "--tlsv1.2", "-o", str(archive), cache["url"]],
                         timeout=self.timeouts["download"])
            require(digest(archive) == cache["sha256"], "sccache archive checksum mismatch")
            # The archive holds one directory, named as the archive is, with the binary in it.
            member = cache["url"].rsplit("/", 1)[1].removesuffix(".tar.gz") + "/sccache"
            with tarfile.open(archive, "r:gz") as stream:
                binary.parent.mkdir(exist_ok=True)
                binary.write_bytes(stream.extractfile(member).read())
            binary.chmod(0o755)
        sccache_config.touch(mode=0o600)
        sccache_config.write_text(toml_document({"cache.redis": {
            "endpoint": cache["endpoint"], "password": self.credentials["build_cache_password"]}}))
        user = self.login_user()
        os.chown(sccache_config, user.pw_uid, user.pw_gid)
        # A rerun finds the previous run's server still running with its old statistics.
        self.command(self.as_user([str(binary), "--stop-server"]), check=False, display=False)
        code, output = self.command(self.as_user([str(binary), "--start-server"]), check=False, display=False)
        if code:
            sccache_config.unlink()
            self.record_results("build_cache", {"status": "unavailable", "endpoint": cache["endpoint"],
                                                "error": self.redact(output.strip())})
            print(f"Build cache at {cache['endpoint']} unavailable; building uncached:\n{self.redact(output)}")
            return
        print(f"Build cache: {cache['endpoint']}")

    def build_cache_statistics(self):
        if not (self.workspace / SCCACHE_CONFIG_FILE).is_file():
            return
        _, output = self.command(self.as_user([str(self.workspace / SCCACHE_BINARY), "--show-stats",
                                               "--stats-format=json"]), display=False)
        statistics = json.loads(output)["stats"]
        hits = sum(statistics["cache_hits"]["counts"].values())
        misses = sum(statistics["cache_misses"]["counts"].values())
        self.record_results("build_cache", {"status": f"{hits} hits, {misses} misses", "hits": hits,
                                            "misses": misses, "endpoint": self.config["build_cache"]["endpoint"]})

    def build_tests(self):
        # One build for the workspace. Building a crate alone with `-p` resolves its dependencies'
        # features for that crate only, which rebuilds them; the integration tests instead run the
        # executables this build made.
        self.build_cache()
        command = ["cargo", "test", "--workspace", "--locked", "--no-run",
                   "--message-format=json-render-diagnostics"]
        code, output = self.logged("build-workspace", self.as_user(command), self.timeouts["build"])
        self.build_cache_statistics()
        require(code == 0, "build-workspace failed")
        crates = self.config["tests"]["integration_crates"]
        executables = test_executables(output, crates)
        missing = [crate for crate in crates if not executables[crate]["paths"]]
        require(not missing, f"No test executables built for: {', '.join(missing)}")
        (self.workspace / TEST_EXECUTABLES_FILE).write_text(toml_document(executables))
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

    def integration_tests(self):
        tests = self.config["tests"]
        # The tests start their own server; the installed service must not compete for resources.
        self.command(["systemctl", "stop", "sherpad"])
        environment = self.cargo_environment(root=True)
        environment.update(assignment.split("=", 1) for assignment in tests["environment"])
        network = f"sherpa-test-tailnet-{self.config['identity']['nonce'][:8]}"
        tailnet = tailnet_test_environment(self.credentials, network)
        skipped = tests["skip_tests"] if tailnet else [*tests["skip_tests"], *TAILNET_TESTS]
        skips = [argument for name in skipped for argument in ("--skip", name)]
        self.record_results("configuration", {"skip_tests": skipped, "environment": tests["environment"],
                                              "tailnet": "live" if tailnet else "skipped: no tailnet key"})
        if not tailnet:
            print("Live tailnet tests skipped: the run has no tailnet key", flush=True)
        executables = tomllib.loads((self.workspace / TEST_EXECUTABLES_FILE).read_text())
        before = self.inventory()
        if tailnet:
            self.command(["docker", "network", "create", "--subnet", TAILNET_SUBNET, network])
        failed = []
        try:
            for crate in tests["integration_crates"]:
                # Run each executable the way `cargo test` does: from its package directory, and on
                # to the next one when it fails. Only the container tests get the tailnet key.
                code, outputs = 0, []
                crate_executables = executables[crate]
                crate_environment = {**environment, **tailnet} if tailnet and crate == "container" else environment
                for index, (path, directory) in enumerate(zip(crate_executables["paths"],
                                                              crate_executables["directories"])):
                    run_environment = {**crate_environment, "CARGO_MANIFEST_DIR": directory, "CARGO_PKG_NAME": crate}
                    run_code, run_output = self.logged(
                        f"integration-{crate}-{index}",
                        [path, "--ignored", "--test-threads=1", "--show-output", *skips],
                        self.timeouts["tests"], env=run_environment, cwd=directory)
                    code = code or run_code
                    outputs.append(run_output)
                output = "\n".join(outputs)
                summary = summarize_tests(output)
                problems = test_problems(code, summary)
                self.record_results(f"integration-{crate}", {**summary, "status": "failed" if problems else "passed",
                                                              "problems": problems})
                print(f"{'FAIL' if problems else 'PASS'} {crate}: {summary['passed']} passed, "
                      f"{summary['failed']} failed{': ' + '; '.join(problems) if problems else ''}", flush=True)
                if problems:
                    failed.append(crate)
        finally:
            if tailnet:
                self.command(["docker", "network", "rm", network], check=False)
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
                   "install": lambda: self.install(fresh=True), "initialize": self.initialize,
                   "authenticate": self.authenticate, "reboot": self.reboot,
                   "reinstall": self.reinstall, "restore": self.reinstall,
                   "keep-data": lambda: self.uninstall("--keep-data"),
                   "remove-data": lambda: self.uninstall("--remove-data"),
                   "remove-all": lambda: self.uninstall("--remove-all"),
                   "repeat-uninstall": self.repeat_uninstall,
                   "diagnostics": self.diagnostics,
                   "diagnostics-live": lambda: self.diagnostics(required=True),
                   "fault-wait": self.fault_wait, "cancel": self.cancel,
                   "install-baseline": lambda: self.install("baseline"),
                   "seed-upgrade": self.seed_upgrade, "upgrade": self.upgrade,
                   "verify-upgrade": lambda: self.verify_upgrade("after"),
                   "verify-upgrade-reboot": lambda: self.verify_upgrade("reboot"),
                   "device-password": self.device_password,
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
            if step not in UNRECORDED_STEPS:
                (self.workspace / PREVIOUS_STEP_FILE).write_text(step)
        finally:
            active.unlink(missing_ok=True)
            signal.signal(signal.SIGTERM, previous)


class GuestTests(unittest.TestCase):
    def test_each_case_is_reported_by_its_id_and_the_step_names_the_failed_ones(self):
        def failing():
            require(False, "the check failed")
        ran = []
        with patch("builtins.print") as printed, self.assertRaises(RuntimeError) as raised:
            check_cases([("first_001", lambda: ran.append(1)), ("second_002", failing),
                         ("third_003", lambda: ran.append(3))])
        self.assertEqual(ran, [1, 3])
        lines = [call.args[0] for call in printed.call_args_list]
        self.assertEqual(lines, ["PASS first_001", "FAIL second_002: the check failed", "PASS third_003"])
        self.assertEqual(str(raised.exception), "Failed cases: second_002")
        with patch("builtins.print"):
            check_cases([("first_001", lambda: None)])

    def test_authenticate_reports_the_case_of_the_step_before_it_outside_the_upgrade(self):
        self.assertEqual(authenticate_case("initialize", upgrade=False), "server_init_001")
        self.assertEqual(authenticate_case("reboot", upgrade=False), "install_host_008")
        self.assertIsNone(authenticate_case("initialize", upgrade=True))
        self.assertIsNone(authenticate_case("authenticate", upgrade=False))
        self.assertIsNone(authenticate_case(None, upgrade=False))

    def test_environment_fingerprint_ignores_comments_but_detects_changed_settings(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "environment"
            path.write_text("# Generated yesterday\nSHERPA_SERVER_IPV4=127.0.0.1\nSHERPA_SERVER_WS_PORT=3030\n")
            before = environment_fingerprint(path)
            path.write_text("# Generated today\n\nSHERPA_SERVER_WS_PORT=3030\nSHERPA_SERVER_IPV4=127.0.0.1\n")
            self.assertEqual(before, environment_fingerprint(path))
            path.write_text("SHERPA_SERVER_WS_PORT=3031\nSHERPA_SERVER_IPV4=127.0.0.1\n")
            self.assertNotEqual(before, environment_fingerprint(path))

    def test_upgrade_evidence_rejects_missing_phases_and_wrong_installed_bytes(self):
        state = {key: "a" * 64 for key in ("config", "environment", "server_key")}
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

    def test_upgrade_state_requires_unchanged_settings_server_key_and_records(self):
        before = {key: "a" * 64 for key in ("config", "environment", "server_key")}
        before["users"] = ['{"username":"admin","is_admin":true}',
                           '{"username":"persisted","is_admin":false}']
        validate_upgrade_state(before, before)
        for key in before:
            changed = dict(before, **{key: [] if key == "users" else "b" * 64})
            with self.subTest(key=key), self.assertRaises(RuntimeError):
                validate_upgrade_state(before, changed)
        with self.assertRaises(RuntimeError):
            validate_upgrade_state({}, {})

    def retained_guest(self, directory, versions):
        guest = Guest.__new__(Guest)
        guest.workspace = Path(directory) / "workspace"
        guest.base = Path(directory) / "opt"
        guest.workspace.mkdir()
        for relative, _ in RETAINED_FILES.values():
            (guest.base / relative).parent.mkdir(parents=True, exist_ok=True)
            (guest.base / relative).write_text(relative)
        guest.installed_version = lambda: versions.pop(0)
        return guest

    def test_the_database_key_is_kept_from_the_first_release_that_makes_it(self):
        with tempfile.TemporaryDirectory() as directory:
            guest = self.retained_guest(directory, ["0.7.1", "0.7.1"])
            # `sherpad init` makes the config; sherpad's first start the server key and the database key.
            (guest.base / ".certs/server.key").unlink()
            guest.record_retained(("config",))
            (guest.base / ".certs/server.key").write_text("server key")
            guest.record_retained(RETAINED_FILES)
            self.assertEqual(set(tomllib.loads((guest.workspace / RETAINED_FILE).read_text())), set(RETAINED_FILES))
            guest.verify_retained()
            (guest.base / ".secret/db.key").write_text("another key")
            with self.assertRaisesRegex(RuntimeError, "db.key changed"):
                guest.verify_retained()
        with tempfile.TemporaryDirectory() as directory:
            guest = self.retained_guest(directory, ["0.6.3", "0.7.1"])
            (guest.base / ".secret/db.key").unlink()
            guest.record_retained(RETAINED_FILES)
            self.assertNotIn("database_key", tomllib.loads((guest.workspace / RETAINED_FILE).read_text()))
            (guest.base / ".secret/db.key").write_text("made by the update")
            guest.record_retained(RETAINED_FILES)
            self.assertIn("database_key", tomllib.loads((guest.workspace / RETAINED_FILE).read_text()))
            guest.verify_retained()

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

    def test_test_summary_counts_every_binary_and_names_failures(self):
        output = ("running 2 tests\ntest a::ok ... ok\ntest a::bad ... FAILED\n"
                  "test result: FAILED. 1 passed; 1 failed; 3 ignored; 0 measured\n"
                  "test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured\n")
        summary = summarize_tests(output)
        self.assertEqual((summary["binaries"], summary["passed"], summary["failed"], summary["ignored"]),
                         (2, 5, 1, 3))
        self.assertEqual(summary["failures"], ["a::bad"])
        self.assertEqual(test_problems(101, summary), ["cargo exited 101", "1 failed"])

    def test_test_executables_are_grouped_by_integration_crate(self):
        def artifact(package_id, manifest, executable, test=True):
            return json.dumps({"reason": "compiler-artifact", "package_id": package_id,
                               "manifest_path": manifest, "executable": executable,
                               "profile": {"test": test}})
        output = "\n".join([
            "   Compiling db v0.5.12 (/src/crates/db)",
            artifact("path+file:///src/crates/db#0.5.12", "/src/crates/db/Cargo.toml", "/t/db-1"),
            artifact("path+file:///src/crates/db#0.5.12", "/src/crates/db/Cargo.toml", "/t/mod-2"),
            artifact("path+file:///src/crates/server#sherpad@0.5.12", "/src/crates/server/Cargo.toml",
                     "/t/sherpad-3"),
            artifact("path+file:///src/crates/server#sherpad@0.5.12", "/src/crates/server/Cargo.toml",
                     "/t/sherpad-bin", test=False),
            artifact("path+file:///src/crates/shared#0.5.12", "/src/crates/shared/Cargo.toml", "/t/shared-4"),
            artifact("path+file:///src/crates/db#0.5.12", "/src/crates/db/Cargo.toml", None),
            json.dumps({"reason": "build-finished", "success": True}),
        ])
        executables = test_executables(output, ["db", "sherpad", "network"])
        self.assertEqual(executables, {
            "db": {"paths": ["/t/db-1", "/t/mod-2"], "directories": ["/src/crates/db", "/src/crates/db"]},
            "sherpad": {"paths": ["/t/sherpad-3"], "directories": ["/src/crates/server"]},
            "network": {"paths": [], "directories": []},
        })

    def test_missing_results_are_failures_and_skips_are_recorded(self):
        self.assertEqual(test_problems(0, summarize_tests("error: could not compile\n")),
                         ["no test results were reported"])
        optional = summarize_tests("Skipping TAP test on kernel 6.8\n"
                                   "test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured\n")
        self.assertEqual(test_problems(0, optional), [])
        self.assertEqual(optional["skips"], ["Skipping TAP test on kernel 6.8"])

    def test_leftovers_report_only_new_resources(self):
        before = {"containers": ["sherpa-router"], "links": ["lo", "eth0"]}
        after = {"containers": ["sherpa-router", "srl1-tc000001"], "links": ["eth0", "lo"], "domains": ["vm1-x"]}
        self.assertEqual(leftover_resources(before, after),
                         {"containers": ["srl1-tc000001"], "domains": ["vm1-x"]})

    def device_password_guest(self, directory, up_results):
        guest = Guest.__new__(Guest)
        guest.workspace = Path(directory)
        guest.base = Path("/opt/sherpa")
        guest.config = {"identity": {"nonce": "abcdef0123456789"}, "vm": {"version": "24.04-20260926"}}
        guest.credentials = {"admin_password": "login-secret", "device_password": "Dp1!device-secret"}
        guest.settings = {"client_host": "127.0.0.1", "ws_port": 3030}
        guest.timeouts = {"connect": 5, "command": 60}
        guest.passwords = ["login-secret", "Dp1!device-secret"]
        guest.record = {"device_password_set": False}
        posts = []

        def request(path, payload=None, token=None):
            if path == "/api/v1/admin/users":
                raise urllib.error.HTTPError(path, 403, "Forbidden", None, None)
            if path == "/api/v1/admin/users/persist-abcdef01" and payload is None:
                return {"user": {"username": "persist-abcdef01", **guest.record}}
            posts.append((path, payload, token))
            if path.endswith("/device-password"):
                guest.record["device_password_set"] = True
                return {"username": "persist-abcdef01", "device_password_set": True}
            return {"keys": ["k"]}

        commands = []

        def command(arguments, **kwargs):
            commands.append((arguments, kwargs))
            if arguments[0] == "ssh-keygen":
                Path(arguments[-1] + ".pub").write_text("ssh-ed25519 AAAA persist\n")
            if arguments[1:] == ["up"]:
                return up_results.pop(0)
            return 0, ""

        guest.request = request
        guest.command = command
        guest.login = lambda username: {"username": username, "token": "user-token"}
        return guest, posts, commands

    def test_the_update_fails_if_labs_are_not_refused_without_a_device_password(self):
        with tempfile.TemporaryDirectory() as directory:
            guest, _, _ = self.device_password_guest(directory, [(1, "Failed to find a node image")])
            with self.assertRaisesRegex(RuntimeError, "not refused"):
                guest.device_password()

    def test_the_device_password_step_needs_no_admin_route(self):
        refusal = "User 'persist-abcdef01' has no device password, and labs need one: " \
                  "'sherpa server user device-password'"
        with tempfile.TemporaryDirectory() as directory, redirect_stdout(io.StringIO()):
            guest, posts, _ = self.device_password_guest(directory, [(1, refusal), (1, "Failed to find a node image")])
            guest.device_password()
        self.assertEqual([(path, token) for path, _, token in posts],
                         [("/api/v1/admin/users/persist-abcdef01/ssh-keys", "user-token"),
                          ("/api/v1/admin/users/persist-abcdef01/device-password", "user-token")])

    def test_the_update_fails_if_labs_are_still_refused_after_it_is_set(self):
        refusal = "User 'persist-abcdef01' has no device password, and labs need one: " \
                  "'sherpa server user device-password'"
        with tempfile.TemporaryDirectory() as directory:
            guest, _, _ = self.device_password_guest(directory, [(1, refusal), (1, refusal)])
            with self.assertRaisesRegex(RuntimeError, "still refused"):
                guest.device_password()

    def test_every_credential_is_redacted_from_guest_output(self):
        credentials = {"admin_password": "a-login-secret", "device_password": "Dp1!device-secret",
                       "build_cache_password": "cache-secret", "tailnet_auth_key": "tskey-test"}
        with tempfile.TemporaryDirectory() as directory:
            config = Path(directory) / "guest.toml"
            config.write_text(toml_document({"guest": {}, "timeouts": {}, "credentials": credentials}))
            guest = Guest(config)
        self.assertEqual(guest.redact(" ".join(credentials.values())), " ".join(["[REDACTED]"] * 4))

    def cache_guest(self, workspace, build_cache=True):
        guest = Guest.__new__(Guest)
        guest.workspace = workspace
        guest.config = {"tests": {}}
        if build_cache:
            guest.config["build_cache"] = {"endpoint": "redis://192.0.2.10:6379",
                                           "url": "https://example.invalid/sccache-v0-x.tar.gz", "sha256": "0" * 64}
        guest.credentials = {"build_cache_password": "cache-secret"}
        guest.passwords = ["cache-secret"]
        guest.timeouts = {"download": 1}
        return guest

    def test_unusable_build_cache_builds_uncached(self):
        account = pwd.struct_passwd(("tester", "x", os.getuid(), os.getgid(), "", "/home/tester", "/bin/bash"))
        with tempfile.TemporaryDirectory() as directory:
            workspace = Path(directory)
            (workspace / "tools").mkdir()
            (workspace / SCCACHE_BINARY).write_text("binary")
            guest = self.cache_guest(workspace)
            failure = (2, "sccache: error: Server startup failed for cache-secret")
            with patch.dict(os.environ, {"SUDO_USER": "tester"}), \
                 patch.object(pwd, "getpwnam", return_value=account), \
                 patch.object(Guest, "record_results") as record, \
                 patch.object(Guest, "command", side_effect=[(0, ""), failure]):
                guest.build_cache()
            self.assertFalse((workspace / SCCACHE_CONFIG_FILE).exists())
            section, values = record.call_args.args
            self.assertEqual((section, values["status"]), ("build_cache", "unavailable"))
            self.assertNotIn("cache-secret", values["error"])

    def test_binary_version_requires_exact_match(self):
        validate_binary_version("sherpad", "v0.5.35", "sherpad 0.5.35\n")
        for output in ("sherpad 0.5.350", "other 0.5.35"):
            with self.assertRaises(RuntimeError):
                validate_binary_version("sherpad", "v0.5.35", output)

    def test_refuses_host_or_another_vm(self):
        for actual_uuid, actual_hostname in (("other", "test"), ("expected", "production")):
            with self.assertRaises(RuntimeError):
                validate_identity("expected", "test", actual_uuid, actual_hostname)


if __name__ == "__main__":
    if sys.argv[1:] == ["--self-test"]:
        unittest.main(argv=[sys.argv[0]])
    else:
        try:
            Guest(Path(sys.argv[1])).run(sys.argv[2])
        except (Exception, KeyboardInterrupt) as error:
            print(f"FAIL: {error}", file=sys.stderr)
            sys.exit(1)
