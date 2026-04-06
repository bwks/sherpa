## Agent Instructions
`important` - I don't need to be flattered. Be direct and honest. I am a big person, tell me if my ideas are shit.

## Sherpa Project Rules

You are working on a Rust application that manages virtual machines, containers, and unikernels.  
The app talks to libvirt/KVM/QEMU for VMs and unikernels, and to Docker for containers. The
database in use is SurrealDB.

## Agent Hints
- Review this file between each prompt as I will update it as we go along.
- In planning mode, don't generate implementation code. Stick to high level implementation plan only.
  Code samples don't add value and will likely change during implementation. Do not show code in the terminal.
- Always run `cargo fmt` and `cargo clippy --workspace -- -D warnings` before running tests.
  Formatting and linting must pass first. CI will fail if formatting is wrong, so catch it locally
  before committing.

## Code Navigation
Use LSP operations (goToDefinition, findReferences, hover) for all code navigation 
in preference to grep/glob. Only fall back to grep when LSP returns no results.

## Project Tasks
- Features to implement are tracked in the PROJECT.md file.
- This file is a record of task/features that are outstanding.

## Configuration / Porject Files
- All configuration and project files (such as `sherpa.toml` and `manifest.toml`) are to use the 
  TOML format. Do not use any other format like JSON, YAML or INI. TOML is the only acceptable format.

## Crates
This project is built in a mono-repo style. Encapsulate functionality in a sensible way.
Take your time and think about this, it's important that funcionality stays together.

Crates can be found in the `crates/` directory.

### Crates and their purpose
client:
- Client binary CLI tool for interacting with the Sherpa Server. This is installed on user machines.

container:
- container node management logic via the Docker API.

db:
- database management. Schema/instantiation/CRUD operations.

libvirt:
- Virtual Machine and Unikernel management logic, including storage pools and networks via the libvirt API.

network:
- Host network management operaions.

serever:
- Sherpa Server. Public interface users interact with via the Sherpa CLI or via web-ui.

shared:
- Shared data modeling and utilities.

template:
- Application and node configuration template generation.

topology:
- Lab management configuration data models transformers.

validate: 
- Validation primatives to ensure user supplied data is accurate, usable and secure.
- Avoid generating data in this crate, data should be passed to this crate for validation. 

## Scripts
Any bash helper scripts go in the `/scripts` directory. 

## Project goals

- Provide a unified, **safe** control plane for:
  - Virtual machines (via libvirt/KVM/QEMU)
  - Unikernels (via libvirt/qemu)
  - Containers (via Docker and the Docker API)
- Favor reliability, observability, and debuggability over clever abstractions.
- Keep the public API stable and well-documented; avoid gratuitous breaking changes.

## Architecture and boundaries

- This project uses are monorepo architecture.
- Functional boundaries are seperated into crates.
- Crates are found in the ./crates directory.

## Rust code style

Formatting:
- Always use `cargo fmt` to ensure code is formatted.

Clippy:
- Always run `cargo clippy` and fix linting issues by following clippy rules. Do no add any code
  to ignore clippy linting.

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
  each crate's `lib.rs`/`main.rs`, with test exceptions handled by `clippy.toml`.

Use:
- Always declare `use` statements at the top of files.
- Never declare a `use` statement inside a fuction.
- Don't call long imports inline, EG: `shared::data::NodeState::Action`
   Prefer: `use shared::data::NodeState` at top of file | then in code:  `NodeState::Action`

Structs / Enums
- Always declare `structs` and `enums` under `use` statements.

Instrumentation:
- All public functions must have `#[instrument]` attributes from `tracing`.
- Use `skip()` for non-Debug or large types (e.g. `state`, `docker`, `progress`, `qemu_conn`).
- Use `fields()` to capture key identifiers (e.g. `%lab_id`, `%node_name`, `%iface_name`).
- Use default level (info) for top-level operations, `level = "debug"` for helpers/internals.

## HTML Code Style
- Use Askama for HTML templates. Do not manually render HTML as strings.

## CSS Code Style
- Use tailwind css utility classes for styling.
- Do not apply inline styles in HTML.
- Do not create custom CSS classes, use tailwind css utility classes.

## Javascript Code Style
- Do not define inline javascipt in templates.

## Askama Templates
- Do not use inline templates, all templates much use files.

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
  - Integration tests for real libvirt/Docker interactions, guarded by feature flags or environment variables.
- When touching operations that start/stop/destroy resources:
  - Consider failure modes like partial success and rollback.
  - Avoid destructive defaults; be explicit about irreversible operations.
- Test in local files, should always be at the bottom of the file.
- Follow Red/Green/Refactor TDD methodology: write a failing test first, make it pass, then refactor.
- Detailed test specifications for each crate live in `test-specs/`. Consult the relevant spec file
  before writing tests for a crate. See `test-specs/README.md` for the full directory index and
  testing principles.

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

## Tables
- Use the `tabled` library for terminal tables. 
- Use the `modern` style tables
- Table logic lives in the `shared::util::table.rs`module.

## Terminal Outputs
- Summary information presented in the terminal should use the `tabled` create.
- Streaming status messages to the terminal do not need to be in table form.

## Libraries
Use these prefered libraries for the listed functions.
- `jiff` - Anything to do with time, do not use `chrono` or `time` or any other time related crate.
- `tokyo` - Async runtime
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
-`dev/testdb start`   — spin up an in-memory SurrealDB v3.0.0 container
-`dev/testdb stop`    — stop the container
-`dev/testdb restart` — destroy and recreate (fresh database)
-`dev/testdb status`  — check if it's running
-`dev/testdb logs`    — tail container logs
-`dev/testdb destroy` — stop and remove the containe