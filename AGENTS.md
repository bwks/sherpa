## Overview
You are working on a Rust application that manages virtual machines, containers, and unikernel labs.  

## Agent Instructions
- `important` - You the LLM, are the soulless computer from startrek.
- `important` - I the person don't need to be flattered. Be direct and honest. I am a big person, tell me if my ideas are shit.

## Project Tasks
- Work is tracked in GitHub issues on `bwks/sherpa`, labelled `P0` to `P3` by priority, with a milestone per release.

## Documentation
### Architecture
- Application architecture is documented in `docs/architecture/`.
- Architecture changes/additions require matching documentation. 
- See `docs/architecture/README.md` for the index and conventions.

### User Facing
- User-facing documentation lives in a separate repository at `../sherpa-docs` (Zensical site).
- Change to user-facing functionality must be kept in sync with the current implementation together in the same piece of work.
  
### Investigations
- Debugging and research notes go in `docs/investigations/`. They record what was tried and found, not settled behaviour. 
- When an investigation produces a lasting fact, move it to `docs/architecture/` or to `../sherpa-docs` and add a pointer to the doc.

## Git
- Prefer local branches rather than worktrees.

## Pull Requests
- Docs-only pull requests (every changed path is a `*.md` file or under `docs/` or `tests/specs/`) skip the build jobs automatically, so commits never carry `[skip ci]`.
- Every pull request description ends with a `## Generation` section. See "Pull request Generation section" in `docs/architecture/AGENT_BUS.md`.

## Dependencies
- Sherpa has one version, `[workspace.package] version` in the root `Cargo.toml`. Every

## Releases
- A pull request never changes the version; only a bump pull request does. 
- How versions, bumps and releases work is in `docs/architecture/RELEASE.md`.

## Configuration / Project Files
- All configuration and project files (such as `sherpa.toml` and `manifest.toml`) are to use the TOML format.

## Crates
- This project uses a monorepo style. Encapsulate functionality in a sensible way, and keep related functionality in the same crate.
- Functional boundaries are separated into crates, found in the `crates/` directory.

### Crates Purpose
agent-bus:
- Message bus 0between the host agent and agents in lab guests. See:
  - Architecture: `docs/architecture/AGENT_BUS.md`.
  - Supporting: `docs/investigations/agent-bus.md` and `docs/investigations/agent-bus-database.md`.

client:
- Client binary CLI tool for interacting with the Sherpa Server. This is installed on user machines.

container:
- container node management logic via the Docker API.

db:
- database management. Schema/instantiation/CRUD operations.

ebpf-redirect:
- eBPF program used for point-to-point links. It is excluded from the workspace and has its own oolchain.

iso:
- ISO 9660 image writer for node configuration media: ZTP config ISOs, NoCloud seeds etc..

libvirt:
- Virtual Machine and Unikernel management logic, including storage pools and networks via the libvirt API.

network:
- Host network management operations.

server:
- Sherpa Server. Public interface users interact with via the Sherpa CLI or via web-ui.

shared:
- Shared data modeling and utilities.

template:
- Application and node configuration template generation.

topology:
- Lab management configuration data models transformers.

## Validators
Validators ensure user supplied data is accurate, usable and secure. They live with the crate that owns the rule:
- Validators used by more than one crate live in `shared::validators`.
- Validators used by a single crate live in that crate's `validate` module, e.g. `server::validate`.
- Validators only validate passed in data, they never generate or transform data.

## Scripts
- `scripts/` holds install, update and uninstall tooling.
- `tests/` holds test tooling, specs and environments. `tests/README.md` is the single reference for running every kind of test.
- `dev/` holds helper scripts for local development, such as `dev/serve`.

## Rust code style

### File Structure
#### Use
- Always declare `use` statements at the top of files.
- Never declare a `use` statement inside a function.
- Don't call long imports inline, EG: `shared::data::NodeState::Action` Prefer: `use shared::data::NodeState` at top of file | then in code:  `NodeState::Action`

#### Const
- Always declare `const` after `use` statements

#### Structs / Enums
- Always declare `structs` and `enums` under `const` statements.

#### Tests
- Always declare unit tests at the end of file after production code.

### Clippy
- Fix linting issues by following clippy rules. Do not add any code to ignore clippy linting.
- `dev/check-lint-exceptions.sh` refuses any `allow` or `expect` attribute, including inside `cfg_attr`, in any tracked Rust file (`crates/ebpf-redirect` included) that is not on the allowlist at the top of the script, and any allowlist entry that no longer exists. It runs in he pre-commit hook (`git config core.hooksPath .githooks`) and in the CI Lint job. Adding to the allowlist needs the user's approval; remove an entry when its exception is removed.
- Every workspace crate has `[lints] workspace = true` in its `Cargo.toml` and inherits the workspace lints in the root `Cargo.toml`, which include `clippy::pedantic`.

### Errors
- Library crates return their own error enum, defined with `thiserror`.
- Use Anyhow handling errors of other libraries. Prefer adding context errors before they are returned.

### Options
- Never use `.unwrap()` and `.expect()` are banned in production code. Options must be handled.
- `.unwrap()` and `.expect()` are allowed in test code (`#[test]` functions and `#[cfg(test)]` modules).

### Instrumentation
- Public functions that perform I/O or lifecycle operations (database, libvirt, Docker, network, filesystem) must have `#[instrument]` attributes from `tracing`. Pure data and validation helpers do not need them.
- Use `skip()` for non-Debug or large types (e.g. `state`, `docker`, `progress`, `qemu_conn`).
- Use `fields()` to capture key identifiers (e.g. `%lab_id`, `%node_name`, `%iface_name`).
- Use default level (info) for top-level operations, `level = "debug"` for helpers/internals.

### Mod files
- Never add code in `mod.rs` files.
- Only add module `docs`, mod `declarations` and `pub use re-exports`.

### Comments
- Doc comments (`//!` and `///`) carry no code blocks: no fenced blocks of any language, Rust, text or TOML included, so there are no doctests.
- Function doc must describe the purpose of the function.
- Inline comments should only be used to explain something that is abiguous if the code cannot be rewritten to make it's purpose obvious.

### Strings
- Write a string that contains `"` as a raw string (`r#"..."#`, `br#"..."#` for bytes), not with `\"` escapes. No lint enforces this.

### General
- Never use wrapper functions in the name of keeping diffs small.

## HTML Templates
- Use Askama for HTML templates. Do not manually render HTML as strings.
- Do not use inline templates, all templates must use files.
- A template decides nothing from an enum's text: no `to_string()` call and no comparison with a string literal in a `{% %}` tag. Rust gives it typed values to decide from. `dev/check-template-text-decisions.sh` enforces this; it runs in the pre-commit hook and in the CI Lint job.

## CSS Code Style
- Use tailwind css utility classes for styling.
- Do not apply inline styles in HTML.
- Do not create custom CSS classes, use tailwind css utility classes.

## Javascript Code Style
- Do not define inline javascript in templates.

## Interacting with libvirt / QEMU
- Always propagate detailed error information from libvirt/QEMU into our domain errors, but do not expose raw FFI types outside the libvirt crate.
- Handle lifecycle race conditions robustly:
  - Verify state before destructive operations.
  - Assume calls may fail due to concurrent operations or external changes.
- When adding new libvirt-based features:
  - Prefer existing safe Rust crates if already in use; avoid introducing a second binding library.
  - Keep XML/domain definition handling together, avoid sprinkling XML string manipulation across the codebase.

## Interacting with Docker
- Ensure operations that change container state are idempotent where possible.
- Gracefully handle:
  - Daemon unavailability.
  - Network timeouts.
  - Inconsistent or stale container state.

## Testing
- `tests/README.md` is the single reference for running unit, integration, VM and installer tests.
- Every test proves one case of a behaviour in a spec in `tests/specs/`, and its name is the case's ID (`<behaviour>_<NNN>_<description>`). The rules, levels and spec format are in `tests/specs/README.md`; read it before writing or changing a test.
- Always add or update a spec case and its test for:
  - New lifecycle transitions (e.g. new stop/pause/migrate logic).
  - New error cases or recovery behavior.
  - Any bug fix.
- Integration tests are marked `#[ignore]` and run with `-- --ignored`.
- Database tests opens ther own in-memory database with `db::open_in_memory()`, and `cargo test -p db` runs them.

## Safety
- When touching operations that start/stop/destroy resources:
  - Consider failure modes like partial success and rollback.
  - Avoid destructive defaults; be explicit about irreversible operations.

## Security and permissions

### Ask Before
- Adding any new dependency: a crate, a Python package or a tool.
- Modifying any CI, release, or deployment pipeline definitions.
- Adding or changing code that performs host-level operations (raw `qemu` CLI, privileged Docker operations, or direct device manipulation).
- Writing or modifying scripts that could destroy user data (VM images, volumes, etc.).

### Never
- Hard-code credentials, private keys, IPs, or hostnames.
- Add code that bypasses authentication or authorization layers.

## Terminal Output
- Information presented in the terminal uses the `tabled` crate with the `modern` style.
- Streaming status messages to the terminal do not need to be in table form.


## Agent Hints
- Before you build or test, read `tests/README.md`: what to run while iterating and before a commit, where to build and test (the dev servers), what never to boot there, and when a change needs a Windows check.
