# Dylint for Sherpa's own lint rules

> Status: investigation, 2026-11-21, for #845. Nothing here is adopted. A prototype lint library
> is in [`lints/`](../../lints/); nothing runs it in CI, in the pre-commit hook, in `mise` or in
> the workspace. The recommendation is at the end: **do not adopt Dylint now**. The decision is
> the user's; once taken, the lasting part moves to `docs/architecture/`.

## Question

Many code rules in `AGENTS.md` are kept by review or by two grep scripts in `dev/`. #845 asks
whether [Dylint](https://github.com/trailofbits/dylint), which runs lints loaded from dynamic
libraries, should enforce them, what that costs, and how it compares with Clippy settings, the
scripts and structural search (`ast-grep`, `semgrep`).

## What was done

- Read Dylint's README, "How Dylint works", changelog, issue list and library template, and
  the crates.io release dates.
- Installed `cargo-dylint` and `dylint-link` 6.4.0 and `nightly-2026-08-20` in one guest.
- Wrote two lints with UI tests in `lints/` and ran them over the whole workspace, cold and warm.
- Ran `cargo dylint upgrade` on a copy of the library to see what a toolchain update breaks.
- Ran the Clippy lints that could cover rules (`absolute_paths`,
  `arbitrary_source_item_ordering`, `items_after_test_module`) over the workspace with a
  temporary `clippy.toml`.
- Not done: `ast-grep` and `semgrep` were not installed or run (the task allowed only the Dylint
  tools), so what is said about them comes from how they work, not from a trial. No account from
  another project of running Dylint in CI was found; the breakage evidence is Dylint's own
  changelog and issues, and the upgrade tried here.

The guest has 12 cores and 31 GB of memory. All timings are wall-clock on it.

## How Dylint works

- **Version:** 6.4.0, released 2026-11-10. Twelve releases since 6.0.0 on 2026-05-11.
- **Tools:** `cargo-dylint` (the `cargo dylint` command) and `dylint-link` (a linker wrapper),
  both installed with `cargo install`.
- **A lint library** is a `cdylib` crate that uses `#![feature(rustc_private)]` and links
  `rustc`'s own crates (`rustc_ast`, `rustc_lint`, ...), usually with Clippy's `clippy_utils` as
  a git dependency. `dylint-link` names the built file after its toolchain:
  `libsherpa_lints@nightly-2026-08-20-x86_64-unknown-linux-gnu.so`.
- **Loading:** `cargo dylint` finds libraries from `[workspace.metadata.dylint] libraries` in the
  workspace's `Cargo.toml` (path or git entries), or from `--path`/`--git` on the command line.
  It builds each in release mode under `target/dylint/libraries/<toolchain>/`, builds one driver
  per toolchain under `~/.dylint_drivers/`, and runs `cargo check` over the workspace with that
  driver, into `target/dylint/`.
- **The pinned nightly compiles the workspace.** The check is run with the library's toolchain,
  not the workspace's. Here the workspace (stable 1.100.0) was compiled by
  `rustc 1.100.0-nightly (2026-08-19)`. It compiled. It stops compiling the day the workspace
  or a dependency uses something newer than the pinned nightly has, and then the library must be
  upgraded before the lint run works again.
- **Which nightly:** `cargo dylint new` (6.4.0) pins `nightly-2026-08-20` with components
  `rustc-dev` and `llvm-tools-preview`, and `clippy_utils` at git revision `21baba9`. The two
  move together: each `clippy_utils` revision builds with one nightly. `cargo dylint upgrade`
  rewrites both; on 2026-11-21 it moved them to `nightly-2026-11-12` and revision `2dd7115`.
- **Tests:** `dylint_testing::ui_test` compiles each `ui/*.rs` file with the lint loaded and
  compares the compiler's output with the `.stderr` file beside it (`compiletest_rs`).

## Toolchain breakage

`rustc`'s internal crates and `clippy_utils` promise no stable API. Evidence of how often that
bites:

- **Writing the prototype.** Lint registration written from memory of the documented API did not
  compile on `nightly-2026-08-20`: `LintStore::register_early_pass` is now
  `register_early_lint_pass` and takes a boxed factory. Two compile errors, a few minutes.
- **Upgrading the prototype.** `cargo dylint upgrade` (23 s) moved a copy of the library 12
  weeks forward, to `nightly-2026-11-12`. It no longer compiled: `declare_lint!` and
  `declare_lint_pass!` are no longer exported by `rustc_session` (now `rustc_lint_defs`), and
  `declare_lint!` now needs `#![feature(default_field_values)]` in the calling crate. Three
  edits, about ten minutes, then both UI tests passed unchanged. The lints' own logic
  (`rustc_ast` items, spans, the source map) did not break. A library with one lint that uses
  `dylint_linting`'s `declare_early_lint!` is shielded from this pair of changes; the prototype
  holds two lints in one library and registers them itself, so it met them.
- **Dylint's changelog** for 6.1.0 and 6.1.1 (2026-09 and 2026-10) records the same kind of
  change on Dylint's side: the rename of `rustc_driver::run_compiler`, the move of
  `declare_lint!`, new lint-registration APIs, and the removal of `rustc`'s `--env-set` option
  (issue 2078). Issue 2146: Clippy renamed its default branch and `cargo dylint upgrade` broke
  until 6.3.0. Dylint 4.0.1 (2025) had to follow a change in Cargo's file hashing.
- **bevy_lint's** upgrade guide (a linter built on the same `rustc` and `clippy_utils` APIs)
  upgrades with each Rust release and notes that `clippy_utils` has no changelog, so the git
  history is what you read.

What keeping it working costs: an upgrade (toolchain, `clippy_utils` revision, fixes, a new
`cargo-dylint` when Dylint itself had to follow `rustc`) at least whenever the workspace
outgrows the pinned nightly, so up to once per stable release, every six weeks. For two small
syntax lints each upgrade is minutes to an hour. It grows with the number of lints and with how
deep they reach into `rustc` (type-checked lints use far more of the unstable API than these
two).

**Stable toolchain and `mise`:** the workspace stays on stable; the library carries its own
`rust-toolchain.toml`, as `crates/ebpf-redirect` does, and `mise` already follows such files.
Adoption adds a second nightly to install (2.4 GB on disk) and `cargo-dylint` and `dylint-link`
to `mise.toml`. One trap met here: this guest's environment sets `RUSTUP_TOOLCHAIN=stable`,
which overrides `lints/rust-toolchain.toml`, so building or testing the library directly needed
`env -u RUSTUP_TOOLCHAIN`. `cargo dylint` itself was not affected.

## Fit per rule

"Exact" means a check that reports every breach and nothing else in the code it sees. A Dylint
lint sees only code compiled for the host and the enabled features: nothing behind
`cfg(windows)`, and nothing in `crates/ebpf-redirect`.

| Rule (`AGENTS.md`) | Clippy today | Dylint lint | Grep or structural search | Best fit |
|---|---|---|---|---|
| No `.unwrap()`/`.expect()` in production code | `unwrap_used`, `expect_used` with `allow-unwrap-in-tests`/`allow-expect-in-tests`: already enforced by `#![deny]` in each crate root and `clippy.toml` | Nothing to add | Not needed | **Clippy** (done) |
| No `allow`/`expect` attributes outside the allowlist | `allow_attributes` refuses `#[allow]` in workspace crates; nothing refuses `#[expect]` | Possible for compiled code, but misses `crates/ebpf-redirect` and `cfg`-ed out code, and the "listed exception no longer exists" check does not fit a per-crate lint | `dev/check-lint-exceptions.sh` does all of it | **Keep the script** |
| Templates decide nothing from an enum's text | None: `.jinja` files are not Rust | Cannot see templates; the Rust that Askama generates has no usable spans | `dev/check-template-text-decisions.sh` | **Keep the script** |
| `use` only at the top of a file, never in a function | None. `items_after_statements` (pedantic, on) only catches a `use` after a statement | **Exact. Prototyped:** 6 findings, all true | Grep guesses badly: 959 indented `use` lines, of which 6 are breaches. Structural search can match a `use` inside a block exactly, and sees `cfg`-ed out code too | Lint or structural search |
| No long qualified paths inline | `absolute_paths` (restriction) with `absolute-paths-max-segments`. **Tried** with 3: 47 findings | Would re-implement the Clippy lint | Not needed | **Clippy setting** |
| `const` after `use`; structs and enums after `const` | `arbitrary_source_item_ordering` (restriction) with `module-item-order-groupings`. **Tried:** 238 findings in 114 files. Stricter than the rule: every item kind must be put in an ordered group, so a struct after an `impl` is reported too | Exact to the rule's letter is possible (order of first `use`, `const`, type items only) | Grep cannot; structural search poorly (ordering is not a pattern) | **Clippy setting**, if the stricter order is acceptable; else review |
| No code in `mod.rs` | None fits: `mod_module_files` bans `mod.rs` files altogether, `self_named_module_files` does the opposite | **Exact. Prototyped:** 321 findings in 30 files | A grep for item keywords in column 0 of `mod.rs` files found the same 30 files. Top-level items in rustfmt-formatted files are easy for grep | Script; a lint adds little |
| `#[instrument]` on public functions that do I/O, with `skip`/`fields` | None | Not exact. "Does I/O" needs a call-graph judgement; a lint could only apply a stand-in such as "every `pub async fn` in `db`, `libvirt`, `container`, `network`", which is a different rule. The right `skip`/`fields` are judgement | No | **Review** |
| Doc comments carry no code blocks | None (Clippy's doc lints check code blocks, none refuses them) | Exact (doc comments are attributes; parse the Markdown as Clippy does) | Grep for a fence in a `///` or `//!` line is exact for fenced blocks and misses only indented blocks | Script |
| A string containing `"` is a raw string | None (`needless_raw_strings` and `needless_raw_string_hashes` are the opposite direction) | Exact (string literal tokens) | Grep guesses (multi-line strings, comments, char literals); structural search can match an escape inside a string literal node | Lint or structural search |
| No GitHub issue numbers in code comments | None | Comments are not in the syntax tree; a lint would re-read the source text, which is grep in a lint's clothes | Grep, and it also covers scripts, Python and templates | **Script** |
| Unit tests at the end of the file | `items_after_test_module` (style, on by default): **already enforced**, 0 findings | Nothing to add | Not needed | **Clippy** (done) |
| Test names are spec case IDs, with the skip list | None | Could check a Rust test's name shape, but must read `tests/specs/*.md` for the known IDs and the skip list, and cannot see the pytest, JavaScript and bats tests | The check script `tests/specs/README.md` already plans covers every language | **Script** (as planned) |

Count: of thirteen rules, two are already enforced by Clippy, two more can be by a Clippy
setting, five belong to scripts (two exist, one is planned), one stays with review. That leaves
**three** where a real syntax tree beats grep: `use` in a function, raw strings, and (barely)
`mod.rs` and doc code blocks. None of them needs types, which is the one thing Dylint has that
structural search does not.

## Prototype

[`lints/`](../../lints/) is one library, `sherpa_lints`, with its own `[workspace]` table, so it
is outside the Sherpa workspace without a change to the root `Cargo.toml`. Both lints are early
(syntax) passes and skip code written by a macro.

- **`use_in_function`** reports a `use` statement inside any block: a function body, a closure,
  a nested block. A `use` in a module, inline or not, passes.
- **`code_in_mod_rs`** reports every item in a file named `mod.rs` that is not `mod name;` or a
  `use` with a visibility (`pub use`, `pub(crate) use`). A private `use`, an inline
  `mod name { }`, and every `fn`, `const`, `struct`, `enum` and the rest are reported.

Each has a UI test (`lints/ui/`): five cases reported and two not for `use_in_function`; a
`mod.rs` with one of each refused kind and a `mod.rs` that passes for `code_in_mod_rs`. The UI
harness compiles a test file as edition 2015 unless told otherwise.

To run them (in a machine that has the tools):

- UI tests: `cd lints && env -u RUSTUP_TOOLCHAIN cargo test`
- Workspace: `cargo dylint --path lints --all -- --workspace --all-targets`

`--all-targets` matters: without it the lints never see test code, and every
`use_in_function` finding below is in test code.

### What they report

327 findings over the workspace, all warnings.

**`use_in_function`: 6, all true, no false positives.** All six are
`use std::os::unix::fs::PermissionsExt;` as the first line of a test in a `#[cfg(test)]` module:

- `crates/client/src/lab_files.rs:270`, `:287`
- `crates/client/src/lab_folder_migration.rs:437`, `:547`
- `crates/client/src/token.rs:130`
- `crates/shared/src/util/file_system.rs:417`

They are there for a reason worth a decision: the import only exists on Unix, and the client's
tests also build on Windows, so at the top of the file it would need its own `#[cfg(unix)]`.

**`code_in_mod_rs`: 321 in 30 files, none misreported.** By kind: 163 functions, 113 private
`use` statements, 43 constants, 2 enums.

- `crates/client/src/cmd/server/mod.rs`: 11. Real code in a source `mod.rs`: `OutputFormat`,
  `ServerCommands`, `run_server` and their imports. The one plain breach of the rule in `src/`.
- `crates/template/tests/*/mod.rs`: 296 in 27 files. Integration tests written in `mod.rs`
  files.
- `crates/db/tests/relationships/mod.rs`: 13. The same.
- `crates/db/tests/mod.rs`: 1, a private `use helper::{...}`. This file is the root of a test
  target that happens to be named `mod.rs`.

Whether those are breaches depends on what the rule means, which the lint cannot decide:

- **Does the rule cover `tests/` trees?** 310 of the 321 findings are there.
- **Is a private `use` code?** The rule's text allows only `pub use`. Every private `use`
  reported sits beside other code except the one in `crates/db/tests/mod.rs`.

**What the lints cannot see (false negatives):**

- Code compiled out on Linux: `crates/client/src/windows_acl.rs` and the other `cfg(windows)`
  code. It has indented `use` lines that no Linux run checks.
- `crates/ebpf-redirect`: its own toolchain and target; the run does not include it.
- Code a macro writes, by design.

### Timings

| Step | Time |
|---|---|
| `cargo install cargo-dylint dylint-link` (from source, 6.4.0) | 4m34s (a Clippy run shared the machine) |
| First debug build of the library: downloads the nightly, builds 125 dependencies | 3m38s |
| UI tests, first run (builds the driver into `~/.dylint_drivers`) | 2m37s |
| UI tests, afterwards | 3.3s |
| **Workspace run, cold** (`--workspace --all-targets`) | **7m50s**: 1m38s release build of the library, 6m06s check |
| Workspace run, warm, nothing changed | 2.8s |
| Workspace run, warm, after touching `crates/client/src/main.rs` | 7.2s |
| Workspace run, warm, after touching `crates/shared/src/lib.rs` | 1m07s |
| For comparison: `cargo clippy --workspace --all-targets`, cold, fresh target folder | 7m53s |
| For comparison: the pre-commit hook today (two scripts) | 1.8s |

Disk: the nightly toolchain 2.4 GB, `target/dylint` 3.4 GB, the driver 43 MB.

A cold Dylint run costs the same as a cold Clippy run, because it is one: a second full check of
the workspace and its dependencies, by a different compiler, into its own target folder.

## Where the lints would live

In this repository, in `lints/`. A lint and the rule it enforces change together, the library
is small, and `--path`/workspace metadata loads it without publishing anything. A separate
repository only adds a git revision to bump. Adoption would add
`[workspace.metadata.dylint] libraries = [{ path = "lints" }]` to the root `Cargo.toml` so that
`cargo dylint --all` finds it; the prototype uses `--path lints` to leave the root untouched.

## CI and the hook

- **CI:** a job of its own beside Lint, not a step in it, so it adds no wall-clock time to the
  pull request: install the pinned nightly with `rustc-dev`, install `cargo-dylint` and
  `dylint-link` (prebuilt, or 4 to 5 minutes from source), run the UI tests, run the workspace
  check with `-D warnings`. Uncached it is a second Clippy-sized run plus the library build.
  Cached it needs its own cache entry of about 3.4 GB before compression, against a repository
  cache limit that `pr.yml` already works around. GitHub's runners have fewer cores than this
  guest, so expect longer than the figures above; not measured.
- **Pre-commit hook:** no. The hook takes 1.8 s today. Dylint takes 3 s when nothing changed but
  over a minute after a change to `shared`, and 8 minutes cold.
- **Windows and macOS client jobs:** not needed for correctness of the rules, which are about
  source layout. Running there would only add the `cfg(windows)` and `cfg(macos)` code to what
  is checked, at the price of the nightly, the tools and a library build on two more runners. A
  text-based check has no such blind spot in the first place.

## Tests

Lints are tested with Dylint's UI tests in `lints/ui/`, run by `cargo test` in `lints/`. They
test build tooling, not Sherpa: they prove no case of a behaviour, carry no case ID, and would
be added to "Tooling outside the specs" in `tests/specs/README.md`, as the `crates/db/build`
tests are.

## Replacing the scripts

No. Neither `dev/` script should become a lint:

- `check-template-text-decisions.sh` reads `.jinja` files, which no Rust lint sees.
- `check-lint-exceptions.sh` covers `crates/ebpf-redirect`, covers code compiled out on the
  host, and fails on a stale allowlist entry. A lint would lose all three.

## Alternatives

| | Exactness | Cost to run | Upkeep | Blind spots |
|---|---|---|---|---|
| **Clippy settings** | Exact, maintained upstream | None extra: same run | A line in `clippy.toml` or `[workspace.lints.clippy]` | Only what Clippy has: 4 of the 13 rules. Host `cfg` only, but the Windows and macOS jobs already run Clippy |
| **`rustc` `-W` lints** | Exact | None extra | None | No `rustc` lint covers any rule in the list |
| **Grep scripts** | Exact for line-shaped rules (attributes, `mod.rs` top-level items, doc fences, comments, non-Rust files); guesses for anything nested (`use` in a function: 959 candidates for 6 breaches) | Under 2 s, no tools | awk that someone must be able to read; each script needs its own self-test | None by platform: reads every tracked file |
| **Structural search** (`ast-grep`, `semgrep`), not tried | Exact for syntax patterns (a `use` inside a block, an escape in a string literal); no types, no macro expansion, poor at ordering rules | Seconds, one prebuilt binary, stable or no toolchain | Rule files in YAML, with the tool's own rule tests; a parser grammar that lags new syntax a little | None by platform. A new tool, which needs approval |
| **Dylint** | Exact for anything in the syntax tree or the types, with compiler-quality spans | A second full workspace check (7m50s cold), 2.4 GB nightly, 3.4 GB target | Pinned nightly and `clippy_utils`, broken by `rustc` API changes; upgrade up to every six weeks | Host `cfg` and enabled features only; not `crates/ebpf-redirect`; not templates or comments |

## Recommendation

**Keep Clippy and the scripts. Do not adopt Dylint now.**

- **It would enforce too little.** Ten of the thirteen rules are better served by Clippy, a
  script or review. The three left are syntax rules. Dylint's real advantage, type information,
  is used by none of them.
- **What it found does not justify it.** `use_in_function` found six lines, all one idiom with a
  reason behind it. `code_in_mod_rs` found one source file and 29 test files, the same 30 files
  a ten-line grep finds.
- **The cost is permanent.** A second compiler, a second full check of the workspace, 6 GB of
  toolchain and build output, a CI job and cache entry, and a library that broke twice in this
  short trial (once while being written, once on a 12-week upgrade). It is small per incident
  and it never ends.
- **It sees less than a text check** where Sherpa has platform-specific code.

What to do instead, each a separate piece of work for the user to approve:

1. **Turn on `clippy::absolute_paths`** with `absolute-paths-max-segments = 3` (47 findings to
   fix first). No new tool.
2. **Decide on `clippy::arbitrary_source_item_ordering`.** It enforces a stricter order than
   `AGENTS.md` states (238 findings in 114 files). Either accept the stricter order and write it
   into `AGENTS.md`, or leave the ordering rule to review.
3. **Settle the two rule questions the prototype raised:** whether "no code in `mod.rs`" covers
   `tests/` trees and private `use`, and whether a `#[cfg(unix)]` import inside a test function
   is allowed. Then fix or exempt what was found (not done here).
4. **Add scripts for the line-shaped rules** if they are wanted enforced: code in `mod.rs`, doc
   comment fences, issue numbers in comments. Same form as the two in `dev/`, with a self-test.
5. **For the nested syntax rules** (`use` in a function, raw strings), try `ast-grep` in an
   investigation of its own before any more grep. It needs approval as a new tool.

Revisit Dylint if a rule appears that needs types or name resolution, for example "`#[instrument]`
on every public function that calls into `db`", and review is failing to keep it.

### If the user adopts Dylint anyway

Rollout:

1. Decide the rule questions in step 3 above and fix or exempt the 327 findings, so the lints
   can run with `-D warnings`.
2. Keep `lints/` as it is; add `[workspace.metadata.dylint] libraries = [{ path = "lints" }]` to
   the root `Cargo.toml`.
3. Add `cargo:cargo-dylint` and `cargo:dylint-link`, pinned, to `mise.toml`.
4. Add a `Dylint` job to `.github/workflows/pr.yml` beside Lint, with its own cache, running the
   UI tests and `cargo dylint --all -- --workspace --all-targets -- -D warnings`. Not in the
   Windows or macOS jobs, not in the pre-commit hook.
5. `AGENTS.md`: name the lints under the rules they enforce; under "Clippy" say that
   `lints/` builds on a pinned nightly outside the workspace and that an upgrade
   (`cargo dylint upgrade lints`) is due when the workspace no longer compiles on it; under
   "Ask Before" nothing changes (tools and CI already need approval).
6. `tests/specs/README.md`: add `lints/ui` to "Tooling outside the specs".
7. `docs/architecture/BUILD.md`: record the toolchain, the job and the upgrade procedure.

Needs the user's approval: the two tools, the pinned nightly, the root `Cargo.toml` metadata,
the CI job and its cache use, and the `AGENTS.md` changes.

## Sources

- Dylint repository, README and "How Dylint works": <https://github.com/trailofbits/dylint>
- Dylint changelog: <https://github.com/trailofbits/dylint/blob/master/CHANGELOG.md>
- Dylint issues 2078 and 2146: <https://github.com/trailofbits/dylint/issues>
- Release dates: <https://crates.io/crates/cargo-dylint/versions>
- bevy_lint, "Upgrade the Rust toolchain":
  <https://thebevyflock.github.io/bevy_cli/contribute/linter/how-to/upgrade-rust-toolchain.html>
- Clippy lint descriptions: `cargo clippy --explain <lint>` on clippy 0.1.100.
