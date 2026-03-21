# Topology Crate — How to Run Tests

## Run All Tests

```bash
cargo test -p topology
```

## Run Only Integration Tests

```bash
cargo test -p topology --test manifest_tests
```

## Run a Single Test

```bash
cargo test -p topology test_parse_full_manifest
```

## Run with Output

```bash
cargo test -p topology -- --nocapture
```

## Prerequisites

- No external services required. All tests are pure TOML parsing and struct logic.
- The write/load round-trip test writes to `/tmp/` — no special permissions needed.

## Test Location

- Inline tests: `crates/topology/src/manifest.rs` (4 tests — deserialization)
- Integration tests: `crates/topology/tests/manifest_tests.rs` (13 tests)

## Linting

```bash
cargo fmt -p topology
cargo clippy -p topology -- -D warnings
```
