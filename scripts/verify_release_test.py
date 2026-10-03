#!/usr/bin/env python3
"""Require a complete VM result for the exact scripts and archives being released."""

import argparse
from copy import deepcopy
import io
from pathlib import Path
import tarfile
import tempfile
import tomllib
import unittest
import subprocess
import sys

from vm_release_guest import validate_retained_resources
from vm_release_test import ROOT, STEPS, archive_binary_hash, encode_toml, sha256


def validate_release(receipt, artifacts, version, commit, require_clean=True, sources=ROOT / "scripts"):
    report = tomllib.loads(receipt.read_text())
    run, candidate = report["run"], report["candidate"]
    if run["status"] != "passed" or run["scenario"] != "lifecycle":
        raise ValueError("Release requires a passed complete lifecycle run")
    if tuple(run["completed_steps"]) != STEPS or run.get("diagnostics_before") != ["keep-data", "remove-data", "remove-all"]:
        raise ValueError("Required scenarios or pre-uninstall diagnostics are missing")
    if run["retained"] or run.get("resource_state") != "removed" or not run.get("cleanup_verified"):
        raise ValueError("Disposable lab cleanup was not verified")
    if any(run.get(key) for key in ("error", "failure_kind", "cleanup_error", "evidence_error", "cancel_error")):
        raise ValueError("Failed or timed-out evidence cannot approve a release")
    if candidate["version"] != version or candidate["commit"] != commit:
        raise ValueError("Receipt refers to a different candidate version or commit")
    if not candidate["artifact_directory"]:
        raise ValueError("Published-release baselines cannot approve an unpublished candidate")
    if require_clean and candidate["dirty"]:
        raise ValueError("Release evidence must come from a clean checkout")
    if report["vm"]["version"] != "26.04":
        raise ValueError("Required Ubuntu 26.04 run is missing")
    for name in ("sherpa_install.sh", "sherpa_uninstall.sh", "vm_release_test.py", "vm_release_guest.py"):
        if sha256(sources / name) != report["scripts"][name.replace(".", "_")]:
            raise ValueError(f"{name} changed after testing; rerun the VM suite")
    for binary in ("sherpa", "sherpad"):
        archive = artifacts / f"{binary}-{candidate['target']}.tar.gz"
        if sha256(archive) != report["artifacts"][binary + "_archive_sha256"]:
            raise ValueError(f"{binary} archive changed after testing")
        if archive_binary_hash(archive, binary) != report["artifacts"][binary + "_binary_sha256"]:
            raise ValueError(f"{binary} binary differs from the installed candidate")
    categories = {"packages", "users", "groups", "images", "volumes", "containers", "networks", "pools", "domains"}
    for mode in ("keep-data", "remove-data", "remove-all"):
        inventory = tomllib.loads((receipt.parent / f"resources-{mode}.toml").read_text())
        if set(inventory["before"]) != categories or any(not inventory["before"][key]
                for key in ("packages", "users", "groups", "images", "networks", "pools")):
            raise ValueError(f"Retained-resource evidence is incomplete for {mode}")
        try:
            validate_retained_resources(inventory["before"], inventory["after"])
        except RuntimeError as error:
            raise ValueError(f"Retained-resource verification failed for {mode}: {error}") from error
    return report


def stage_evidence(report, receipt, directory):
    directory.mkdir(parents=True, exist_ok=True)
    public = {key: deepcopy(report[key]) for key in ("run", "candidate", "image", "vm", "scripts", "artifacts")}
    for key in ("directory", "jump_host", "owned_disks"):
        public["run"].pop(key, None)
    public["candidate"]["artifact_directory"] = "supplied-local-archives"
    public["image"].pop("path", None)
    (directory / "result.toml").write_text(encode_toml(public))
    for mode in ("keep-data", "remove-data", "remove-all"):
        name = f"resources-{mode}.toml"
        (directory / name).write_bytes((receipt.parent / name).read_bytes())


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--self-test", action="store_true")
    inputs = parser.add_mutually_exclusive_group()
    inputs.add_argument("--receipt", type=Path)
    inputs.add_argument("--result-file", type=Path, help="TOML reference written by scripts/test_install.sh")
    inputs.add_argument("--receipt-root", type=Path, help="Directory containing exactly one lifecycle result")
    parser.add_argument("--artifact-dir", type=Path)
    parser.add_argument("--version")
    parser.add_argument("--commit", help="Expected source commit; defaults to this checkout's HEAD")
    cleanliness = parser.add_mutually_exclusive_group()
    cleanliness.add_argument("--require-clean", action="store_true", help="Require a clean checkout (the default)")
    cleanliness.add_argument("--allow-dirty", action="store_true", help="Verify local development artifacts without requiring a clean source checkout")
    parser.add_argument("--stage-evidence", type=Path, help="Copy only nonsecret release evidence for CI upload")
    args = parser.parse_args()
    if args.self_test:
        unittest.main(argv=[sys.argv[0]])
        return 0
    if not (args.receipt or args.receipt_root or args.result_file) or not args.artifact_dir or not args.version:
        parser.error("--receipt or --receipt-root, --artifact-dir and --version are required")
    try:
        receipt = args.receipt
        if args.result_file:
            reference = tomllib.loads(args.result_file.read_text())["result"]
            if reference["status"] != "passed":
                raise ValueError("Referenced run did not pass")
            receipt = Path(reference["receipt"])
            if tomllib.loads(receipt.read_text())["run"]["id"] != reference["run_id"]:
                raise ValueError("Result reference points to a different run")
        if args.receipt_root:
            receipts = [path for path in args.receipt_root.rglob("result.toml")
                        if tomllib.loads(path.read_text())["run"].get("scenario") == "lifecycle"]
            if len(receipts) != 1:
                raise ValueError("Specify an exact receipt: expected exactly one lifecycle result")
            receipt = receipts[0]
        commit = args.commit or subprocess.check_output(["git", "-C", str(ROOT), "rev-parse", "HEAD"], text=True).strip()
        report = validate_release(receipt, args.artifact_dir, args.version, commit, not args.allow_dirty)
        if args.stage_evidence:
            stage_evidence(report, receipt, args.stage_evidence)
        mode = "local verification (dirty checkout allowed)" if args.allow_dirty else "release gate"
        print(f"PASS {mode}: {args.version}, commit {commit}, exact tested scripts and archives")
        return 0
    except (ValueError, OSError, KeyError) as error:
        print(f"FAIL release gate: {error}", file=sys.stderr)
        return 1


class GateTests(unittest.TestCase):
    def test_staged_evidence_excludes_credentials_and_connection_settings(self):
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            receipt, report, sources = self.fixture(directory)
            report.update(host={"server_url": "private-server"}, credentials={"password": "private-password"},
                          image={"build": "test", "sha256": "b" * 64, "path": "private-path"})
            report["run"].update(directory="private-directory", jump_host="private-server")
            stage_evidence(report, receipt, directory / "public")
            exported = tomllib.loads((directory / "public/result.toml").read_text())
            self.assertNotIn("credentials", exported)
            self.assertNotIn("host", exported)
            self.assertNotIn("directory", exported["run"])
            self.assertNotIn("jump_host", exported["run"])
            validate_release(directory / "public/result.toml", directory, "v0.3.80", "a" * 40, sources=sources)

    def fixture(self, directory):
        sources = directory / "scripts"
        sources.mkdir()
        report = {"run": {"status": "passed", "scenario": "lifecycle", "retained": False,
                          "resource_state": "removed", "cleanup_verified": True,
                          "completed_steps": list(STEPS), "diagnostics_before": ["keep-data", "remove-data", "remove-all"]},
                  "candidate": {"version": "v0.3.80", "target": "x86_64-unknown-linux-gnu",
                                "commit": "a" * 40, "dirty": False, "artifact_directory": str(directory)},
                  "vm": {"version": "26.04"}, "scripts": {}, "artifacts": {}}
        for name in ("sherpa_install.sh", "sherpa_uninstall.sh", "vm_release_test.py", "vm_release_guest.py"):
            (sources / name).write_text(name)
            report["scripts"][name.replace(".", "_")] = sha256(sources / name)
        for binary in ("sherpa", "sherpad"):
            archive = directory / f"{binary}-x86_64-unknown-linux-gnu.tar.gz"
            with tarfile.open(archive, "w:gz") as stream:
                member = tarfile.TarInfo(binary)
                member.size = 6
                stream.addfile(member, io.BytesIO(b"binary"))
            report["artifacts"][binary + "_archive_sha256"] = sha256(archive)
            report["artifacts"][binary + "_binary_sha256"] = archive_binary_hash(archive, binary)
        inventory = {key: ["retained"] for key in ("packages", "users", "groups", "images", "networks", "pools", "domains", "containers", "volumes")}
        for mode in ("keep-data", "remove-data", "remove-all"):
            (directory / f"resources-{mode}.toml").write_text(encode_toml({"before": inventory, "after": inventory}))
        receipt = directory / "result.toml"
        receipt.write_text(encode_toml(report))
        return receipt, report, sources

    def test_complete_result_accepts_only_identical_archives_and_scripts(self):
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            receipt, report, sources = self.fixture(directory)
            validate_release(receipt, directory, "v0.3.80", "a" * 40, sources=sources)
            (sources / "sherpa_install.sh").write_text("changed since testing")
            with self.assertRaises(ValueError):
                validate_release(receipt, directory, "v0.3.80", "a" * 40, sources=sources)
            (sources / "sherpa_install.sh").write_text("sherpa_install.sh")
            (directory / "sherpad-x86_64-unknown-linux-gnu.tar.gz").write_bytes(b"changed archive")
            with self.assertRaises(ValueError):
                validate_release(receipt, directory, "v0.3.80", "a" * 40, sources=sources)

    def test_failed_timed_out_skipped_or_dirty_results_block_release(self):
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            receipt, report, sources = self.fixture(directory)
            for key, value in (("status", "failed"), ("scenario", "preflight"),
                               ("completed_steps", list(STEPS[:-1])), ("diagnostics_before", []),
                               ("cleanup_verified", False), ("retained", True), ("failure_kind", "timeout")):
                candidate = {**report, "run": {**report["run"], key: value}}
                receipt.write_text(encode_toml(candidate))
                with self.subTest(key=key), self.assertRaises(ValueError):
                    validate_release(receipt, directory, "v0.3.80", "a" * 40, sources=sources)
            report["candidate"]["dirty"] = True
            receipt.write_text(encode_toml(report))
            with self.assertRaises(ValueError):
                validate_release(receipt, directory, "v0.3.80", "a" * 40, sources=sources)


if __name__ == "__main__":
    sys.exit(main())
