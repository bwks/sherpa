---
name: version-bump
description: "Bump the version of all workspace crates. Optional arg: target version (e.g. /version-bump 0.4.0)"
allowed-tools: Bash, Grep
model: haiku
---

# Version Bump

Bump the version of all workspace crates in the `crates/` directory.

If the user provides a target version as an argument, use that version directly. Otherwise, increment the patch number of the current version.

## Steps

### 1. Find current version

Run:

```
grep -rn '^version = ' crates/*/Cargo.toml
```

All crates should be on the same version. If they are not, report the discrepancy and stop.

### 2. Determine new version

- If the user provided a target version argument: use that version.
- Otherwise: increment the patch number (e.g. `0.3.57` becomes `0.3.58`).

### 3. Bump the version

Use `sed -i` to replace the old version with the new version across all `crates/*/Cargo.toml` files in a single command.

The `ebpf-redirect` crate is outside the workspace, so cargo does not rewrite its lockfile.
Also update the `ebpf-redirect` package entry in `crates/ebpf-redirect/Cargo.lock` to the new
version, then run `cargo check --workspace` so the root `Cargo.lock` picks up the new version.

### 4. Verify

Run the same grep to confirm all crates now show the new version, and confirm `git diff --stat`
shows both lockfiles changed.

### Report

Print the old version, the new version, and the number of crates updated.
