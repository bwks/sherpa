# Installer and uninstaller testing

Run the real scripts in fresh Ubuntu VMs managed by the existing Sherpa server:

```sh
./scripts/test_install.sh
```

The runner uses [TOML settings](../dev/release-test/config.toml) and the
[VM manifest](../dev/release-test/manifest.toml). See the
[developer guide](../dev/release-test/README.md) for prerequisites, candidate
artifacts, evidence and inspecting a failed run.

The old host-based test script has been replaced. The current runner does not stop
host containers or install server dependencies on the machine invoking it.

Run harness regression tests without provisioning a VM:

```sh
./scripts/test_install.sh --self-test
python3 scripts/vm_release_guest.py --self-test
python3 scripts/vm_release_faults.py --self-test
python3 scripts/verify_release_test.py --self-test
```

Run `cargo fmt` and `cargo clippy --workspace -- -D warnings` before tests, as
required by `AGENTS.md`.

Exercise real timeout, interruption and incomplete provisioning in disposable VMs:

```sh
bash scripts/test_install.sh --scenario failure-checks
```

VM tests run locally against your existing Sherpa server using the authenticated
CLI and SSH access. The existing GitHub workflows do not enforce these results;
publication enforcement remains an open task in the
[release checklist](../test-specs/integration/vm-release-test-plan.md).

The [BATS suite](../test-scripts/install_tests.bats) provides additional installer
unit tests. Its integration checks can skip when prerequisites are absent, so its
result alone is not the VM release gate.
