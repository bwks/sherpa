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

## Known issue: concurrent `up`

Bring environments up one at a time. On 2026-10-16, running `sherpa up` for both
environments at once gave both labs the same management subnet; the second failed
with `Network is already in use by interface brm-<lab-id>` and cleaned itself up.
That is a Sherpa server bug in lab subnet allocation
([#318](https://github.com/bwks/sherpa/issues/318)), not a test-environment
problem.

## Consumers

- The [VM release-test harness](../test-specs/install/vm-release-harness.md) builds
  its disposable guests from [ubuntu-2604](ubuntu-2604/manifest.toml), with settings
  in [release-test.toml](ubuntu-2604/release-test.toml).
- Integration tests still run in the `devbox_linux` VM from `dev/integration-test/`
  ([how to run](../test-specs/integration/HOW-TO-RUN.md)). That moves here once the
  integration tests run on a plain Ubuntu environment with Sherpa installed by its
  installer.
