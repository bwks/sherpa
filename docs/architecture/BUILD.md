# Build profiles and build speed

The profiles are in the root `Cargo.toml`, whose comments say why each setting is there; the
measurements behind them, the linker's included, are in issue #583. Nothing in `[profile.dev*]`
reaches a released binary: `release` and `dist` do not inherit `dev`.

## Crypto dependencies

The workspace builds one TLS crypto provider, aws-lc-rs, and never `ring`: `cargo tree -i ring
--workspace --target all` must print nothing, and nothing automated checks it. `async-nats` and
`rcgen` bring `ring` back with their default features, so both turn them off, and `x509-parser`
checks signatures with `verify-aws`, not `verify`. Every binary installs the aws-lc-rs provider in
`main`, so a crate that enabled `ring` again would still build; `cargo tree` is the only check.

`crates/db/build.rs` makes Cargo build Turso twice, for the build script and for the crate: the
price of a build that fails on any SQL statement that does not fit the schema (see `DATABASE.md`).
