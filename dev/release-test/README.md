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

To test locally built candidate archives:

```sh
./scripts/test_install.sh --version v0.3.80 --artifact-dir /path/to/release-artifacts
```

Use `--result-file /path/to/result-reference.toml` to write a stable, nonsecret
TOML reference to the generated receipt. This identifies one exact run when the
output directory contains earlier results.

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

Each uninstall compares package versions, account/group entries, Docker images,
volumes and unrelated containers, and libvirt domain/network/pool definitions and
state before and after removal. The resource inventories are retained with the
result. Full removal deletes `/opt/sherpa`, including paths referenced by retained
libvirt pool definitions; it does not remove those definitions or restore Ubuntu's
original package set. Server journal and database logs are captured before each
uninstall. A daemon log file is captured as well when present; the systemd service
runs in foreground mode and writes its server logs to the journal.

## Upgrade testing

Select both versions explicitly and supply locally built candidate archives:

```sh
./scripts/test_install.sh --scenario upgrade --baseline-version v0.3.79 --version v0.3.80 --artifact-dir /path/to/release-artifacts --result-file /path/to/upgrade-reference.toml
```

The runner downloads the previous release on the controller and transfers it
separately from the candidate. Use `--baseline-artifact-dir /path/to/previous-artifacts`
to supply the previous release locally. The optional TOML `[baseline]` section has
`version` and `artifact_directory` settings; the CLI flags override them.
Both installations use the recorded checkout's real installer. This verifies a
binary upgrade using that installer; it does not test the previous release's
historical installer. Both input archive/binary hashes and the baseline installer
hash/source are recorded. Equal versions, identical binary bytes or a missing
local candidate selection are rejected before provisioning.

The upgrade scenario creates a fresh guest, installs and initializes the baseline,
then creates an additional non-admin user through its authenticated API. It upgrades
the same installation without reinitializing the database. It checks candidate
binary versions/hashes, API/CLI authentication, both persisted users' credentials
and privileges, complete user records, configuration, environment, SSH identity
and TLS certificate fingerprints. It repeats those checks after a real reboot.
Environment fingerprints cover every assignment while excluding generated
comments and blank lines, so the installer's timestamp comment can change.
Before creating the workspace, upgrade runs wait for authenticated SSH within
the configured startup timeout. Guest identity checks still run before changes.

`upgrade-state.toml` records retained state before upgrade, after upgrade and after
reboot, including installed binary fingerprints for all three phases. A completed
run requires that evidence to be collected and validated. The receipt records its
SHA-256 along with both sets of inputs. Upgrade receipts are separate from complete
install/uninstall lifecycle receipts; the existing lifecycle artifact verifier
continues to require the lifecycle scenario. Failure retention, diagnostics and
targeted cleanup use the same settings and ownership checks as other scenarios.

## Failure checks and artifact verification

Run the live harness failure suite with the same candidate inputs:

```sh
./scripts/test_install.sh --scenario failure-checks --version v0.3.80 --artifact-dir /path/to/release-artifacts
```

It creates separate fresh guests for a timed-out command, SIGTERM interruption
while a guest command is running, and incomplete provisioning after a readiness
timeout. The existing CLI can report readiness warnings with exit status zero;
the harness must still reject the unready guest. Each fault case must produce a
failed receipt and nonzero exit, preserve resource identity and diagnostics, stop
its guest command when applicable, and pass verified targeted cleanup. Expected
faults count as a passing failure-suite result, never as a passing lifecycle run.

After a complete lifecycle run, verify the exact archives and checkout:

```sh
python3 scripts/verify_release_test.py --receipt /path/to/run/result.toml --artifact-dir /path/to/release-artifacts --version v0.3.80
```

Alternatively use `--result-file` with the reference written by the runner.
The verifier rejects changed installer/harness scripts, archive or binary hashes,
missing required steps, incomplete resource evidence, failed cleanup and dirty
checkouts. It checks the recorded candidate commit against the current checkout's
HEAD, or the explicit `--commit` value, and requires a clean source checkout for
the recorded run. `--allow-dirty` permits local development
verification. Its optional `--stage-evidence` directory contains
only the selected receipt and guest resource inventories, without connection
settings, credentials or SSH keys.

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

For verified removal, including partially provisioned runs without local SSH
files, use `./scripts/test_install.sh --cleanup-run /absolute/path/to/run-directory`.
The receipt keeps its original pass/fail status and records verified removal.

Harness regression checks do not provision a VM:

```sh
./scripts/test_install.sh --self-test
python3 scripts/vm_release_guest.py --self-test
python3 scripts/vm_release_faults.py --self-test
python3 scripts/verify_release_test.py --self-test
```

The [VM testing checklist](../../test-specs/integration/vm-release-test-plan.md)
tracks verified VM runs and remaining harness coverage. Run the VM harness
locally against the existing Sherpa server using your authenticated CLI and SSH
access.
