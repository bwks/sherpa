# sherpa_lints (prototype)

Two [Dylint](https://github.com/trailofbits/dylint) lints written for the investigation in
[`docs/investigations/dylint.md`](../docs/investigations/dylint.md):

- `use_in_function`: a `use` statement inside a function.
- `code_in_mod_rs`: anything in a `mod.rs` other than docs, `mod` declarations and `pub use`.

This is a prototype. Nothing runs it: not CI, not the pre-commit hook, not `mise`, not the
workspace. It is its own Cargo workspace on a pinned nightly (`rust-toolchain.toml`).

With `cargo-dylint` and `dylint-link` installed:

- UI tests: `cd lints && env -u RUSTUP_TOOLCHAIN cargo test`
- Over the workspace, from the repository root:
  `cargo dylint --path lints --all -- --workspace --all-targets`
