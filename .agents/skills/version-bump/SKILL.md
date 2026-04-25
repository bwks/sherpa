---
name: version-bump
description: Bump the version of all workspace crates. Use when the user asks to bump crate versions, optionally to a target version such as 0.4.0.
allowed-tools: bash
---

# Version Bump

Bump the version of all workspace crates in the `crates/` directory.

If the user provides a target version as an argument, use that version directly. Otherwise, increment the patch number of the current version.

## Steps

### 1. Find current version

Run:

```bash
rg -n '^version = ' crates/*/Cargo.toml
```

All crates should be on the same version. If they are not, report the discrepancy and stop.

### 2. Determine new version

- If the user provided a target version argument: use that version.
- Otherwise: increment the patch number, for example `0.3.57` becomes `0.3.58`.

### 3. Bump the versions

Update every `crates/*/Cargo.toml` package version from the old version to the new version.

Also update workspace crate package versions in:

- `Cargo.lock`
- `crates/ebpf-redirect/Cargo.lock`, if present

Do not run `cargo generate-lockfile` unless necessary, because it may update unrelated third-party dependency versions.

### 4. Verify

Run:

```bash
rg -n '^version = ' crates/*/Cargo.toml
rg -n '^version = "<old-version>"' crates Cargo.lock
```

Confirm all workspace crates now show the new version and no workspace crate lockfile entries still use the old version.

### Report

Print the old version, the new version, and the number of crates updated.
