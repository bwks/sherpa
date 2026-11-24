# sherpa_lints (prototype)

[Dylint](https://github.com/trailofbits/dylint) lints for Sherpa's own code rules, written for
the evaluation in GitHub issue 936 of `bwks/sherpa`, where the results are.

| Lint | Rule | Kind |
|---|---|---|
| `error_text_match` | No decision is taken from an error's text | Late (types) |
| `error_text_match_in_test` | The same, in test code | Late (types) |
| `field_comment` | Struct fields and enum variants carry no comments, except on clap types | Late (types), reads tokens |
| `version_history` | Comments and string literals carry no version-history narrative | Early, reads tokens |
| `single_use_closure` | A closure is not bound to a name to be used once | Late (name resolution) |
| `use_in_function` | A `use` statement is never declared inside a function | Early (syntax) |
| `code_in_mod_rs` | A `mod.rs` holds only docs, `mod` declarations and `pub use` | Early (syntax) |

This is a prototype. Nothing runs it: not CI, not the pre-commit hook, not `mise`, not the
workspace. It is its own Cargo workspace on a pinned nightly (`rust-toolchain.toml`), built
against the `clippy_utils` revision in `Cargo.toml`. The two are a pair: `cargo dylint upgrade
lints` moves both.

## Tools

- `cargo-dylint` 6.4.0 and `dylint-link` 6.4.0:
  `mise install cargo:cargo-dylint@6.4.0 cargo:dylint-link@6.4.0`, or `cargo install` of the same.
- `nightly-2026-08-20` with `rustc-dev` and `llvm-tools-preview`, which rustup installs from
  `rust-toolchain.toml` on first use.

## Running

- UI tests: `cd lints && env -u RUSTUP_TOOLCHAIN cargo test`. Each `ui/<lint>.rs` is compiled
  with the lints loaded and its diagnostics are compared with `ui/<lint>.stderr`. The stand-in
  `anyhow` and `clap` crates in `ui/auxiliary/` let the tests name those crates' types without
  depending on them.
- Over the workspace, from the repository root:
  `cargo dylint --path lints --all -- --workspace --all-targets`

`env -u RUSTUP_TOOLCHAIN` is needed where the environment sets `RUSTUP_TOOLCHAIN`, which
overrides `rust-toolchain.toml`. `--all-targets` matters: without it the lints never see test
code.
