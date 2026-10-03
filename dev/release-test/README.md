# Bare Ubuntu release-test VM

This lab provides the clean Ubuntu baseline for testing Sherpa's server installer
and uninstaller. The test runtime will be installed inside the guest; the existing
Sherpa host manages the outer VM.

The [manifest](manifest.toml) selects Ubuntu `26.04` explicitly and allocates
4 vCPUs, 8192 MiB RAM and a 60 GiB boot disk. Change these settings in the manifest
when needed. The image build and checksum are recorded in the
[release-test plan](../../test-specs/integration/vm-release-test-plan.md).

## Access

From the repository root:

```sh
cd dev/release-test
sherpa inspect
sherpa ssh testbox
```

Use `sherpa up` from this directory to create or start the lab on the configured
Sherpa server. Sherpa generates `lab-info.toml`, an SSH configuration and an SSH
key locally; these runtime files are ignored by Git.

## Verified baseline

Verified on 2026-10-03 for lab `f804c420`, domain `testbox-f804c420`:

| Check | Result |
|---|---|
| Operating system | Ubuntu 26.04.1 LTS, `VERSION_ID=26.04` |
| Resources | 4 vCPUs, 7.75 GiB usable memory, 60 GiB disk |
| Guest initialization | Cloud-init finished without errors; SSH and passwordless sudo work |
| Nested virtualization | CPU exposes VMX; `/dev/kvm` reports API version 12; creating a temporary KVM VM succeeds |
| Server baseline | Sherpa, Docker, libvirt and SurrealDB commands and runtime packages absent; `/opt/sherpa` absent |
| Installer prerequisite | Curl is already present in the official image; no extra packages were installed |
| Guest network access | Ubuntu repository metadata, GitHub release API and Docker installer reachable; Docker Hub and GHCR return their expected authentication challenges |

The existing host has nested virtualization enabled, and Sherpa's normal
host-model CPU configuration exposes the required features to the guest. No changes
to the host's virtualization settings were needed. Existing VMs were left unchanged.

The KVM check created and closed an in-memory VM handle; it did not boot an inner
guest. Registry checks establish connectivity, not successful image pulls.
The automated runner creates separate VMs; this original baseline lab stays clean.

## Automated install/uninstall tests

The local machine needs Python 3.11 or later, curl, OpenSSH with scp, and an
authenticated Sherpa CLI. Its SSH account on the configured Sherpa host must be
able to read the image checksum and query domain UUIDs with virsh. The guest needs
internet access for Ubuntu packages, Docker and container registries.

Run from the repository root:

```sh
cargo fmt
cargo clippy --workspace -- -D warnings
./scripts/test_install.sh
```

All connection settings, image identity, candidate inputs, timeouts, output and
retention settings are in [config.toml](config.toml). Resources and Ubuntu version
are in the [manifest](manifest.toml). An empty host server URL uses the existing
CLI connection. Each run generates a unique lab name, guest name and credentials.

The default tests the current checkout's installer/uninstaller scripts with
explicitly selected published `v0.3.79` binaries. It records the checkout commit,
whether it is dirty, script hashes, archive and installed binary hashes, base
image checksum and VM identity. This is a published-release baseline for the
current scripts, not proof that unpublished source changes are in those binaries.

To test release archives before publication:

```sh
./scripts/test_install.sh --version v0.3.80 --artifact-dir /path/to/release-artifacts
```

The directory must contain `sherpad-x86_64-unknown-linux-gnu.tar.gz` and
`sherpa-x86_64-unknown-linux-gnu.tar.gz`, with the corresponding binary at each
archive's root. The installer uses `SHERPA_ARTIFACT_DIR` for these supplied files;
its normal GitHub download workflow remains available. The runner verifies the
installed bytes against the archives and checks the selected version.

The lifecycle scenario checks preflight errors, fresh installation, ownership and
permissions, Docker/libvirt/database readiness, server initialization, authenticated
API and CLI access with TLS validation, reboot recovery, repeat installation, data
retention/reinstallation, data removal, full uninstall and repeated uninstall.
Its persistent test record is the generated admin account created during init.
Required checks cannot silently skip, and a failure or timeout returns a nonzero
exit code. Use `--scenario preflight` to run only baseline and argument/error checks.

Server removal with `--keep-data` retains the database, configuration, SSH identity
and CLI. `--remove-data` empties the database but retains the configuration, identity
and CLI. `--remove-all` removes the installation directory and its binary symlinks.
Docker/libvirt packages, images, users/groups and libvirt resources are retained by
all modes; recreating the guest provides a genuinely fresh baseline.

## Evidence and failure inspection

Each run writes `result.toml`, candidate inputs and numbered logs under
`.tmp/vm-release-tests/`, in a private directory ignored by Git. The runner prints
the result path. Logs include installation output, service and database diagnostics
and boot information, with generated credentials redacted.

Successful runs destroy only their own lab after checking its lab and domain
identity. Failed runs are retained by default. `--keep-vm` retains a successful VM
as well. For a retained run, enter its result directory and use `sherpa inspect`;
use the receipt's `node_name` with `sherpa ssh` to connect. Run `sherpa destroy --yes`
from that same directory when finished inspecting its manifest and lab identity.

Harness regression checks do not provision a VM:

```sh
./scripts/test_install.sh --self-test
python3 scripts/vm_release_guest.py --self-test
```

The [release checklist](../../test-specs/integration/vm-release-test-plan.md)
tracks verified VM runs and the remaining release gate work. CI/release pipeline
integration is a separate step.
