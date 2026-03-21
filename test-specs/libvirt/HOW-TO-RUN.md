# Libvirt Crate — How to Run Tests

## Prerequisites

- Running libvirt daemon with QEMU/KVM support
- Verify with: `virsh -c qemu:///system version`

## Run All Tests

```bash
cargo test -p libvirt -- --ignored --test-threads=1
```

## Run a Single Test

```bash
cargo test -p libvirt test_create_nat_network_ipv4 -- --ignored
```

## Cleanup Stale Resources

If tests fail midway, stale networks/pools may remain:

```bash
virsh -c qemu:///system net-list --all | grep sherpa-test
virsh -c qemu:///system net-destroy sherpa-test-xxx
virsh -c qemu:///system net-undefine sherpa-test-xxx
virsh -c qemu:///system pool-destroy sherpa-test-pool
virsh -c qemu:///system pool-undefine sherpa-test-pool
```

Tests auto-clean stale resources before each run.

## Test Location

- `crates/libvirt/tests/integration_tests.rs` (10 tests)

## Linting

```bash
cargo fmt -p libvirt
cargo clippy -p libvirt -- -D warnings
```
