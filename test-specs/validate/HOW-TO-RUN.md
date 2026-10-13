# Validate Crate — How to Run Tests

## Run All Tests

```bash
cargo test -p validate
```

## Run Tests for a Specific Module

```bash
# Connection tests
cargo test -p validate connection::

# Device duplicate detection
cargo test -p validate device::

# Environment variable validation
cargo test -p validate environment::

# IPv6 validation
cargo test -p validate ipv6::

# Link and bridge validation
cargo test -p validate link::

# Node image field validation
cargo test -p validate node_image::

# Version resolution and image existence
cargo test -p validate version::
```

## Run a Single Test

```bash
cargo test -p validate test_validate_node_image_update_all_valid
```

## Run with Output (see println/debug)

```bash
cargo test -p validate -- --nocapture
```

## Prerequisites

- No external services required. All tests are pure unit tests.
- Exception: `tcp_connect` tests hit localhost ports — no setup needed but results depend on what's listening.

## Test Location

All tests are inline `#[cfg(test)]` modules at the bottom of each source file in `crates/validate/src/`.

## Linting

Always run after making changes:

```bash
cargo fmt -p validate
cargo clippy -p validate -- -D warnings
```
