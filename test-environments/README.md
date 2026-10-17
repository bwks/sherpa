# Test environments

Sherpa tests Sherpa. Each directory here holds a Sherpa manifest for a disposable
test server, built from an official Ubuntu cloud image by an existing, known-good
Sherpa server. The test candidate is installed inside the guest; the host
installation is never the thing under test.

| Environment | Ubuntu | Sherpa image version | Resources |
|---|---|---|---|
| [ubuntu-2404](ubuntu-2404/README.md) | 24.04 LTS | `ubuntu_linux` `24.04-20260926` | 8 vCPUs, 16 GiB RAM, 100 GiB disk |
| [ubuntu-2604](ubuntu-2604/README.md) | 26.04 LTS | `ubuntu_linux` `26.04` | 8 vCPUs, 16 GiB RAM, 100 GiB disk |

## Rules

- One directory per test server, each with one `manifest.toml` describing a
  single `ubuntu_linux` node. Each environment is its own lab.
- Pin the image `version` in the manifest. Changing the server's default
  `ubuntu_linux` image must never silently change a test environment.
- Record the Canonical build and SHA-256 of each image in the environment's
  README. An image without a verified checksum is not a test baseline.
- Install nothing beyond what cloud-init needs for guest access. Docker, libvirt
  and SurrealDB must come from Sherpa's installer, so the installer is tested too.
- `sherpa up` writes `lab-info.toml`, `sherpa_ssh_config` and `sherpa_ssh_key`
  into the environment directory. Git ignores them; never commit them.

## Usage

Run from the repository root with an authenticated Sherpa CLI:

```sh
python3 scripts/test_env.py up ubuntu-2604       # sherpa up, then check
python3 scripts/test_env.py check ubuntu-2604    # check a running environment
python3 scripts/test_env.py destroy ubuntu-2604  # sherpa destroy --yes
```

`check` copies `scripts/test_env_guest.py` and `scripts/vm_release_guest.py` into
the guest and runs them with passwordless sudo. A passing environment has:

- cloud-init finished without errors,
- the Ubuntu release, vCPU count and disk size from its manifest,
- no Sherpa, Docker, libvirt or SurrealDB commands and no `/opt/sherpa`,
- working nested KVM (`/dev/kvm` reports API version 12 and creates a VM handle),
- Sherpa's cloud-init login account with passwordless sudo,
- HTTPS access to the Ubuntu archive, the GitHub release API and the Docker
  installer, and the expected authentication challenge from Docker Hub and GHCR.

Network checks retry each endpoint up to three times; Canonical's archive
intermittently fails a single TLS handshake. A failure names the endpoint.

Self-tests for the helpers run without a VM:

```sh
python3 scripts/test_env.py --self-test
python3 scripts/test_env_guest.py --self-test
```

## Testing the local code

Two test suites run in these environments:

| Suite | Command | Sherpa under test |
|---|---|---|
| Installer | `./scripts/test_install.sh --config test-environments/<env>/release-test.toml` | The checkout's installer and uninstaller with published release binaries |
| Code | `python3 scripts/test_env.py run` | Release binaries and integration tests built from the checkout |

The code suite builds `sherpad` and `sherpa` release archives once on the host from a
snapshot of the checkout (tracked and untracked, non-ignored files, so uncommitted work
is included). It builds them per package with Tailwind CSS rebuilt, as the release
workflow does. Then, for every environment in parallel, the release-test runner:

1. creates a fresh lab from the environment's manifest,
2. installs the candidate archives with the checkout's installer, initializes the
   server and checks API and CLI access,
3. uploads the same source snapshot and installs the Rust toolchain and build packages,
4. pulls the container image and downloads the checksummed VM image the tests need,
5. builds all test binaries, runs the workspace unit tests, then runs the db,
   container, network, libvirt and sherpad integration tests one crate at a time with
   the installed `sherpad` service stopped,
6. fails on any failed test, any crate with no results, a required test that skipped
   (such as the VM lifecycle test without KVM) or any Docker, libvirt or network
   resource left behind by the tests.

Settings are in each environment's `release-test.toml` `[tests]` section. The client
crate's integration tests are not run.

A passing environment is destroyed. A failing one is kept for debugging. To retest
the current checkout on a kept VM without rebuilding it:

```sh
python3 scripts/test_env.py rerun .tmp/vm-release-tests/<run-id>
```

A rerun reinstalls nothing: it uploads a new source snapshot and repeats the build,
unit and integration steps. If it passes, the VM is destroyed. A rerun result is for
debugging; release confidence needs a passing fresh run. Remove a kept VM without
rerunning with `python3 scripts/vm_release_test.py --cleanup-run .tmp/vm-release-tests/<run-id>`.

Each run writes `.tmp/test-env-runs/<id>/` with the build logs, candidate hashes
(`candidate.toml`) and one log per environment. Each environment's receipt is under
`.tmp/vm-release-tests/<run-id>/`, with `test-results.toml` (per-crate counts,
failed test names, skips and leftover resources) and full test logs in `logs/`.
Both directories are ignored by Git.

## Concurrent lab creation

Sherpa servers before the fix for [#318](https://github.com/bwks/sherpa/issues/318)
can give concurrent `sherpa up` requests the same lab subnet. The host server that builds
these environments may be such a version, so the release-test runner takes a host lock
(`.tmp/sherpa-up.lock`) around `sherpa up`: environments start one after another and then
run in parallel. Remove the lock once the host runs a fixed server. When using
`scripts/test_env.py up` by hand against an older server, bring environments up one at a
time.

## Consumers

- The [VM release-test harness](../test-specs/install/vm-release-harness.md) builds
  its disposable guests from each environment's manifest, with settings in that
  environment's `release-test.toml`. Without `--config` it uses ubuntu-2604.
- Integration tests still run in the `devbox_linux` VM from `dev/integration-test/`
  ([how to run](../test-specs/integration/HOW-TO-RUN.md)). That moves here once the
  integration tests run on a plain Ubuntu environment with Sherpa installed by its
  installer.
