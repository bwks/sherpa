# Sherpa Testing

This directory holds the testing principles for the project and the runbooks for tests that
need external services or a disposable VM. The tests in each crate are the specification for
that crate's behaviour.

## Testing Principles

### Red/Green/Refactor (TDD)

All test implementation should follow the Red/Green/Refactor cycle:

1. **Red** — Write a failing test first. The test defines the expected behavior before any implementation exists or changes. Run it and confirm it fails.
2. **Green** — Write the minimum code necessary to make the test pass. No more, no less. Resist the urge to over-engineer at this stage.
3. **Refactor** — Clean up the implementation and the test. Remove duplication, improve naming, simplify logic. The tests must still pass after refactoring.

This cycle applies whether you are adding new functionality, fixing a bug, or extending existing behavior. Every bug fix should start with a test that reproduces the bug.

### General Principles

- **Test behavior, not implementation.** Tests should verify what a function does, not how it does it. This makes tests resilient to refactoring.
- **One assertion per concern.** Each test should validate a single behavior or invariant. If a test name needs "and" in it, consider splitting it.
- **Tests are documentation.** A well-named test communicates intent. Someone reading the test suite should understand the system's expected behavior without reading the source code.
- **Isolate external dependencies.** Unit tests must not require Docker, libvirt, SurrealDB, or network access. Use integration tests (gated behind `#[ignore]`) for real service interactions.
- **Deterministic and repeatable.** Tests must produce the same result every run. No reliance on global state, ordering, or timing (except where explicitly testing timeouts).
- **Fast feedback.** Unit tests should run in milliseconds. Keep the unit test suite fast so it can run on every change. Integration tests are slower by nature but should still avoid unnecessary waits.
- **Test at the right level.** Use the test pyramid: many unit tests, fewer integration tests, fewest e2e tests. Push testing as low in the stack as possible.

## Runbooks

| Document | Purpose |
|---|---|
| [integration/HOW-TO-RUN.md](integration/HOW-TO-RUN.md) | Running the `#[ignore]` integration tests (db, container, network, libvirt, server) inside the `devbox_linux` VM, plus known issues. |
| [integration/vm-release-test-plan.md](integration/vm-release-test-plan.md) | Task checklist for disposable VM release testing. |
| [integration/tailscale.md](integration/tailscale.md) | Verification guide for Tailscale regressions, live enrollment, mixed VM/container acceptance, and cleanup retries. |
| [integration/current-user.md](integration/current-user.md) | Gated personal Linux VM login, sudo, resume persistence, and key rotation on owner redeploy. |
| [integration/qcow2-overlays.md](integration/qcow2-overlays.md) | Live QCOW2 overlay provisioning tests. |
| [install/HOW-TO-RUN.md](install/HOW-TO-RUN.md) | Running the installer test harness and the supplementary BATS suite. |
| [install/sherpa-install.md](install/sherpa-install.md) | What the installer and uninstaller tests cover. |
