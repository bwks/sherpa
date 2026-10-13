# DB Crate — How to Run Tests

## Prerequisites

A running SurrealDB instance is required. Use the project's test database helper:

```bash
dev/testdb start     # spin up in-memory SurrealDB v3.0.0 container
dev/testdb status    # check if it's running
dev/testdb restart   # destroy and recreate (fresh database)
dev/testdb stop      # stop the container
dev/testdb destroy   # stop and remove the container
```

Optionally set the password via environment variable:
```bash
export SHERPA_DB_PASSWORD="Everest1953!"
```

## Run All Tests

```bash
cargo test -p db -- --ignored --test-threads=1
```

Note: `--ignored` is required because all DB tests use `#[ignore]` to prevent running without a database. `--test-threads=1` avoids race conditions on shared DB state.

## Run by Entity

```bash
cargo test -p db user -- --ignored
cargo test -p db lab -- --ignored
cargo test -p db node -- --ignored --test-threads=1
cargo test -p db link -- --ignored --test-threads=1
cargo test -p db node_image -- --ignored
```

## Run by Operation

```bash
cargo test -p db lab::create_tests -- --ignored
cargo test -p db lab::read_tests -- --ignored
cargo test -p db lab::update_tests -- --ignored
cargo test -p db lab::delete_tests -- --ignored
cargo test -p db lab::auth_tests -- --ignored
```

## Run New Test Modules

```bash
cargo test -p db schema -- --ignored
cargo test -p db relationships -- --ignored --test-threads=1
```

## Run a Single Test

```bash
cargo test -p db test_delete_lab_cascade_removes_nodes_links_bridges -- --ignored
```

## Test Location

- `crates/db/tests/helper.rs` — shared setup/teardown/builders
- `crates/db/tests/{user,lab,node,link,node_image}/` — CRUD tests per entity
- `crates/db/tests/schema/` — schema idempotency and table creation
- `crates/db/tests/relationships/` — cascade deletes, query isolation

## Linting

```bash
cargo fmt -p db
cargo clippy -p db -- -D warnings
```
