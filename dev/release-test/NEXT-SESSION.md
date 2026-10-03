# Next session: Stage 6 container lifecycle testing

Updated on 2026-10-03 after implementing and verifying the Stage 6 upgrade
scenario. The next unchecked scenario is container lifecycle coverage.

## Scope and restrictions

- Run tests locally against the existing Sherpa server, using the existing CLI
  authentication and SSH access. Do not introduce another account or runner VM.
- Do not modify `.github/workflows`, run these tests in CI, or add runner services.
- Publication gates and publication tooling are out of scope. The user explicitly
  removed that work. Do not recreate the deleted publisher, publication config or
  associated guide, or restore the removed publication tasks.
- Keep developer testing instructions in this source repository. The separate
  user-facing documentation repository is not the place for test instructions.
- Use the current branch. Do not create a branch, commit, push or publish without
  a new request.
- Read `AGENTS.md` before working. Run `cargo fmt`, then
  `cargo clippy --workspace -- -D warnings`, and require both to pass before tests.
- Preserve unrelated workloads. Installation and removal operations belong inside
  fresh disposable test guests; destroy test labs when finished.
- Manage test VM lifecycle through Sherpa commands (`up`, `inspect`, `destroy`).

## Completed upgrade scenario

The `v0.3.79` to locally built `v0.3.80` upgrade passed on a fresh Ubuntu 26.04
VM. Both installs used the recorded checkout installer, with separate baseline
and candidate archives. This tests the binary upgrade with that installer, not
the previous release's historical installer.

The runner now supports `--scenario upgrade`, an explicit `--baseline-version`,
optional `--baseline-artifact-dir`, and an optional TOML `[baseline]` section.
Local candidate archives are required. Equal versions and identical binary bytes
are rejected before provisioning. Upgrade guests pass a bounded read-only SSH
readiness wait before workspace creation and the existing identity guards.

The scenario initializes the baseline, creates an additional non-admin user,
upgrades without reinitialization, verifies both users and retained state, reboots
and repeats verification. Environment fingerprints compare assignments rather
than generated comments. Collected evidence must include matching retained state
and installed binary fingerprints for all three phases before a run can pass.

## Start with container lifecycle coverage

1. Read the [checklist](../../test-specs/integration/vm-release-test-plan.md),
   [VM guide](README.md), [configuration](config.toml) and
   [installer test specification](../../test-specs/install/sherpa-install.md),
   plus the [lab lifecycle specification](../../test-specs/integration/lab-lifecycle-e2e.md)
   and [container lifecycle specification](../../test-specs/container/lifecycle.md).
2. Inspect the existing host controller and guest assertions in
   `scripts/vm_release_test.py` and `scripts/vm_release_guest.py`. The entry point
   is `scripts/test_install.sh`. Extend the existing harness.
3. Define acceptance criteria and write failing local regression tests before
   implementation, following Red/Green/Refactor. Select a suitable container model
   and image using existing project conventions; do not add dependencies without
   authorization.
4. Install and initialize the explicit candidate in a fresh disposable guest.
   Test container lab creation, inspection, shutdown, resume and destruction through
   Sherpa, including persisted state, networking and resource ownership assertions.
5. Collect diagnostics and evidence, verify cleanup of only that run's resources,
   and mark checklist completion only after the real scenario passes.

After container coverage, the checklist order is nested VM lifecycle, mixed
labs/networking, Tailscale with explicit disposable credentials, service/crate
integration suites and additional operating systems/architectures.

## Repository state to preserve

At handoff, the branch is `v0.3.80`, tracking `origin/v0.3.80`, at
`2070f3ee77d36dc8c74f39a1c6d4a5eaeb4a22af`:
`test: verify release cleanup and failure handling in VMs`.
All 11 crate versions are `0.3.80`.

The working tree preserves the earlier publication-task removal and local
verification wording, plus the upgrade harness implementation and documentation:

- `dev/release-test/README.md`
- `scripts/README.md`
- `scripts/TESTING.md`
- `scripts/verify_release_test.py`
- `scripts/vm_release_test.py`
- `scripts/vm_release_guest.py`
- `test-specs/install/sherpa-install.md`
- `test-specs/integration/vm-release-test-plan.md`

The verifier's messages, help text and test names changed; its lifecycle validation
behavior was retained. The controller and guest helper now implement upgrade
coverage. Formatting, workspace Clippy and all 45 local regression tests passed.
No workflow edits remain. `CLAUDE.md` is also deleted in the
working tree; that deletion was not made by this agent. Preserve existing edits.
This handoff and its checklist link are additional uncommitted documentation.

Stage 5 is now local validation/documentation. Its remaining unchecked item is
recording completion of phases 1–5. Do not interpret that bookkeeping item as a
request to restore publication work. The user explicitly chose Stage 6 next.

## Existing local evidence

The final upgrade run is `35a1cc067fb6487bbaa5113fbf1bc048`, passed with verified
cleanup. Its receipt and `upgrade-state.toml` are under that run's directory in
`.tmp/vm-release-tests/`. The stable reference is
`.tmp/vm-release-tests/upgrade-v0.3.79-to-v0.3.80.toml`.
The evidence SHA-256 is
`bf6a250bb9957792f6644a265bdaf4bda5a24255fd09b74281d0afa76e5af6f9`.
Current script hashes, the candidate build hashes, all three retained-state/binary
phases, cleanup and the unchanged host domain inventory were rechecked after the
run. The source checkout was dirty; the archive build at `2070f3e` was clean.

Earlier upgrade attempts `045bb47090ed4899ac3bf342db0076a1` and
`1c2c7acc23254bb7b7df2f637e05bdfe` remain failed, with diagnostics and verified
cleanup. They exposed the timestamp comparison and transient SSH refusal,
respectively; both have regression coverage. All three owned guests were removed.

The committed harness has already passed fresh installation, initialization,
authenticated API/CLI access, reboot, repeat installation, retention/reinstallation,
all uninstall modes and resource retention checks. Timeout, SIGTERM interruption
and incomplete-provisioning scenarios also passed their expected failure and
verified cleanup assertions. Earlier full workspace tests and 34 harness
regression checks passed.

A further clean-checkout verification was completed at commit `2070f3e`. Its local
evidence still exists, although it has not yet been added to the checklist's
historical run table:

- Candidate archives: `.tmp/release-artifacts/v0.3.80-2070f3e/`, containing the
  normal Linux x64 `sherpa` and `sherpad` tarballs.
- Lifecycle reference: `.tmp/vm-release-tests/clean-v0.3.80-2070f3e.toml`.
- Lifecycle receipt:
  `.tmp/vm-release-tests/14e2fab943554ba7b32f12c03459579a/result.toml`.
- Combined failure results:
  `.tmp/vm-release-tests/failure-checks/8ed627ee4ab44afe8224002fbca9b894/fault-results.toml`.
- Failure run IDs: timeout `c6cf78804b564dc79033d513bbae99ac`, interruption
  `29f56a4ddfaf45d398d444a1a83a651e`, incomplete provisioning
  `ffbaac1a6ae342b6ba02846f0210727a`.

These four runs recorded the clean source commit; all their owned test VMs were
removed with verified cleanup. Their results and reference files were checked
while writing this note. Strict local artifact verification previously passed
without `--allow-dirty`. Recheck evidence against current inputs before relying on
it for a different candidate. Files under `.tmp/` are ignored and may disappear.

## Environment reminders

The image is official Ubuntu 26.04 amd64, build `20260927`. Its location and
checksum are configured in `config.toml`; guest resources are in `manifest.toml`.
The original clean baseline lab `f804c420`, domain `testbox-f804c420`, was destroyed
on 2026-10-03 at the user's request using `sherpa destroy --yes` from
`dev/release-test`. Sherpa reported removal of its VM, both disks, router,
Docker/libvirt networks and lab records; subsequent `sherpa inspect` returned
`Lab not found: f804c420`. Do not assume that baseline VM still exists or retain
another baseline indefinitely. The manifest and verified base image remain
available for fresh disposable guests. Preserve unrelated labs and recheck current
server state through Sherpa before resource operations.

The default runner configuration selects published `v0.3.79` binaries. Use an
explicit version and artifact directory for candidate tests. The existing ignored
`.tmp/release-artifacts/v0.3.80-2070f3e/runner-config.toml` contains the prior local
connection override; inspect it locally and keep connection settings and
credentials out of committed notes. Do not assume ignored configuration or
authentication is still available next session.
The private `.tmp/vm-release-tests/upgrade-runner-config.toml` contains the latest
connection override and explicit candidate/baseline inputs. The successful run
used cached published baseline archives retained under the first failed run's
`baseline/` directory; its recorded hashes match the original downloads.
Inspect ignored settings locally and recheck them before reuse. Upgrade receipts
are separate from install/uninstall lifecycle receipts; the lifecycle verifier
does not accept them as a substitute for the complete lifecycle suite.
