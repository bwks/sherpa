#!/usr/bin/env python3
"""Guest-only test-environment checks. Never invoke this helper on the Sherpa host."""

import argparse
import os
import subprocess
import sys
import unittest
import urllib.error
import urllib.request

from vm_release_guest import check_bare_ubuntu, require


# Each endpoint is reachable when it answers with its expected status.
# Registries answer anonymous /v2/ requests with an authentication challenge.
ENDPOINTS = (
    ("Ubuntu archive", "https://archive.ubuntu.com/ubuntu/", 200),
    ("GitHub release API", "https://api.github.com/repos/bwks/sherpa/releases/latest", 200),
    ("Docker installer", "https://get.docker.com/", 200),
    ("Docker Hub", "https://registry-1.docker.io/v2/", 401),
    ("GHCR", "https://ghcr.io/v2/", 401),
)


def run(command, timeout=90, check=True):
    process = subprocess.run(command, stdin=subprocess.DEVNULL, stdout=subprocess.PIPE,
                             stderr=subprocess.STDOUT, text=True, timeout=timeout)
    print(process.stdout, end="", flush=True)
    if check:
        require(process.returncode == 0, f"Command failed ({process.returncode}): {command[0]}")
    return process.returncode, process.stdout


def http_status(url, timeout):
    request = urllib.request.Request(url, method="GET", headers={"User-Agent": "sherpa-test-env"})
    try:
        with urllib.request.urlopen(request, timeout=timeout) as response:
            return response.status
    except urllib.error.HTTPError as error:
        return error.code


def endpoint_status(status, name, url, timeout, attempts):
    for attempt in range(1, attempts + 1):
        try:
            return status(url, timeout)
        except (urllib.error.URLError, TimeoutError) as error:
            print(f"Attempt {attempt}/{attempts}: {name} unreachable: {error}", flush=True)
    raise RuntimeError(f"{name} unreachable after {attempts} attempts: {url}")


def check_network(status=http_status, timeout=20, attempts=3):
    for name, url, expected in ENDPOINTS:
        actual = endpoint_status(status, name, url, timeout, attempts)
        require(actual == expected, f"{name} returned {actual}, expected {expected}: {url}")
        print(f"Reachable: {name} ({actual})")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--version", required=True)
    parser.add_argument("--cpu-count", type=int, required=True)
    parser.add_argument("--boot-disk-size", type=int, required=True)
    parser.add_argument("--initialize-timeout", type=int, default=900)
    args = parser.parse_args()
    require(os.geteuid() == 0, "Guest checks require passwordless sudo")
    vm = {"version": args.version, "cpu_count": args.cpu_count,
          "boot_disk_size": args.boot_disk_size}
    check_bare_ubuntu(run, vm, args.initialize_timeout)
    print("PASS bare Ubuntu baseline, cloud-init and nested KVM")
    check_network()
    print("PASS guest network access")


class GuestTests(unittest.TestCase):
    def test_network_accepts_expected_statuses(self):
        expected = {url: status for _, url, status in ENDPOINTS}
        check_network(lambda url, _timeout: expected[url])

    def test_network_retries_transient_failures_then_names_endpoint(self):
        calls = []

        def flaky(url, _timeout):
            calls.append(url)
            if len(calls) == 1:
                raise TimeoutError("handshake timed out")
            return {url: status for _, url, status in ENDPOINTS}[url]

        check_network(flaky)
        self.assertEqual(calls[0], calls[1])

        def down(_url, _timeout):
            raise urllib.error.URLError("handshake timed out")

        with self.assertRaisesRegex(RuntimeError, "Ubuntu archive unreachable after 3 attempts"):
            check_network(down)

    def test_network_rejects_unexpected_status(self):
        with self.assertRaises(RuntimeError):
            check_network(lambda _url, _timeout: 503)


if __name__ == "__main__":
    if sys.argv[1:] == ["--self-test"]:
        unittest.main(argv=[sys.argv[0]])
    else:
        try:
            main()
        except (Exception, KeyboardInterrupt) as error:
            print(f"FAIL: {error}", file=sys.stderr)
            sys.exit(1)
