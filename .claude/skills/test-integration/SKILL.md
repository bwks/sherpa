---
name: test-integration
description: Run Rust integration tests locally (intended for use inside a devbox_linux VM with passwordless sudo)
allowed-tools: Bash
model: sonnet
---

# Integration Tests

Run integration tests locally. This skill is designed to be invoked from within a
`devbox_linux` VM that has passwordless sudo and all dependencies pre-installed
(Rust, Docker, libvirt).

## Arguments

- No argument: run all integration tests
- `<crate>`: run tests for a specific crate (`db`, `container`, `network`, `libvirt`, `server`)

## Instructions

Execute the following phases sequentially. Report progress to the user as you go.
Stop and report if any phase fails.

### Phase 1 — Format and Lint

Run formatting and linting checks. Use `--check` for fmt — report failures for the
user to fix.

```bash
cargo fmt --check
```

```bash
cargo clippy --workspace -- -D warnings
```

If either fails, report the error output and stop.

### Phase 2 — Test Execution

Run integration tests. Execute sequentially and stop on first failure.

Map the `server` crate argument to the `-p sherpad` package name. All other crate names
map directly to their `-p <name>` package.

**If a specific crate was provided**, run only that crate's tests using the appropriate
command from the table below.

**If no crate was provided**, run all crates in this order:

| Order | Crate | Command |
|-------|-------|---------|
| 1 | db | `cargo nextest run -p db --run-ignored ignored-only -j 1` |
| 2 | container | `cargo nextest run -p container --run-ignored ignored-only -j 1` |
| 3 | network | `sudo -E env PATH="$HOME/.cargo/bin:$PATH" cargo nextest run -p network --run-ignored ignored-only -j 1` |
| 4 | libvirt | `cargo nextest run -p libvirt --run-ignored ignored-only -j 1` |
| 5 | server | `sudo -E env PATH="$HOME/.cargo/bin:$PATH" cargo nextest run -p sherpad --run-ignored ignored-only -j 1` |

Note: `network` and `server` require sudo for network capabilities.

### Phase 3 — Report

Present a clear summary of results:

- List each crate tested with PASS or FAIL status
- If any crate failed, show the relevant test output
- Show total pass/fail count
