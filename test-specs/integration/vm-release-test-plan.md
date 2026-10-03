# VM Release Testing — Implementation Checklist

## Goal and scope

Build a repeatable end-to-end release test harness using disposable VMs managed
by a stable Sherpa server. The first target is a bare Ubuntu 26.04 guest running
the real server installation and uninstallation scripts.

Complete phases 1–5 before adding new product features. Phase 6 extends the same
harness to broader release coverage. This document records the plan; an imported
image alone does not establish a test harness. The local runner now passes the
Ubuntu installation/uninstallation lifecycle; remaining diagnostic, isolation
and release gate tasks are listed below.

**External dependencies:** a working Sherpa host, libvirt/KVM with nested
virtualization support, guest SSH access and sudo, and internet access for Ubuntu
packages and runtime images. Scenarios in this plan are `[e2e]` tests.

## Design decisions

- Keep the host Sherpa installation on a known working version. Install and test
  the release candidate inside the guest.
- Use an official Ubuntu cloud image with a recorded build and checksum.
- Configure the VM through a normal Sherpa TOML manifest. Select version `26.04`
  explicitly so later default changes do not silently change the test.
- Start each independent scenario from a fresh guest disk. Preserve state only
  within scenarios that deliberately test reinstallation and data retention.
- Keep helper scripts under `scripts/` and developer instructions in this source
  repository. Test settings use TOML and remain overridable.
- Run installer and uninstaller operations only inside the disposable guest.
- Record the exact candidate commit, scripts, artifacts and image used by a run.
  A release result applies only to those inputs.

## Phase 0 — Image preparation

Completed on 2026-10-03:

- [x] Import the official Ubuntu 26.04 Server amd64 cloud image into Sherpa.
- [x] Verify the stored image against Canonical's SHA-256 checksum.
- [x] Set Ubuntu 26.04 as the default `ubuntu_linux` image.

| Property | Recorded value |
|---|---|
| Sherpa model | `ubuntu_linux` |
| Sherpa version | `26.04` |
| Canonical build | `20260927` |
| Image filename | `ubuntu-26.04-server-cloudimg-amd64.img` |
| Server path | `/opt/sherpa/images/ubuntu_linux/26.04/virtioa.qcow2` |
| SHA-256 | `8800651811af9a85465ad1d552add729947bb16488dddb4a9b5305a3d97332b2` |

Source: [Canonical build 20260927](https://cloud-images.ubuntu.com/releases/resolute/release-20260927/)
and its [SHA256SUMS](https://cloud-images.ubuntu.com/releases/resolute/release-20260927/SHA256SUMS).
Recheck the checksum when preparing a run because Sherpa's `26.04` version label
does not itself identify the Canonical build.

## Phase 1 — First bare Ubuntu VM

Completed on 2026-10-03 using the existing configured Sherpa server:

- [x] Confirm which stable Sherpa host will create and manage the test VM.
- [x] Confirm host capacity and nested virtualization support without changing
  running workloads.
- [x] Create a dedicated test manifest for one Ubuntu 26.04 VM. Start with
  4 CPUs, 8 GB RAM and a 60 GB disk; keep these values configurable.
- [x] Ensure Sherpa exposes the host's supported virtualization features to the
  guest and verify usable nested KVM inside it.
- [x] Limit bootstrap setup to guest access and required management tooling:
  SSH, sudo, cloud-init and any necessary guest agent. Record any additional
  installer prerequisite packages, such as curl.
- [x] Boot the VM, wait for cloud-init completion and verify SSH access.
- [x] Assert Ubuntu 26.04 is running and Sherpa, Docker, libvirt and SurrealDB
  are absent before installation begins.
- [x] Verify the guest can reach package repositories and required registries.

Manifest: [`dev/release-test/manifest.toml`](../../dev/release-test/manifest.toml).
The [baseline record and access instructions](../../dev/release-test/README.md)
cover lab `f804c420`, running Ubuntu 26.04.1 LTS. A temporary KVM VM was created
successfully inside the guest. No server runtime or extra packages were installed.

**Acceptance:** Sherpa can create a clean, reachable Ubuntu 26.04 guest suitable
for testing the installer. No development image or preinstalled server runtime
is used as the baseline.

## Phase 2 — Harness and exact candidate inputs

Local runner implemented and validated on 2026-10-03. See the
[developer guide](../../dev/release-test/README.md) and
[runner settings](../../dev/release-test/config.toml).

- [x] Define one local entry point to prepare a VM, run a selected scenario,
  collect results and clean up resources created by that run.
- [x] Define configurable inputs for host connection, VM resources, image build,
  candidate version/artifacts, timeouts, output directory and failure retention.
- [x] Transfer the exact installer and uninstaller scripts from the candidate
  commit into the guest; record their hashes.
- [x] Establish a baseline run using an explicitly selected published release.
- [x] Provide a way for the installer to consume candidate release artifacts
  before publication. Preserve its normal published-release workflow.
- [x] Verify installed binaries match the supplied candidate artifacts; a
  matching version string alone is insufficient evidence.
- [x] Automate installer inputs and required server initialization without
  manual prompts, using disposable test credentials.
- [x] Audit the existing install tests, including obsolete `--db-pass` usage in
  `scripts/test_install.sh`, and reuse valid assertions from the BATS suite.
- [x] Add bounded waits, clear failures and a nonzero exit status when a required
  scenario fails, times out or is unexpectedly skipped.
- [x] Verify the target guest identity before running installation, removal or
  cleanup operations; keep the stable host outside those operations.

**Acceptance:** a run tests the intended scripts and binaries inside the guest
and produces an unambiguous pass or fail result without manual intervention.

## Phase 3 — Installation and uninstallation scenarios

- [x] Fresh installation: verify exit status, binary versions, symlinks,
  account/group setup, directory ownership and permissions.
- [x] Runtime setup: verify Docker and libvirt availability, required images,
  SurrealDB persistence and database health.
- [x] Service setup: verify the systemd unit, environment file permissions and
  enablement. Account for the installer leaving startup until initialization.
- [x] Server startup: complete initialization, start the service and verify
  authenticated CLI/API access.
- [x] Reboot recovery: reboot the guest and verify the server and database
  return to a usable state with retained data.
- [x] Repeat installation: rerun on an initialized guest and verify success,
  unchanged configuration and preserved test data.
- [x] Uninstall with data retention: verify server shutdown, removal of its
  service/binaries/database container and preservation of the documented data.
- [x] Reinstall after data retention: verify the saved configuration and a
  previously created test record remain usable through the server.
- [x] Uninstall with database removal: verify database data is removed and
  other retained files follow the documented `--remove-data` contract.
- [x] Full uninstall: verify removal of Sherpa's installation directory,
  service, symlinks, logrotate entry and database container.
- [ ] Define and verify which dependencies, users, groups and external resources
  each uninstall mode intentionally retains. Full uninstall is not assumed to
  restore the original Ubuntu package set.
- [x] Repeat uninstall: verify the documented behavior on an already removed
  installation, including handling of missing services and containers.
- [x] Failure cases: verify invalid inputs, an unavailable artifact and an
  occupied port produce clear errors and the documented cleanup behavior.

**Acceptance:** the real installer and uninstaller pass the required lifecycle
scenarios on Ubuntu 26.04. Database health alone is not a server startup check.

## Phase 4 — Isolation, evidence and repeatability

- [x] Recreate the guest from the verified base image between independent
  scenarios; do not use uninstall as a substitute for a fresh baseline.
- [x] Assign each run its own lab/resources and prevent concurrent runs from
  sharing guest disks, credentials or result directories.
- [ ] Capture scenario results, script output, server/database logs, systemd
  status and boot diagnostics before cleanup, including on timeout or failure.
- [x] Record the candidate commit/version, artifact/script hashes, Ubuntu build,
  image checksum and resolved VM settings with the results.
- [x] Ensure credentials are excluded from shared logs and reports.
- [x] Provide an option to retain a failed VM and report how to inspect and
  remove that specific run's resources.
- [x] Verify normal cleanup removes only resources owned by the run and leaves
  the stable host, other labs and base image intact.
- [ ] Verify interruptions and partial provisioning leave enough information for
  targeted cleanup.
- [x] Run the complete required suite twice from fresh guests and investigate
  any difference in outcomes.

**Acceptance:** runs are repeatable, failures are diagnosable and cleanup is
limited to the disposable test environment.

**Verification recorded on 2026-10-03:** both complete lifecycle runs passed
with the same installer, uninstaller and binary hashes in different fresh VMs.

| Run ID | Binary source | Result | Owned VM/disks |
|---|---|---|---|
| `3f052e69107b42e796ece077f21aff51` | GitHub release `v0.3.79` | Passed | Removed |
| `8478c7676f0c459aa92748e9210c9490` | Supplied local `v0.3.79` archives | Passed | Removed |

Each local receipt is at `.tmp/vm-release-tests/<run-id>/result.toml`. The
base image checksum remained unchanged after both runs. The original bare
lab and pre-existing VMs remained unchanged. Four earlier failed labs are
retained under the configured failure-retention policy.

Both runs used the checkout's scripts and `v0.3.79` binaries. For a future
candidate, supply that candidate's archives with `--version` and
`--artifact-dir`. Formatting, workspace Clippy and 17 harness regression
checks passed. No CI/release pipeline definitions were changed.

Still outstanding: explicit assertions for all retained external resources,
server/database log collection before each uninstall, and live interruption
or partial-provisioning checks. The runner already retains failure evidence
and conservatively records possible resources when provisioning is interrupted;
that reporting behavior has a regression test.

## Phase 5 — Release gate

- [x] Document the local workflow, prerequisites, expected results and failure
  inspection steps in the source repository.
- [ ] Require successful formatting and workspace Clippy before Rust tests,
  following the existing repository rules.
- [ ] Require the Ubuntu 26.04 installation/uninstallation suite for every
  release candidate and retain its result record with the release evidence.
- [ ] Publish the same artifacts that passed the harness; rerun the gate if
  scripts or artifacts change afterwards.
- [ ] Integrate the local entry point into the release workflow after review of
  the required CI/release pipeline changes.
- [ ] Confirm a failed, timed-out or unexpectedly skipped required scenario
  blocks release publication.
- [ ] Record completion of phases 1–5 before starting new product features.

**Acceptance:** an exact candidate can be tested before publication, and the
release process requires a passing result for those artifacts.

## Phase 6 — Later coverage using the same harness

- [ ] Upgrade from the previous release and verify persisted data and settings.
- [ ] Container lab creation, inspection, shutdown, resume and destruction.
- [ ] Nested VM lab creation, inspection, shutdown, resume and destruction.
- [ ] Mixed VM/container labs and their networking.
- [ ] Tailscale enrollment, routing, stop/resume, isolation and cleanup using a
  disposable tailnet and explicitly supplied credentials.
- [ ] Run the existing service and crate integration suites inside suitable
  disposable guests, keeping their setup separate from the bare installer baseline.
- [ ] Add supported Ubuntu versions and architectures as separate test targets.

## Existing material to reconcile during implementation

- [Installer specification](../install/sherpa-install.md).
- [Installer VM instructions](../install/HOW-TO-RUN.md): updated to point to the
  automated Ubuntu 26.04 workflow and distinguish supplementary BATS coverage.
- [Existing integration VM instructions](HOW-TO-RUN.md): use a development image
  with dependencies already installed.
- [Existing integration plan](../../docs/integration-test-plan.md).
- [Lab lifecycle specification](lab-lifecycle-e2e.md).
- [Tailscale verification guide](tailscale.md).

Reconcile the remaining service integration instructions as their coverage is
added. Keep the bare installer baseline distinct from guest environments prepared
for service integration tests.
