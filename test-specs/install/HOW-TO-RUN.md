# Running installer tests

The supported local end-to-end workflow uses a fresh Ubuntu 26.04 VM provisioned
by Sherpa. Follow the [VM release-test guide](../../dev/release-test/README.md):

```sh
./scripts/test_install.sh
```

The runner tests installer preflight failures, installation, initialization,
authenticated server access, reboot recovery, repeat installation, data retention,
data removal, full removal and repeated uninstall. Required checks fail rather
than skip. Run settings are in [TOML](../../dev/release-test/config.toml).

It uses `SHERPA_DB_PASSWORD` for the installer. The `--db-pass` flag belongs to
`sherpad init`, not to `sherpa_install.sh`. Initialization and its prompts are
automated inside the guest.

Each independent run starts from a fresh cloud-image disk. Reinstallation and
uninstallation within a run intentionally retain state to test those contracts.
Uninstalling does not restore the original Ubuntu package set.

## Existing BATS tests

For the supplementary installer unit tests:

```sh
bats test-scripts/install_tests.bats
```

BATS must already be installed. Some tests require an Ubuntu host with
virtualization support, and integration tests skip without an installation.
Do not count skipped BATS integration checks as release coverage. Use the VM
runner for lifecycle testing; its repeat-install check keeps the existing
database container in place so the installer must handle it itself.

See the [test specification](sherpa-install.md) and
[release checklist](../integration/vm-release-test-plan.md) for coverage and
remaining release integration work.
