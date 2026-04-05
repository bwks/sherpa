---
name: qc
description: Run formatting, linting, and tests for the entire workspace
allowed-tools: Bash
---

# Quality Check

Run formatting, clippy, and tests across all workspace crates.

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

### 3. Tests

```
cargo test --workspace
```

### Report

Summarise the result of each step (pass/fail). If any step failed, show the relevant error output.
