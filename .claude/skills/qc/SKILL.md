---
name: qc
description: Run formatting, linting, tests, and coverage for the entire workspace
allowed-tools: Bash
---

# Quality Check

Run formatting, clippy, tests, and coverage across all workspace crates.

## Steps

Run these steps sequentially. Stop and report if any step fails.

### 1. Format

```
cargo fmt
```

### 2. Clippy

```
cargo clippy --workspace -- -D warnings
```

### 3. Tests with Coverage

Run tests via `cargo-llvm-cov` with `cargo-nextest` to produce a coverage summary:

```
cargo llvm-cov nextest --workspace
```

This runs all workspace tests through nextest and prints a per-file coverage table to stdout.

### Report

Summarise the result of each step (pass/fail). If any step failed, show the relevant error output.

For coverage, show the summary table and highlight crates or files with notably low coverage.
