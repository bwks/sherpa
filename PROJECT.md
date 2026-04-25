# Project Backlog

## Build Performance

- [x] **Baseline build timings** — Clean debug workspace build: 8m 40s; incremental rebuild after touching `crates/shared/src/lib.rs`: 12.9s; `cargo clippy --workspace -- -D warnings`: 4m 47s.
- [x] **Workspace profile cleanup** — Moved release profile settings to the workspace root and removed ignored package-local profile settings.
- [x] **Optional fast linker docs** — Documented local opt-in `lld` configuration. `mold` is not used or recommended.
- [x] **Reqwest dependency deduplication** — Upgraded direct Sherpa `reqwest` usage to workspace `reqwest` 0.13.2 and disabled default OpenTelemetry OTLP HTTP client features, removing duplicate direct `reqwest` 0.11/0.12 stacks from the lockfile.
- [x] **Tokio feature review** — Replaced `tokio` `full` usage in workspace crates with narrower feature sets where safe.
- [x] **Post-change build timings** — Clean debug workspace build: 9m 15s; incremental rebuild after touching `crates/shared/src/lib.rs`: 41.7s; `cargo clippy --workspace -- -D warnings`: 4m 46s. A trial with global `profile.dev.opt-level = 1` was worse at 10m 45s clean, so it was not kept.
- [x] **Quality checks** — Ran `cargo fmt`, `cargo clippy --workspace -- -D warnings`, and `cargo test --workspace`.

## Web UI
### Console
- [ ] **Console access** — Nodes table "Console" button is disabled and marked "Coming Soon". No web terminal or VNC integration.

### Links
- [ ] **Link impairment config** — API `POST /api/v1/labs/{lab_id}/links/{link_index}/impairment` exists. Lab detail shows links read-only with no edit UI.


## CLI

- [ ] **Link impairment command** — No CLI command to set/view link impairment (latency, jitter, packet loss). API exists but CLI has no way to use it.

## Server

- [ ] **Rate limiting** — Acknowledged as missing in API.md.
- [ ] **Configurable self-registration** — Signup is always enabled with no way to disable it.

## Frontend

- [ ] **Dioxus native GUI frontend** — Planned but not yet started.
