# Test environments

Sherpa tests Sherpa. Each directory here holds a Sherpa manifest for a disposable
test server, built from an official Ubuntu cloud image by an existing, known-good
Sherpa server. The test candidate is installed inside the guest; the host
installation is never the thing under test.

| Environment | Ubuntu | Sherpa image version | Resources |
|---|---|---|---|
| [ubuntu-2404](ubuntu-2404/README.md) | 24.04 LTS | `ubuntu_linux` `24.04-20260926` | 8 vCPUs, 16 GiB RAM, 100 GiB disk |
| [ubuntu-2604](ubuntu-2604/README.md) | 26.04 LTS | `ubuntu_linux` `26.04` | 8 vCPUs, 16 GiB RAM, 100 GiB disk |

[dev](dev/README.md) is not a test environment: it is a long-lived development server, with
toolchains and checkouts provisioned on top of Ubuntu 24.04. The rules below apply to it only
where they make sense; it is not disposable and installs more than cloud-init needs.

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

## Environment check

Commands for bringing environments up, checking them and running the test suites in them
are in [running tests](../README.md). `test_env.py check` copies the guest helpers into the
guest and runs them with passwordless sudo. A passing environment has:

- cloud-init finished without errors,
- the Ubuntu release, vCPU count and disk size from its manifest,
- no Sherpa, Docker, libvirt or SurrealDB commands and no `/opt/sherpa`,
- working nested KVM (`/dev/kvm` reports API version 12 and creates a VM handle),
- Sherpa's cloud-init login account with passwordless sudo,
- HTTPS access to the Ubuntu archive, the GitHub release API and the Docker
  installer, and the expected authentication challenge from Docker Hub and GHCR.

Network checks retry each endpoint up to three times; Canonical's archive
intermittently fails a single TLS handshake. A failure names the endpoint.

## Settings

Each environment's `release-test.toml` holds the runner settings for both suites. Its
`[tests]` section configures the VM code suite: the integration crates, test images,
tests skipped by name and non-secret environment variables for opt-in live tests.
Credentials are never stored there.

## Concurrent lab creation

Sherpa servers before the fix for [#318](https://github.com/bwks/sherpa/issues/318)
can give concurrent `sherpa up` requests the same lab subnet. The host server that builds
these environments may be such a version, so the release-test runner takes a host lock
(`.tmp/sherpa-up.lock`) around `sherpa up`: environments start one after another and then
run in parallel. Remove the lock once the host runs a fixed server. When using
`tests/harness/test_env.py up` by hand against an older server, bring environments up one at a
time.

## Consumers

- The [VM release-test harness](../specs/install/vm-release-harness.md) builds
  its disposable guests from each environment's manifest, with settings in that
  environment's `release-test.toml`. Without `--config` it uses ubuntu-2604.
- The VM code suite (`tests/harness/test_env.py run`) runs the integration tests in these
  environments with Sherpa installed by its installer. See [running tests](../README.md).
