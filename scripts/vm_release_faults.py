#!/usr/bin/env python3
"""Exercise harness failures in disposable Sherpa guests, then verify cleanup."""

import argparse
from copy import deepcopy
from contextlib import suppress
from pathlib import Path
import signal
import subprocess
import sys
import time
import tomllib
import unittest
import uuid

from vm_release_test import ROOT, Runner, encode_toml, interrupted, load_config


def validate_failure(report, exit_code, scenario):
    run = report["run"]
    kind = {"fault-timeout": "timeout", "fault-interrupt": "interrupted", "fault-provision": "error"}[scenario]
    if exit_code == 0 or run["status"] != "failed" or run["scenario"] != scenario:
        raise ValueError("Fault check must observe a failed run with a nonzero exit")
    if run.get("failure_kind") != kind or not run.get("guest_uuid") or not run.get("owned_disks"):
        raise ValueError("Failure kind or provisioned-resource evidence is missing")
    if not run.get("lab_id") or not run["retained"]:
        raise ValueError("Fault guest must be retained for verified targeted cleanup")
    if scenario != "fault-provision" and not run.get("command_stopped"):
        raise ValueError("Guest command cancellation was not verified")
    if scenario == "fault-provision" and (not run.get("readiness_timed_out")
            or run.get("completed_steps") or not run.get("error", "").startswith("create-workspace exited 255")):
        raise ValueError("Expected readiness timeout and incomplete guest provisioning were not observed")
    if run.get("cancel_error") or run.get("evidence_error"):
        raise ValueError("Failure evidence or guest cancellation failed")


def run_case(config, directory, scenario):
    case = deepcopy(config)
    output = directory / scenario
    output.mkdir(mode=0o700)
    case["runner"]["output_directory"] = str(output)
    case["runner"]["retain_failed"] = True
    config_path = directory / (scenario + ".toml")
    config_path.write_text(encode_toml(case))
    command = ["bash", str(ROOT / "scripts/test_install.sh"), "--config", str(config_path),
               "--scenario", scenario, "--keep-vm"]
    deadline = time.monotonic() + (case["timeouts"]["provision"] + 2 * case["timeouts"]["download"]
                                  + case["timeouts"]["initialize"] + 3 * case["timeouts"]["command"])
    receipt_path = None
    sent_signal = False
    trigger_deadline = None
    print(f"[fault] {scenario}: provisioning disposable guest", flush=True)
    with (directory / (scenario + ".log")).open("w") as log:
        process = subprocess.Popen(command, stdout=log, stderr=subprocess.STDOUT, start_new_session=True)
        try:
            while process.poll() is None:
                receipts = list(output.glob("*/result.toml"))
                if len(receipts) == 1:
                    receipt_path = receipts[0]
                    report = tomllib.loads(receipt_path.read_text())
                    if scenario == "fault-interrupt" and report["run"].get("active_step") == "fault-wait" and not sent_signal:
                        if trigger_deadline is None:
                            trigger_deadline = time.monotonic() + case["faults"]["trigger_timeout"]
                        observer = Runner.resume(receipt_path.parent)
                        code, _ = observer.ssh("fault-observe", ["test", "-s", observer.remote_directory + "/fault-child.pid"],
                                               case["timeouts"]["command"], check=False)
                        if code == 0:
                            print("[fault] Sending SIGTERM while a verified guest command is running", flush=True)
                            process.send_signal(signal.SIGTERM)
                            sent_signal = True
                        elif time.monotonic() >= trigger_deadline:
                            raise TimeoutError("Guest fault command did not become observable")
                if time.monotonic() >= deadline:
                    raise TimeoutError(f"Fault case did not finish: {scenario}")
                time.sleep(case["timeouts"]["poll"])
            if receipt_path is None:
                receipts = list(output.glob("*/result.toml"))
                if len(receipts) != 1:
                    raise ValueError("Fault case did not produce exactly one run receipt")
                receipt_path = receipts[0]
            report = tomllib.loads(receipt_path.read_text())
            validate_failure(report, process.returncode, scenario)
            if scenario == "fault-interrupt" and not sent_signal:
                raise ValueError("Interruption was not injected")
            run = report["run"]
            print(f"[fault] Observed expected {run['failure_kind']}; lab {run['lab_id']}", flush=True)
            return {"status": "passed", "run_id": run["id"], "lab_id": run["lab_id"],
                    "receipt": str(receipt_path), "exit_code": process.returncode,
                    "observed_failure": run["failure_kind"]}
        finally:
            if process.poll() is None:
                process.send_signal(signal.SIGTERM)
                try:
                    process.wait(timeout=case["timeouts"]["cleanup"] + case["timeouts"]["command"])
                except subprocess.TimeoutExpired:
                    with suppress(ProcessLookupError):
                        process.kill()
                    process.wait()
            if receipt_path is not None:
                cleanup = Runner.resume(receipt_path.parent)
                if cleanup.guest_uuid:
                    if cleanup.remove_retained():
                        raise RuntimeError(f"Targeted cleanup failed: {receipt_path}")
                elif cleanup.report["run"].get("retained"):
                    raise RuntimeError(f"Resource identity incomplete; retain evidence for manual inspection: {receipt_path}")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--config", type=Path, default=ROOT / "dev/release-test/config.toml")
    parser.add_argument("--version")
    parser.add_argument("--artifact-dir", type=Path)
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args()
    if args.self_test:
        unittest.main(argv=[sys.argv[0]])
        return 0
    config = load_config(args.config)
    if args.version:
        config["candidate"]["version"] = args.version
    if args.artifact_dir:
        config["candidate"]["artifact_directory"] = str(args.artifact_dir.resolve())
    directory = (ROOT / config["runner"]["output_directory"] / "failure-checks" / uuid.uuid4().hex).resolve()
    directory.mkdir(parents=True, mode=0o700)
    (directory / ".gitignore").write_text("*\n")
    report = {"suite": {"status": "running", "directory": str(directory)}}
    previous = signal.signal(signal.SIGTERM, interrupted)
    try:
        for scenario in ("fault-timeout", "fault-interrupt", "fault-provision"):
            report[scenario] = run_case(config, directory, scenario)
        report["suite"]["status"] = "passed"
    except (Exception, KeyboardInterrupt) as error:
        report["suite"].update(status="failed", error=str(error))
        print(f"[fail] {error}", flush=True)
    (directory / "fault-results.toml").write_text(encode_toml(report))
    print(f"[result] {report['suite']['status']}: {directory / 'fault-results.toml'}", flush=True)
    signal.signal(signal.SIGTERM, previous)
    return 0 if report["suite"]["status"] == "passed" else 1


class FaultTests(unittest.TestCase):
    def test_partial_provisioning_requires_the_expected_readiness_failure(self):
        report = self.report()
        report["run"].update(scenario="fault-provision", failure_kind="error", completed_steps=[],
                             error="unrelated failure")
        with self.assertRaises(ValueError):
            validate_failure(report, 1, "fault-provision")

    def report(self):
        return {"run": {"status": "failed", "scenario": "fault-timeout", "failure_kind": "timeout",
                        "lab_id": "12345678", "guest_uuid": str(uuid.uuid4()),
                        "owned_disks": ["owned.qcow2"], "retained": True, "command_stopped": True}}

    def test_expected_failure_is_accepted(self):
        validate_failure(self.report(), 1, "fault-timeout")

    def test_success_or_missing_evidence_cannot_pass_a_fault_check(self):
        for key, value in (("status", "passed"), ("failure_kind", "error"), ("guest_uuid", ""),
                           ("owned_disks", []), ("command_stopped", False)):
            report = self.report()
            report["run"][key] = value
            with self.subTest(key=key), self.assertRaises(ValueError):
                validate_failure(report, 1, "fault-timeout")
        with self.assertRaises(ValueError):
            validate_failure(self.report(), 0, "fault-timeout")


if __name__ == "__main__":
    sys.exit(main())
