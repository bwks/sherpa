# Build profiles and build speed

How the workspace is compiled: the Cargo profiles in the root `Cargo.toml`, the linker, and
the build scripts. The measurements behind these settings are in issue #583.

## Profiles

| Profile | Used by | Workspace crates | Dependencies |
|---|---|---|---|
| `dev` | `cargo build`, `cargo clippy`, `cargo run` | `opt-level = 0`, incremental, `debug = "line-tables-only"` | `opt-level = 0`, no debug info; the hashing and bignum crates listed below at `opt-level = 3` |
| `test` | `cargo test`, `cargo nextest run` | inherits `dev` | inherits `dev` |
| `release` | `cargo build --release` | `opt-level = 3`, no debug info | same |
| `dist` | `cargo build --profile dist`, the Release workflow | inherits `release`, plus `lto = true`, `codegen-units = 1`, `strip = true` | same |

The `[profile.dev.package.*]` overrides apply to `dev` and to `test`, which inherits it.
`release` and `dist` do not inherit `dev`, so nothing in this section changes a released
binary.

### Debug info

`dev` keeps line tables for workspace crates (`debug = "line-tables-only"`), so panic messages
and backtraces show function, file and line. A debugger shows no local variables or types.
Dependencies get no debug info (`[profile.dev.package."*"] debug = false`). Full debug info
made up most of `target/`: a `dev` tree of the workspace went from 17 GB to 6.2 GB, and
rebuilding all targets after an edit to `shared` from about 2.5 min to 21 s.

For a debugging session that needs local variables in workspace code, set
`CARGO_PROFILE_DEV_DEBUG=true`. It replaces the `[profile.dev]` value only; dependencies keep
no debug info unless the `"*"` override is changed locally.

### Optimised dependencies

Argon2 password hashing and RSA key generation are very slow at `opt-level = 0`, and the
tests that seed users or generate lab SSH keys spend most of their time there. These crates
are built at `opt-level = 3` in `dev`: `argon2`, `blake2`, `password-hash`, `rsa`,
`num-bigint`, `num-bigint-dig`, `num-integer`, `num-traits`, `crypto-bigint`, `sha2`,
`digest`, `block-buffer`, `rand`, `rand_core`, `rand_chacha`, `ppv-lite86`, `subtle` and
`zeroize`. An override names a crate, not a version, so it covers every version in
`Cargo.lock`. The crates are small, so this costs no measurable build time. Optimising every
dependency (`opt-level = 1`) gave the same test time but doubled the cold build.

Cargo applies `[profile.dev.package."*"]` and then the crate's own override; the crate
overrides set only `opt-level`, so these crates are optimised and have no debug info.

## Linker

rustc 1.90 and later links `x86_64-unknown-linux-gnu` with its bundled `rust-lld` by default,
so the workspace needs no `.cargo/config.toml` or linker setting. GNU ld was much slower and
wild no faster in #583.

## Build scripts

- `crates/db/build.rs` applies the schema and migrations to a database and prepares every SQL
  statement against it, so a statement that does not fit the schema fails the build (see
  `DATABASE.md`). It makes Cargo build Turso twice, once for the build script and once for the
  crate; that cost is accepted.
- `crates/network` has no build script. It embeds the eBPF program with
  `include_bytes!("../../ebpf-redirect/ebpf-redirect.elf")`, and rustc records that file in the
  crate's dep-info, so Cargo rebuilds `network` when the checked-in ELF changes. An earlier
  build script watched `crates/ebpf-redirect/target/.../ebpf-redirect`, a file that exists only
  after building the eBPF program locally; where it did not exist, every Cargo command rebuilt
  `network` and `sherpad`.

## Tests

`cargo nextest run --workspace` runs the same tests as `cargo test --workspace`, each in its own
process. It is pinned in `mise.toml`; CI runs it through `cargo llvm-cov nextest`. See
`tests/README.md`.
