## Agent Instructions
`important` - I don't need to be flattered. Be direct and honest. I am a big person, tell me if my ideas are shit.

## Sherpa Project Rules

You are working on a Rust application that manages virtual machines, containers, and unikernels.  
The app talks to libvirt/KVM/QEMU for VMs and unikernels, and to Docker for containers. The
database in use is SurrealDB.

## Agent Hints
- In planning mode, don't generate implementation code. Stick to high level implementation plan only.
  Code samples don't add value and will likely change during implementation. Do not show code in the terminal.
- Always run `cargo fmt` and `cargo clippy --workspace -- -D warnings` before running tests.
  Formatting and linting must pass first. CI will fail if formatting is wrong, so catch it locally
  before committing.

## Project Tasks
- Features to implement are tracked in the PROJECT.md file.
- This file is a record of task/features that are outstanding.

## Documentation
- Application architecture is documented in `docs/architecture/`. When you change how a subsystem
  works (server startup, routing, auth, services, networking, observability), update the matching
  document there, or add one. See `docs/architecture/README.md` for the index and conventions.
- User-facing documentation lives in a separate repository at `../sherpa-docs` (MkDocs site).
  Any change to user-facing functionality must be reviewed against that repository, and if the
  docs there need to change, update them in that repository as part of the same piece of work.
  User-facing functionality includes CLI commands and flags, manifest fields and defaults,
  `sherpa.toml` settings, node models and their default parameters, API operations, install and
  update scripts, and user-visible behaviour or error messages.
- Debugging and research notes go in `docs/investigations/`. They record what was tried and found,
  not settled behaviour. When an investigation produces a lasting fact, move it to
  `docs/architecture/` or to `../sherpa-docs` and leave a pointer.
- Do not add user-facing documentation to this repository. Architecture docs describe how the
  code works, user docs describe how to use it.

## Git and Pull Requests
- Prefer local branches rather than worktrees.
- Do not include Claude session links or "Generated with Claude Code" footers in commit messages
  or pull request descriptions.
- Add `[skip ci]` to the commit message of changes that contain no code, such as documentation-only
  changes.

## Configuration / Project Files
- All configuration and project files (such as `sherpa.toml` and `manifest.toml`) are to use the 
  TOML format. Do not use any other format like JSON, YAML or INI. TOML is the only acceptable format.

## Crates
This project is built in a monorepo style. Encapsulate functionality in a sensible way, and
keep related functionality in the same crate.
Functional boundaries are separated into crates, found in the `crates/` directory.

### Crates and their purpose
client:
- Client binary CLI tool for interacting with the Sherpa Server. This is installed on user machines.

container:
- container node management logic via the Docker API.

db:
- database management. Schema/instantiation/CRUD operations.

ebpf-redirect:
- eBPF program used for point-to-point links. It is excluded from the workspace and has its own
  toolchain.

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

## Validation
Validators ensure user supplied data is accurate, usable and secure. They live with the crate
that owns the rule:
- Validators used by more than one crate live in `shared::validators`.
- Checks on manifest types, shared by the client and the server, live in `topology::validate`
  (they cannot go in `shared` because `topology` depends on `shared`).
- Validators used by a single crate live in that crate's `validate` module, e.g. `server::validate`.
- Avoid generating data in validators; data should be passed to them for validation.

## Scripts
- `scripts/` holds install, update and uninstall tooling.
- `tests/` holds test tooling, specs and environments. `tests/README.md` is the single reference
  for running every kind of test.
- `dev/` holds helper scripts for local development, such as `dev/testdb`.

## Project goals

- Provide a unified, **safe** control plane for:
  - Virtual machines (via libvirt/KVM/QEMU)
  - Unikernels (via libvirt/qemu)
  - Containers (via Docker and the Docker API)
- Favor reliability, observability, and debuggability over clever abstractions.
- Keep the public API stable and well-documented; avoid gratuitous breaking changes.

## Rust code style

Clippy:
- Fix linting issues by following clippy rules. Do not add any code to ignore clippy linting.

Errors:
- Use Anyhow for error handling. Prefer adding context errors before they are returned.

Concurrency:
- Use Tokio for async.
- Use Threadpools when necessary if an Async interface is not available.

Options:
- Never use `.unwrap()` in production code. Options must be handled.
- Never use `.expect()` in production code. Results must be handled.
- `.unwrap()` and `.expect()` are allowed in test code (`#[test]` functions and `#[cfg(test)]` modules).
- These rules are enforced by clippy via `#![deny(clippy::unwrap_used, clippy::expect_used)]` in
  each workspace crate's `lib.rs`/`main.rs`, with test exceptions handled by `clippy.toml`.
  The `ebpf-redirect` crate is `no_std` and outside the workspace, so the lint does not apply to it.

Use:
- Always declare `use` statements at the top of files.
- Never declare a `use` statement inside a function.
- Don't call long imports inline, EG: `shared::data::NodeState::Action`
   Prefer: `use shared::data::NodeState` at top of file | then in code:  `NodeState::Action`

Structs / Enums
- Always declare `structs` and `enums` under `use` statements.

Tests:
- Always declare unit tests at the end of file after production code.

Instrumentation:
- Public functions that perform I/O or lifecycle operations (database, libvirt, Docker, network,
  filesystem) must have `#[instrument]` attributes from `tracing`. Pure data and validation helpers do not need them.
- Use `skip()` for non-Debug or large types (e.g. `state`, `docker`, `progress`, `qemu_conn`).
- Use `fields()` to capture key identifiers (e.g. `%lab_id`, `%node_name`, `%iface_name`).
- Use default level (info) for top-level operations, `level = "debug"` for helpers/internals.

Mod files:
- Never add code in `mod.rs` files.
- Only add module `docs`, mod `declarations` and `pub use re-exports`.

General:
- Never use wrapper functions in the name of keeping diffs small.

## HTML Templates
- Use Askama for HTML templates. Do not manually render HTML as strings.
- Do not use inline templates, all templates must use files.

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
  - Keep XML/domain definition handling in one place; avoid sprinkling XML string manipulation across the codebase.

## Interacting with Docker

- Do not shell out to `docker` CLI from core code; use the Bollard Rust library used by the project.
- Ensure operations that change container state are idempotent where possible.
- Gracefully handle:
  - Daemon unavailability.
  - Network timeouts.
  - Inconsistent or stale container state.

## Testing and safety
- Always add or update tests for:
  - New lifecycle transitions (e.g. new stop/pause/migrate logic).
  - New error cases or recovery behavior.
  - Any bug fix.
- Prefer:
  - Unit tests for domain logic (no I/O, no libvirt/Docker).
  - Integration tests for real libvirt/Docker/database interactions. Mark them `#[ignore]` and run
    them with `-- --ignored`.
- When touching operations that start/stop/destroy resources:
  - Consider failure modes like partial success and rollback.
  - Avoid destructive defaults; be explicit about irreversible operations.
- Test in local files, should always be at the bottom of the file.
- Follow Red/Green/Refactor TDD methodology: write a failing test first, make it pass, then refactor.
- `tests/README.md` is the single reference for running unit, integration, VM and installer tests.
- `tests/specs/` holds the testing principles and per-crate specs. The spec files can lag the code,
  so where a spec and a test disagree, the test is authoritative. See `tests/specs/README.md`.

## Security and permissions

Allowed without asking:
- Reading project files and tests.
- Editing Rust code, configuration, and docs in this repository.
- Running `cargo fmt`, `cargo clippy`, and `cargo test` on the workspace or individual crates.

Ask before:
- Adding new external dependencies, especially FFI or network-heavy crates.
- Modifying any CI, release, or deployment pipeline definitions.
- Adding or changing code that performs host-level operations (raw `qemu` CLI, privileged Docker operations, or direct device manipulation).
- Writing or modifying scripts that could destroy user data (VM images, volumes, etc.).

Never:
- Hard-code credentials, private keys, IPs, or hostnames.
- Add code that bypasses authentication or authorization layers if they exist.

## Terminal Output
- Summary information presented in the terminal uses the `tabled` crate with the `modern` style.
- Table logic lives in the `shared::util::table` module.
- Streaming status messages to the terminal do not need to be in table form.

## Libraries
Use these preferred libraries for the listed functions.
- `jiff` - Anything to do with time, do not use `chrono` or `time` or any other time related crate.
  The one exception is converting to `time::OffsetDateTime` where `rcgen` requires it (`crates/server/src/tls/generator.rs`).
- `tokio` - Async runtime
- `bollard` - Container related functionality for connection to docker daemon.
- `virt` - Virtual machine related functionality for connection to libvirt daemon.
- `clap` - CLI utilities.
- `reqwest` - Web client.
- `axum` - Web server.
- `surrealdb` - Database.
- `anyhow` - Errors.
- `serde` - Serialization/Deserialization.
- `tabled` - Display information in the terminal

## Test Database
The following commands can be used to create a database for running tests:
- `dev/testdb start`   — spin up an in-memory SurrealDB v3.0.0 container
- `dev/testdb stop`    — stop the container
- `dev/testdb restart` — destroy and recreate (fresh database)
- `dev/testdb status`  — check if it's running
- `dev/testdb logs`    — tail container logs
- `dev/testdb destroy` — stop and remove the container
