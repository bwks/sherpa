# DB Crate — How to Run Tests

## Prerequisites

None. sherpad embeds its database (Turso, a Rust rewrite of SQLite), and every test opens its
own in-memory database with `db::open_in_memory()`, so the tests need no database server and
run in parallel with the unit tests.

## Run All Tests

```bash
cargo test -p db
```

## Run by Entity

```bash
cargo test -p db user
cargo test -p db lab
cargo test -p db node
cargo test -p db link
cargo test -p db node_image
```

## Run by Operation

```bash
cargo test -p db lab::create_tests
cargo test -p db lab::read_tests
cargo test -p db lab::update_tests
cargo test -p db lab::delete_tests
cargo test -p db lab::auth_tests
```

## Run Schema and Relationship Tests

```bash
cargo test -p db schema
cargo test -p db relationships
```

## Run a Single Test

```bash
cargo test -p db test_delete_lab_cascade_removes_nodes_links_bridges
```

## Test Location

- `crates/db/tests/helper.rs` — shared setup and builders (`setup_db()` opens an in-memory database)
- `crates/db/tests/{user,lab,node,link,node_image}/` — CRUD tests per entity
- `crates/db/tests/schema/` — schema idempotency and table creation
- `crates/db/tests/relationships/` — cascade deletes, query isolation, rollback, concurrency
- `#[cfg(test)]` modules in `crates/db/src/` — tests that need raw SQL, such as the admin
  protection triggers in `schema/user.rs`

## Linting

```bash
cargo fmt -p db
cargo clippy -p db -- -D warnings
```
