# Running tests

This is the single reference for running Sherpa's tests. Other documents link here
instead of repeating these instructions.

## Layout

| Path | Contents |
|---|---|
| `crates/*/tests/`, `#[cfg(test)]` modules | Rust unit and integration tests (Cargo requires these locations) |
| `tests/specs/` | Testing principles and what each crate must test ([README](specs/README.md)) |
| `tests/environments/` | Disposable Ubuntu VMs for the VM suites ([README](environments/README.md)) |
| `tests/harness/` | VM test tooling: `test_env.py`, the release-test runner and guest helpers |
| `tests/labs/` | Labs under test, driven by the `sherpa` CLI against an installed server ([README](labs/README.md)) |
| `tests/bats/` | Supplementary installer BATS tests |
| `tests/web/` | Web UI JavaScript tests |
| `dev/testdb` | Local SurrealDB container for integration tests |

## Quick reference

| Test | Command | Needs | Time |
|---|---|---|---|
| Format and lint | `cargo fmt --check && cargo clippy --workspace -- -D warnings` | Rust toolchain | about 1–5 min |
| Unit tests | `cargo test --workspace` | Nothing external | about 1–3 min |
| db integration | `cargo test -p db -- --ignored --test-threads=1` | `dev/testdb` | about 3 min |
| Other crate integration | see [Crate integration tests](#crate-integration-tests) | Docker, libvirt, sudo | minutes |
| VM code suite | `python3 tests/harness/test_env.py run [ubuntu-2404] [ubuntu-2604]` | Sherpa host, logged-in CLI | about 40 min per release, in parallel |
| Installer suite | `./tests/harness/test_install.sh --config tests/environments/<env>/release-test.toml` | Sherpa host, logged-in CLI | about 20–30 min |
| Private bridge lab test | `python3 tests/labs/private-bridge/private_bridge_lab_test.py` | Sherpa host with the alpine and FRR images, logged-in CLI | about 5–10 min |
| P2p containers lab test | `python3 tests/labs/p2p-containers/p2p_containers_lab_test.py` | On the Sherpa server: the node images, Docker access, logged-in CLI | about 2–5 min |
| Talos lab test | `python3 tests/labs/talos-linux/talos_lab_test.py` | Sherpa host with the Talos image, logged-in CLI, internet | about 25–35 min |
| Web UI JavaScript | `node tests/web/test_theme.cjs` | Node.js | seconds |
| Harness self-tests | `python3 tests/harness/<script>.py --self-test` | Python 3.11+ | seconds |

Nothing runs these automatically. Every suite is started by hand, and the VM suites work
from a snapshot of the checkout, so you can keep editing and committing while they run.

## Before any tests

Formatting and linting must pass first; CI fails on either:

```sh
cargo fmt
cargo clippy --workspace -- -D warnings
```

## Unit tests

```sh
cargo test --workspace
```

Unit tests need no database, Docker, libvirt or network access. Integration tests are
marked `#[ignore]` and are skipped here.

## Crate integration tests

Integration tests are `#[ignore]` tests that talk to real services. They need a host with
the services each crate uses; the [VM code suite](#vm-code-suite) provides one
automatically. To run them directly, start the test database:

```sh
SHERPA_DEV_DB_USER=sherpa SHERPA_DEV_DB_PASS="$PASSWORD" dev/testdb restart
export SHERPA_DB_PASSWORD="$PASSWORD"
```

Then run the crates you need:

| Crate | Command | Needs |
|---|---|---|
| db | `cargo test -p db -- --ignored --test-threads=1` | Test database |
| container | `cargo test -p container -- --ignored --test-threads=1` | Docker, `alpine:latest` |
| network | `sudo -E env PATH="$HOME/.cargo/bin:$PATH" cargo test -p network -- --ignored --test-threads=1` | root (rtnetlink) |
| libvirt | `cargo test -p libvirt -- --ignored --test-threads=1` | libvirtd, `sherpa-pool` |
| sherpad | `sudo -E env PATH="$HOME/.cargo/bin:$PATH" cargo test -p sherpad -- --ignored --test-threads=1` | root, Docker, libvirt, test images |

`tests/harness/run-integration-tests.sh` runs all of them in order. Container, network,
libvirt and sherpad tests create real resources: run them only on a disposable host.

Opt-in live tests read these environment variables:

| Variable | Effect |
|---|---|
| `SHERPA_OVERLAY_LIVE=1` | Runs the libvirt overlay isolation test (disposable volumes in `sherpa-pool`) |
| `SHERPA_TEST_KEEP_FAILED_LABS=1` | A failed lab test keeps its lab for debugging |
| `SHERPA_TEST_TAILSCALE_*` | The live tailnet enrollment test; deferred, see `PROJECT.md` |

Test database commands: `dev/testdb start | stop | restart | status | logs | destroy`.
It listens on `ws://localhost:42069`.

## VM code suite

Builds the checkout and tests it in fresh Ubuntu VMs created on a Sherpa server:

```sh
python3 tests/harness/test_env.py run                  # 24.04 and 26.04
python3 tests/harness/test_env.py run ubuntu-2604      # one release
```

It needs an authenticated `sherpa` CLI with at least four hours left on its token
(`sherpa login`), and a Sherpa server with the environment images. For each release it:

1. builds `sherpad` and `sherpa` release archives from a snapshot of the checkout,
   including uncommitted changes,
2. creates a fresh lab, installs those archives with the checkout's installer, initializes
   the server and checks API and CLI access,
3. installs the Rust toolchain, then runs the workspace unit tests and the db, container,
   network, libvirt and sherpad integration tests inside the VM,
4. fails on any failed or skipped required test and on Docker, libvirt or network resources
   left behind.

A passing VM is destroyed; a failing one is kept. Then:

```sh
python3 tests/harness/test_env.py rerun .tmp/vm-release-tests/<run-id>    # retest a kept VM
python3 tests/harness/vm_release_test.py --cleanup-run .tmp/vm-release-tests/<run-id>   # remove it
```

A rerun uploads the current checkout and repeats the test steps without reinstalling; it is
for debugging, not a substitute for a fresh run. Results are in `.tmp/test-env-runs/<id>/`
(build logs, candidate hashes) and `.tmp/vm-release-tests/<run-id>/` (`result.toml`,
`test-results.toml`, `logs/`). Settings are in each environment's `release-test.toml`.

To manage one environment by hand: `python3 tests/harness/test_env.py up | check | destroy <env>`.

## Installer suite

Tests the checkout's installer and uninstaller with published release binaries:

```sh
./tests/harness/test_install.sh --config tests/environments/ubuntu-2604/release-test.toml
```

Scenarios (`--scenario`): `lifecycle` (default), `preflight`, `upgrade` and
`failure-checks`. See the [harness guide](specs/install/vm-release-harness.md) for each
scenario, local candidate archives and result verification.

The BATS tests are supplementary:

```sh
bats tests/bats/install_tests.bats
```

Skipped BATS integration checks do not count as installer coverage.

## Lab tests

Lab tests build real labs with the `sherpa` CLI against the server it is logged in to, check
them from inside the lab, and always destroy them. They test whatever `sherpad` that server
runs, so install the build under test first.

```sh
python3 tests/labs/private-bridge/private_bridge_lab_test.py  # private bridges: a VM and containers
python3 tests/labs/p2p-containers/p2p_containers_lab_test.py  # P2p links on NOS, app and OS containers
python3 tests/labs/talos-linux/talos_lab_test.py         # Talos clusters: single and ha labs in parallel
```

See [labs/README.md](labs/README.md) for the rules and each lab's README for what it checks.

## Harness self-tests

```sh
python3 tests/harness/test_env.py --self-test
python3 tests/harness/test_env_guest.py --self-test
python3 tests/harness/vm_release_test.py --self-test
python3 tests/harness/vm_release_guest.py --self-test
python3 tests/labs/private-bridge/private_bridge_lab_test.py --self-test
python3 tests/labs/p2p-containers/p2p_containers_lab_test.py --self-test
python3 tests/labs/talos-linux/talos_lab_test.py --self-test
```

Run these after changing anything in `tests/harness/` or a lab runner.

## When to run what

| Change | Run |
|---|---|
| Any change | Format, lint and unit tests |
| Database schema or queries | Also the db integration tests |
| Runtime code (server services, libvirt, container, network, guest templates) | The VM code suite on one release, in the background, before merging |
| Installer or uninstaller scripts | The installer suite |
| Lab networking (links, bridges, container or VM interfaces) | The private bridge lab test, against a server running the change; for P2p or container interfaces, also the P2p containers lab test |
| Talos nodes or clusters | The Talos lab test, against a server running the change |
| Before a release, or periodically | The VM code suite on both releases |

These are guidelines, not gates: none of them blocks a commit, a version bump or a merge.

## Known issues

- **TAP devices on kernel 6.8+:** `create_tap()` uses `InfoKind::Tun` through rtnetlink,
  which kernel 6.8 rejects. `test_create_tap`, `test_get_ifindex` and
  `test_ebpf_redirect_between_taps` detect this and skip.

## Conventions

- Integration tests use `#[ignore]` and run with `-- --ignored`.
- Tests that share host state run with `--test-threads=1`.
- Database tests use a unique namespace per test.
- Container tests prefix resources with `sherpa-test-`.
- Tests that create labs always clean them up, including on failure.
