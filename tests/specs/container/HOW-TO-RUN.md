# Container Crate — How to Run Tests

## Prerequisites

- Running Docker daemon
- `alpine:latest` image pulled: `docker pull alpine:latest`

## Run All Tests

```bash
cargo test -p container -- --ignored --test-threads=1
```

`--test-threads=1` is required to avoid port/name collisions between tests.

## Run a Single Test

```bash
cargo test -p container test_run_and_remove_container -- --ignored
```

## Cleanup Stale Resources

If a test run fails midway, stale containers/networks may be left behind:

```bash
docker ps -a --format "{{.Names}}" | grep "sherpa-test-" | grep -v "sherpa-test-db" | xargs -r docker rm -f
docker network ls --format "{{.Name}}" | grep "sherpa-test" | xargs -r docker network rm
```

Tests auto-clean stale resources before each run, but manual cleanup may be needed after crashes.

## Test Location

- `crates/container/tests/integration_tests.rs` (14 tests)

## Linting

```bash
cargo fmt -p container
cargo clippy -p container -- -D warnings
```
