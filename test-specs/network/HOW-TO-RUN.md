# Network Crate — How to Run Tests

## Prerequisites

- Root privileges or CAP_NET_ADMIN capability
- Run inside the test-runner VM: `sherpa up --manifest manifest-test-runner.toml`

## Run All Tests

```bash
sudo -E cargo test -p network -- --ignored --test-threads=1
```

`sudo -E` preserves environment variables. `--test-threads=1` avoids interface name collisions.

## Run a Single Test

```bash
sudo -E cargo test -p network test_create_bridge -- --ignored
```

## Cleanup Stale Interfaces

If tests fail midway, stale interfaces may remain:

```bash
ip link show | grep "st-"
sudo ip link del st-br0
sudo ip link del st-veth0a
```

Tests auto-clean stale interfaces before each run.

## Test Location

- `crates/network/tests/integration_tests.rs` (8 tests)

## Linting

```bash
cargo fmt -p network
cargo clippy -p network -- -D warnings
```
