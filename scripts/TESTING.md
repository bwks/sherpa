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
```

Run `cargo fmt` and `cargo clippy --workspace -- -D warnings` before tests, as
required by `AGENTS.md`.

The [BATS suite](../test-scripts/install_tests.bats) provides additional installer
unit tests. Its integration checks can skip when prerequisites are absent, so its
result alone is not the VM release gate.
