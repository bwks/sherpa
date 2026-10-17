# Shared Crate — How to Run Tests

## Run All Tests

```bash
cargo test -p shared
```

## Run Auth Tests Only

```bash
cargo test -p shared auth::
```

## Run a Specific Module

```bash
# Password hashing
cargo test -p shared auth::password::

# JWT claims
cargo test -p shared auth::jwt::

# SSH key validation
cargo test -p shared auth::ssh::
```

## Run a Single Test

```bash
cargo test -p shared test_verify_password_correct
```

## Run with Output

```bash
cargo test -p shared -- --nocapture
```

## Prerequisites

- No external services required for auth tests (pure crypto/logic).
- Some utility tests in other modules may require network interfaces (feature-gated).

## Test Location

All tests are inline `#[cfg(test)]` modules at the bottom of each source file in `crates/shared/src/`.

## Linting

```bash
cargo fmt -p shared
cargo clippy -p shared -- -D warnings
```
