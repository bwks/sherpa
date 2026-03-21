# Template Crate — How to Run Tests

## Run All Tests

```bash
cargo test -p template
```

## Run Only Render Tests (integration)

```bash
cargo test -p template --test render_tests
```

## Run Only Nokia SR Linux Tests (inline)

```bash
cargo test -p template nokia_srlinux::
```

## Run a Single Test

```bash
cargo test -p template test_arista_veos_renders
```

## Run with Output

```bash
cargo test -p template -- --nocapture
```

## Prerequisites

- No external services required. All tests are pure rendering/serialization tests.
- Tests verify templates render without error and output contains expected content.

## Test Location

- Inline tests: `crates/template/src/nokia_srlinux.rs` (2 tests)
- Integration tests: `crates/template/tests/render_tests.rs` (40 tests)

## Linting

```bash
cargo fmt -p template
cargo clippy -p template -- -D warnings
```
