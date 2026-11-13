#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.15"
# dependencies = []
# ///
"""Bring up, check and destroy Sherpa test environments (Python 3.15+, standard library only)."""

import argparse
import base64
from contextlib import redirect_stdout
import io
import json
import os
from pathlib import Path
import queue
import shlex
import shutil
import subprocess
import sys
import tarfile
import tempfile
import threading
import time
import tomllib
import unittest
import uuid

import build_cache
from host_memory import MEMORY_REFUSED_EXIT
from vm_release_test import encode_toml, guest_python, guest_uv_setup, sha256


HARNESS = Path(__file__).resolve().parent
ROOT = HARNESS.parent.parent
ENVIRONMENTS = ROOT / "tests" / "environments"
GUEST_SCRIPTS = ("vm_release_guest.py", "test_env_guest.py")
GUEST_DIRECTORY = "/tmp/sherpa-test-env"
CONNECT_TIMEOUT = 10
RUNS = ROOT / ".tmp" / "test-env-runs"
# The first lines of every Python script in tests/ and dev/ that has a #! line.
SCRIPT_HEADER = ('#!/usr/bin/env -S uv run --script\n# /// script\n# requires-python = ">=3.15"\n'
                 '# dependencies = []\n# ///\n')
TARGET = "x86_64-unknown-linux-gnu"
# Release binaries are built per package, as the release workflow does, so features are not unified.
RELEASE_PACKAGES = ("sherpad", "sherpa")
CLI_TOKEN = Path.home() / ".sherpa" / "token"
# A full run takes over an hour, and the CLI token must stay valid until cleanup.
MINIMUM_TOKEN_LIFETIME = 4 * 3600


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
    require("version" in node, "Test environment node must set version explicitly")
    for group, key in (("cpu", "count"), ("boot_disk", "size")):
        require(key in node.get(group, {}), f"Test environment node must set {group}.{key} explicitly")
    return node


def lab_id(directory, manifest):
    path = directory / ".sherpa" / "lab-info.toml"
    require(path.is_file(), f"No lab is up for {directory.name}; run: test_env.py up {directory.name}")
    info = tomllib.loads(path.read_text())
    require(info.get("name") == manifest["name"],
            f".sherpa/lab-info.toml belongs to lab {info.get('name')!r}, not {manifest['name']!r}")
    return info["id"]


def run(command, directory):
    print("+ " + shlex.join(command), flush=True)
    process = subprocess.run(command, cwd=directory, stdin=subprocess.DEVNULL)
    require(process.returncode == 0, f"Command failed ({process.returncode}): {shlex.join(command)}")


def ssh_options(directory):
    return ["-F", str(directory / ".sherpa" / "ssh" / "config"), "-o", "BatchMode=yes",
            "-o", f"ConnectTimeout={CONNECT_TIMEOUT}"]


def guest_command(node):
    return ["sudo", "-n", *guest_python(f"{GUEST_DIRECTORY}/test_env_guest.py",
                                        "--version", node["version"], "--cpu-count", str(node["cpu"]["count"]),
                                        "--boot-disk-size", str(node["boot_disk"]["size"]))]


def check(directory):
    manifest = tomllib.loads((directory / "manifest.toml").read_text())
    node = environment_node(manifest)
    target = f"{node['name']}.{lab_id(directory, manifest)}"
    options = ssh_options(directory)
    run(["ssh", *options, target, shlex.join(["mkdir", "-p", GUEST_DIRECTORY])], directory)
    run(["scp", "-q", *options, *(str(HARNESS / script) for script in GUEST_SCRIPTS),
         f"{target}:{GUEST_DIRECTORY}/"], directory)
    run(["ssh", *options, target, shlex.join(guest_uv_setup())], directory)
    run(["ssh", *options, target, shlex.join(guest_command(node))], directory)
    print(f"PASS test environment {directory.name} ({target})")


def up(directory):
    environment_node(tomllib.loads((directory / "manifest.toml").read_text()))
    run(["sherpa", "up"], directory)
    check(directory)


def destroy(directory):
    lab_id(directory, tomllib.loads((directory / "manifest.toml").read_text()))
    run(["sherpa", "destroy", "--yes"], directory)


def token_seconds_remaining(token, now):
    """Seconds until a JWT expires, from its payload's exp claim."""
    parts = token.strip().split(".")
    require(len(parts) == 3, "The Sherpa CLI token is not a JWT")
    payload = parts[1] + "=" * (-len(parts[1]) % 4)
    claims = json.loads(base64.urlsafe_b64decode(payload))
    require(isinstance(claims.get("exp"), int), "The Sherpa CLI token has no expiry")
    return claims["exp"] - now


def require_cli_login(path=CLI_TOKEN, now=None):
    require(path.is_file(), "Not logged in: run sherpa login")
    remaining = token_seconds_remaining(path.read_text(), now if now is not None else int(time.time()))
    require(remaining >= MINIMUM_TOKEN_LIFETIME,
            f"The Sherpa CLI token expires in {max(remaining, 0) // 60} minutes; run sherpa login first")


def snapshot_files(root):
    output = subprocess.run(["git", "-C", str(root), "ls-files", "-z", "--cached", "--others", "--exclude-standard"],
                            check=True, capture_output=True).stdout
    names = {name for name in output.decode().split("\0") if name}
    # Tracked files deleted in the working tree are not part of the snapshot.
    return sorted(name for name in names if (root / name).is_file() or (root / name).is_symlink())


def copy_snapshot(root, destination):
    for name in snapshot_files(root):
        target = destination / name
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(root / name, target, follow_symlinks=False)


def write_archive(source, path):
    with tarfile.open(path, "w:gz") as archive:
        for item in sorted(source.rglob("*")):
            if item.is_file() or item.is_symlink():
                archive.add(item, arcname=str(item.relative_to(source)), recursive=False)


def git_state(root):
    commit = subprocess.run(["git", "-C", str(root), "rev-parse", "HEAD"],
                            check=True, capture_output=True, text=True).stdout.strip()
    dirty = bool(subprocess.run(["git", "-C", str(root), "status", "--porcelain"],
                                check=True, capture_output=True, text=True).stdout.strip())
    return commit, dirty


def build_logged(name, command, directory, cwd, environment):
    log = directory / f"{name}.log"
    print(f"[build] {name}: {shlex.join(command)}", flush=True)
    with log.open("w") as stream:
        process = subprocess.run(command, cwd=cwd, env=environment, stdin=subprocess.DEVNULL,
                                 stdout=stream, stderr=subprocess.STDOUT)
    require(process.returncode == 0, f"{name} failed; see {log}")


def prepare_source(directory):
    """Snapshot the checkout (tracked and untracked, non-ignored files) for the guest."""
    source = directory / "source"
    copy_snapshot(ROOT, source)
    commit, dirty = git_state(ROOT)
    return source, commit, dirty


def prepare_candidate(directory):
    """Build release archives from the checkout snapshot and package the same source for the guest."""
    source, commit, dirty = prepare_source(directory)
    tailwind = ROOT / "tailwindcss"
    if tailwind.is_file():
        shutil.copy2(tailwind, source / "tailwindcss")
    environment = dict(os.environ, CARGO_TARGET_DIR=str(ROOT / "target" / "test-env"), CARGO_TERM_COLOR="never")
    build_logged("build-tailwind", ["bash", "scripts/tailwind-build.sh"], directory, source, environment)
    (source / "tailwindcss").unlink(missing_ok=True)
    version = "v" + tomllib.loads((source / "crates/server/Cargo.toml").read_text())["package"]["version"]
    artifacts = directory / "artifacts"
    artifacts.mkdir()
    for package in RELEASE_PACKAGES:
        build_logged(f"build-{package}", ["cargo", "build", "--release", "--locked", "-p", package,
                                          "--target", TARGET], directory, source, environment)
        with tarfile.open(artifacts / f"{package}-{TARGET}.tar.gz", "w:gz") as archive:
            archive.add(ROOT / "target" / "test-env" / TARGET / "release" / package, arcname=package)
    archive = directory / "source.tar.gz"
    write_archive(source, archive)
    candidate = {"version": version, "commit": commit, "dirty": dirty,
                 "source_archive_sha256": sha256(archive),
                 **{f"{package}_archive_sha256": sha256(artifacts / f"{package}-{TARGET}.tar.gz")
                    for package in RELEASE_PACKAGES}}
    (directory / "candidate.toml").write_text(encode_toml({"candidate": candidate}))
    print(f"[build] Candidate {version} from {commit[:12]}{' (uncommitted changes)' if dirty else ''}", flush=True)
    return candidate, artifacts, archive


def relay(name, stream, log):
    # A release run again after a memory refusal appends to its first attempt's log.
    with log.open("a") as output:
        for line in stream:
            output.write(line)
            output.flush()
            print(f"[{name}] {line}", end="", flush=True)


def runner_command(environment, candidate, artifacts, archive, directory):
    return [sys.executable, str(HARNESS / "vm_release_test.py"), "--scenario", "code",
            "--config", str(environment_directory(environment) / "release-test.toml"),
            "--version", candidate["version"], "--artifact-dir", str(artifacts),
            "--source-archive", str(archive), "--result-file", str(directory / f"{environment}.toml")]


def start_release(name, command, directory, finished):
    """Start one release's runner; its exit code goes to finished once its output is relayed."""
    process = subprocess.Popen(command, cwd=ROOT, stdin=subprocess.DEVNULL, stdout=subprocess.PIPE,
                               stderr=subprocess.STDOUT, text=True)

    def watch():
        with process.stdout as stream:
            relay(name, stream, directory / f"{name}.log")
        finished.put((name, process.wait()))

    threading.Thread(target=watch).start()


def run_releases(commands, start):
    """Run the releases together. One the server refuses for memory while another runs is run
    again on its own once nothing else runs; refused when it ran alone, it fails.

    start(name, command, finished) starts a release and puts (name, exit code) on finished.
    """
    finished = queue.Queue()
    running = set()
    deferred = []
    codes = {}
    for name, command in commands.items():
        start(name, command, finished)
        running.add(name)
    alone = set(running) if len(running) == 1 else set()
    while running:
        name, code = finished.get()
        running.discard(name)
        if code == MEMORY_REFUSED_EXIT and name not in alone:
            print(f"[run] {name}: the server refused it for memory; it runs alone once the others finish",
                  flush=True)
            deferred.append(name)
        else:
            codes[name] = code
        if not running and deferred:
            name = deferred.pop(0)
            print(f"[run] {name}: running it alone", flush=True)
            alone.add(name)
            start(name, commands[name], finished)
            running.add(name)
    return {name: codes[name] for name in commands}


def summarize(directory, codes):
    print("\n=== Test environment results ===", flush=True)
    for name, code in codes.items():
        reference = directory / f"{name}.toml"
        if not reference.is_file():
            print(f"{name}: failed before a result was recorded (exit {code})")
            continue
        receipt_path = Path(tomllib.loads(reference.read_text())["result"]["receipt"])
        receipt = tomllib.loads(receipt_path.read_text())
        run = receipt["run"]
        retained = " (VM retained for debugging)" if run.get("retained") else ""
        print(f"{name}: {run['status']}{retained} - {receipt_path}")
        for key, value in receipt.get("tests", {}).items():
            if key != "leftovers":
                print(f"  {key.removesuffix('_status')}: {value}")
        for leftover in receipt.get("tests", {}).get("leftovers", []):
            print(f"  leftover {leftover}")
        if run.get("error"):
            print(f"  error: {run['error']}")


def run_environments(environments):
    for environment in environments:
        require((environment_directory(environment) / "release-test.toml").is_file(),
                f"{environment} has no release-test.toml")
    require_cli_login()
    directory = RUNS / uuid.uuid4().hex
    directory.mkdir(parents=True, mode=0o700)
    print(f"[run] {directory}", flush=True)
    candidate, artifacts, archive = prepare_candidate(directory)
    commands = {environment: runner_command(environment, candidate, artifacts, archive, directory)
                for environment in environments}
    codes = run_releases(commands, lambda name, command, finished:
                         start_release(name, command, directory, finished))
    summarize(directory, codes)
    return 0 if not any(codes.values()) else 1


def rerun(result_directory):
    """Retest a retained failed run with the current checkout. Passing destroys its VM."""
    require_cli_login()
    with tempfile.TemporaryDirectory(dir=RUNS.parent) as temporary:
        directory = Path(temporary)
        source, commit, dirty = prepare_source(directory)
        archive = directory / "source.tar.gz"
        write_archive(source, archive)
        print(f"[rerun] Source from {commit[:12]}{' (uncommitted changes)' if dirty else ''}", flush=True)
        process = subprocess.run([sys.executable, str(HARNESS / "vm_release_test.py"),
                                  "--rerun-run", str(result_directory), "--source-archive", str(archive)],
                                 cwd=ROOT, stdin=subprocess.DEVNULL)
    return process.returncode


def cache(action):
    settings = build_cache.load_settings()
    if action == "start":
        build_cache.start(settings, build_cache.server_address())
    elif action == "stop":
        build_cache.stop(settings)
    elif action == "clear":
        build_cache.clear(settings)
    else:
        return build_cache.status(settings, build_cache.server_address())
    return 0


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="action", required=True)
    for action in ("up", "check", "destroy"):
        command = commands.add_parser(action, help=f"{action} one environment by hand")
        command.add_argument("environment", help="Directory name under tests/environments/")
    command = commands.add_parser("run", help="Build the checkout and test it in fresh environments")
    command.add_argument("environments", nargs="*", default=["ubuntu-2404", "ubuntu-2604"])
    command = commands.add_parser("rerun", help="Retest a retained failed run with the current checkout")
    command.add_argument("result_directory", type=Path, help="The failed run's .tmp/vm-release-tests/<id> directory")
    command = commands.add_parser("cache", help="Manage the VM code suite's build cache on this host")
    command.add_argument("cache_action", choices=("start", "stop", "status", "clear"))
    args = parser.parse_args()
    if args.action == "run":
        return run_environments(args.environments)
    if args.action == "rerun":
        return rerun(args.result_directory.resolve())
    if args.action == "cache":
        return cache(args.cache_action)
    actions = {"up": up, "check": check, "destroy": destroy}
    actions[args.action](environment_directory(args.environment))
    return 0


def unsigned_token(exp):
    """A JWT-shaped test token with only an expiry claim."""
    payload = base64.urlsafe_b64encode(json.dumps({"exp": exp}).encode()).decode().rstrip("=")
    return f"header.{payload}.signature"


class HostTests(unittest.TestCase):
    def test_node_requires_single_pinned_ubuntu_node(self):
        node = {"name": "testbox", "model": "ubuntu_linux", "version": "26.04",
                "cpu": {"count": 8}, "boot_disk": {"size": 100}}
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

    def test_snapshot_includes_untracked_and_skips_ignored_and_deleted_files(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            subprocess.run(["git", "init", "-q", str(root)], check=True)
            (root / ".gitignore").write_text("target/\n")
            (root / "tracked.rs").write_text("tracked")
            (root / "deleted.rs").write_text("deleted")
            subprocess.run(["git", "-C", str(root), "add", "."], check=True)
            (root / "deleted.rs").unlink()
            (root / "new.rs").write_text("new")
            (root / "target").mkdir()
            (root / "target" / "build.o").write_text("ignored")
            self.assertEqual(snapshot_files(root), [".gitignore", "new.rs", "tracked.rs"])
            copy_snapshot(root, root / "copy")
            write_archive(root / "copy", root / "source.tar.gz")
            with tarfile.open(root / "source.tar.gz") as archive:
                self.assertEqual(sorted(archive.getnames()), [".gitignore", "new.rs", "tracked.rs"])

    def test_cli_login_must_outlast_a_run(self):
        self.assertEqual(token_seconds_remaining(unsigned_token(1_000_600), 1_000_000), 600)
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "token"
            path.write_text(unsigned_token(1_000_000 + MINIMUM_TOKEN_LIFETIME))
            require_cli_login(path, now=1_000_000)
            path.write_text(unsigned_token(1_000_000 + 60))
            with self.assertRaisesRegex(RuntimeError, "expires in 1 minutes"):
                require_cli_login(path, now=1_000_000)
            with self.assertRaisesRegex(RuntimeError, "Not logged in"):
                require_cli_login(Path(directory) / "missing")

    def test_runner_command_selects_code_scenario_and_candidate_inputs(self):
        command = runner_command("ubuntu-2604", {"version": "v0.3.88"}, Path("/run/artifacts"),
                                 Path("/run/source.tar.gz"), Path("/run"))
        self.assertEqual(command[command.index("--scenario") + 1], "code")
        self.assertEqual(command[command.index("--version") + 1], "v0.3.88")
        self.assertEqual(command[command.index("--artifact-dir") + 1], "/run/artifacts")
        self.assertEqual(command[command.index("--source-archive") + 1], "/run/source.tar.gz")
        self.assertTrue(command[command.index("--config") + 1].endswith("ubuntu-2604/release-test.toml"))

    def schedule(self, names, completions):
        """Run run_releases with releases that finish in the given (name, exit code) order.

        A completion is delivered once its release has been started and not yet finished.
        Returns the codes and, for each start, the name and whether every earlier result had been
        taken from the queue by then.
        """
        completions = list(completions)
        active = set()
        starts = []

        def start(name, command, finished):
            starts.append((name, finished.empty()))
            active.add(name)
            while completions and completions[0][0] in active:
                done, code = completions.pop(0)
                active.discard(done)
                finished.put((done, code))

        with redirect_stdout(io.StringIO()):
            codes = run_releases({name: [name] for name in names}, start)
        self.assertEqual(completions, [])
        return codes, starts

    def test_a_release_refused_for_memory_runs_alone_after_the_others(self):
        codes, starts = self.schedule(["ubuntu-2404", "ubuntu-2604"],
                                      [("ubuntu-2604", MEMORY_REFUSED_EXIT), ("ubuntu-2404", 0),
                                       ("ubuntu-2604", 0)])
        self.assertEqual(codes, {"ubuntu-2404": 0, "ubuntu-2604": 0})
        self.assertEqual(starts, [("ubuntu-2404", True), ("ubuntu-2604", True), ("ubuntu-2604", True)])

    def test_a_release_refused_alone_fails(self):
        codes, starts = self.schedule(["ubuntu-2604"], [("ubuntu-2604", MEMORY_REFUSED_EXIT)])
        self.assertEqual(codes, {"ubuntu-2604": MEMORY_REFUSED_EXIT})
        self.assertEqual([name for name, _ in starts], ["ubuntu-2604"])
        codes, starts = self.schedule(["ubuntu-2404", "ubuntu-2604"],
                                      [("ubuntu-2604", MEMORY_REFUSED_EXIT), ("ubuntu-2404", 1),
                                       ("ubuntu-2604", MEMORY_REFUSED_EXIT)])
        self.assertEqual(codes, {"ubuntu-2404": 1, "ubuntu-2604": MEMORY_REFUSED_EXIT})
        self.assertEqual([name for name, _ in starts], ["ubuntu-2404", "ubuntu-2604", "ubuntu-2604"])

    def test_releases_all_refused_run_one_at_a_time(self):
        codes, starts = self.schedule(["ubuntu-2404", "ubuntu-2604"],
                                      [("ubuntu-2404", MEMORY_REFUSED_EXIT), ("ubuntu-2604", MEMORY_REFUSED_EXIT),
                                       ("ubuntu-2404", 0), ("ubuntu-2604", 0)])
        self.assertEqual(codes, {"ubuntu-2404": 0, "ubuntu-2604": 0})
        self.assertEqual([name for name, _ in starts], ["ubuntu-2404", "ubuntu-2604", "ubuntu-2404", "ubuntu-2604"])

    def test_other_failures_are_not_run_again(self):
        codes, starts = self.schedule(["ubuntu-2404", "ubuntu-2604"], [("ubuntu-2604", 1), ("ubuntu-2404", 0)])
        self.assertEqual(codes, {"ubuntu-2404": 0, "ubuntu-2604": 1})
        self.assertEqual(len(starts), 2)

    def test_release_processes_report_their_exit_codes(self):
        with tempfile.TemporaryDirectory() as directory:
            commands = {"first": [sys.executable, "-c", "print('one')"],
                        "second": [sys.executable, "-c", f"import sys; print('two'); sys.exit({MEMORY_REFUSED_EXIT})"]}
            with redirect_stdout(io.StringIO()) as output:
                codes = run_releases(commands, lambda name, command, finished:
                                     start_release(name, command, Path(directory), finished))
            self.assertEqual(codes, {"first": 0, "second": MEMORY_REFUSED_EXIT})
            self.assertEqual((Path(directory) / "second.log").read_text(), "two\ntwo\n")
            self.assertIn("[run] second: running it alone", output.getvalue())

    def test_guest_command_passes_manifest_expectations(self):
        node = {"version": "24.04-20260926", "cpu": {"count": 8}, "boot_disk": {"size": 100}}
        command = guest_command(node)
        self.assertEqual(command[-6:],
                         ["--version", "24.04-20260926", "--cpu-count", "8", "--boot-disk-size", "100"])
        self.assertNotIn("python3", command)
        self.assertEqual(command[command.index("--python") + 1], "3.15")

    def test_scripts_run_directly_use_uv_python(self):
        """A script with a #! line runs on uv's Python 3.15, never on the system's python3."""
        scripts = [path for directory in ("tests", "dev") for path in (ROOT / directory).rglob("*.py")]
        self.assertTrue(scripts)
        for path in scripts:
            text = path.read_text()
            if not text.startswith("#!"):
                continue
            with self.subTest(script=str(path.relative_to(ROOT))):
                self.assertTrue(text.startswith(SCRIPT_HEADER))


if __name__ == "__main__":
    if sys.argv[1:] == ["--self-test"]:
        unittest.main(argv=[sys.argv[0]])
    else:
        try:
            sys.exit(main())
        except (Exception, KeyboardInterrupt) as error:
            print(f"FAIL: {error}", file=sys.stderr)
            sys.exit(1)
