# VM Release Testing — Implementation Checklist

## Goal and scope

Build a repeatable end-to-end release test harness using disposable VMs managed
by a stable Sherpa server. The first target is a bare Ubuntu 26.04 guest running
the real server installation and uninstallation scripts.

Complete phases 1–5 before adding new product features. Phase 6 extends the same
harness to broader release coverage. This document records the plan; an imported
image alone does not establish a test harness. Local lifecycle and failure checks
now pass for freshly built `0.3.80` archives. Tests run locally against the existing
Sherpa server.

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
- Invoke the VM controller locally using the existing Sherpa CLI and SSH access.

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
  from local files. Preserve its normal published-release workflow.
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
- [x] Define and verify which dependencies, users, groups and external resources
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
- [x] Capture scenario results, script output, server/database logs, systemd
  status and boot diagnostics before cleanup, including on timeout or failure.
- [x] Record the candidate commit/version, artifact/script hashes, Ubuntu build,
  image checksum and resolved VM settings with the results.
- [x] Ensure credentials are excluded from shared logs and reports.
- [x] Provide an option to retain a failed VM and report how to inspect and
  remove that specific run's resources.
- [x] Verify normal cleanup removes only resources owned by the run and leaves
  the stable host, other labs and base image intact.
- [x] Verify interruptions and partial provisioning leave enough information for
  targeted cleanup.
- [x] Run the complete required suite twice from fresh guests and investigate
  any difference in outcomes.

**Acceptance:** runs are repeatable, failures are diagnosable and cleanup is
limited to the disposable test environment.

**Verification recorded on 2026-10-03:** two complete lifecycle runs passed
with identical installer, uninstaller, harness and binary hashes in separate fresh
VMs. A third run then verified the final host helper's configurable SSH destination.

| Run ID | Binary source | Result | Owned VM/disks |
|---|---|---|---|
| `f0455214fff0475dab4ed3bf945364f1` | Fresh local `v0.3.80` archives | Passed | Removed |
| `d5f9c5c7e7014843bdaada67edde79d9` | Same local `v0.3.80` archives | Passed | Removed |
| `dc227d81ea624a7597b19ec7e44fb4df` | Same archives, final host helper | Passed | Removed |

Each local receipt is at `.tmp/vm-release-tests/<run-id>/result.toml`. The
base image checksum remained unchanged after cleanup. The original bare lab and
pre-existing VMs remain. The four previously retained failed labs were removed at
the user's request. Failed exploratory runs from this implementation were also
removed after their evidence was collected.

Every uninstall mode now compares inventories of packages, users, groups, Docker
images/volumes/unrelated containers, and libvirt domains/networks/pools. All were
retained unchanged during uninstall. Full removal leaves libvirt definitions and
Docker volumes; it removes `/opt/sherpa`, including the pool's backing directory.
It does not restore bare Ubuntu. Server journal entries and database container
logs were captured before all three uninstall modes.

Live timeout, SIGTERM interruption and incomplete-provisioning checks passed in
three separate fresh guests. Their intentionally failed receipts recorded the
allocated lab, domain UUID and disks; targeted cleanup verified their removal.
The controller also verified that active guest probe processes stopped.
Combined evidence: `.tmp/vm-release-tests/failure-checks/`
`c86086494ed7441c95a98e5164fbe374/fault-results.toml`.

All 11 crate versions are `0.3.80`. Formatting, workspace Clippy, the locked
workspace test suite and 34 harness regression checks passed. The server and CLI
were built with the release profile and packaged locally. The final receipt's
scripts, archive hashes and extracted binary hashes passed the artifact verifier,
including after staging the selected nonsecret evidence.

These are development results from an uncommitted checkout at
`cba3c377c8863a677d8cc42ff0abc8a378ec982e`. Local verification explicitly used
`--allow-dirty` because the checkout contained uncommitted changes. Earlier
`v0.3.79` baseline receipts remain historical evidence.

## Phase 5 — Local validation and documentation

- [x] Document the local workflow, prerequisites, expected results and failure
  inspection steps in the source repository.
- [x] Run successful formatting and workspace Clippy before local Rust tests,
  following the existing repository rules.
- [x] Provide a local Ubuntu 26.04 installation/uninstallation suite and retain
  its result record with the candidate's local test evidence.
- [x] Verify the candidate version, commit, scripts, archives, binary hashes and
  required scenarios; reject incomplete or failed evidence.
- [ ] Record completion of phases 1–5 before starting new product features.

**Acceptance:** local VM runs produce complete results for the recorded candidate,
and the artifact verifier checks those results against the supplied scripts and
archives.

## Phase 6 — Later coverage using the same harness

Resume from the [next-session handoff](../../dev/release-test/NEXT-SESSION.md).

- [x] Upgrade from the previous release and verify persisted data and settings.
- [ ] Container lab creation, inspection, shutdown, resume and destruction.
- [ ] Nested VM lab creation, inspection, shutdown, resume and destruction.
- [ ] Mixed VM/container labs and their networking.
- [ ] Tailscale enrollment, routing, stop/resume, isolation and cleanup using a
  disposable tailnet and explicitly supplied credentials.
- [ ] Run the existing service and crate integration suites inside suitable
  disposable guests, keeping their setup separate from the bare installer baseline.
- [ ] Add supported Ubuntu versions and architectures as separate test targets.

**Upgrade verification recorded on 2026-10-03:** published `v0.3.79` binaries
were installed and initialized in a fresh Ubuntu 26.04 guest, then upgraded to the
local `v0.3.80` archives from clean build commit `2070f3e`. Both installs used the
recorded checkout installer; the previous release's historical installer was not
tested. The harness ran from the uncommitted checkout at that same HEAD and
recorded its script hashes and dirty state separately from the archive inputs.

| Run ID | Scenario | Result | Owned VM/disks |
|---|---|---|---|
| `35a1cc067fb6487bbaa5113fbf1bc048` | `v0.3.79` → local `v0.3.80`, reboot recovery | Passed | Removed, cleanup verified |

The receipt and `upgrade-state.toml` are under
`.tmp/vm-release-tests/35a1cc067fb6487bbaa5113fbf1bc048/`. The reference is
`.tmp/vm-release-tests/upgrade-v0.3.79-to-v0.3.80.toml`.
Both versions' archive and binary hashes are recorded; installed fingerprints
match baseline inputs before upgrade and candidate inputs after upgrade and
reboot. The initialized admin and an additional API-created non-admin retained
their complete user records, credentials and privileges. Configuration,
environment assignments, SSH identity and TLS certificate fingerprints matched
across all three phases. Generated environment comments are excluded from its
fingerprint. API/CLI authentication and service/database health passed after both
upgrade and reboot. The five pre-existing host domains, including the clean
baseline, remained in the inventory, and the base image checksum was unchanged.

Two exploratory runs remain failed in their receipts, with verified cleanup:
`045bb47090ed4899ac3bf342db0076a1` exposed the generated environment timestamp
comparison, and `1c2c7acc23254bb7b7df2f637e05bdfe` encountered transient guest SSH
refusal before installation. Regression tests cover semantic environment
comparison, bounded SSH readiness, separate baseline inputs, changed/missing
upgrade evidence, timeout reporting and cleanup. Formatting, workspace Clippy
and all 45 local harness/guest/fault/verifier regression checks passed.

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
