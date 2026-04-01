# Project Backlog

## High Priority

### Testing
- [x] **Clippy unwrap/expect lint enforcement** — Added `#![deny(clippy::unwrap_used, clippy::expect_used)]` to all crates with test exemptions via `clippy.toml`.
- [x] **Validate crate tests** — 79 tests already exist covering all modules. Test spec gap summary was stale.
- [x] **Server crate unit tests** — 87 tests (86 pass, 1 ignored). Covers auth (JWT, cookies, context, extractors), API errors, daemon/pidfile, services (import, node_ops, env), TLS (generator, loader, cert manager). Integration tests (login/signup flow, protected routes) still need a running DB.
- [x] **Client crate tests** — 35 unit tests covering manifest processing, SSH, up (ZTP, env vars, text files, SSH config rewrite), and token management.
- [ ] **Integration test suite** — Specs exist in `test-specs/integration/` but no test code yet.

## Medium Priority

### Features
- [x] **Container image import** — Loads tar/tar.gz archives into Docker via Bollard `import_image_stream`. Supports air-gapped environments.
- [ ] **REST API for lab management** — Stub endpoints registered at `/api/v1/labs`. Needs async job pattern for streaming operations (up, destroy, redeploy). Non-streaming operations (down, resume, inspect, list) can call service layer directly. Endpoints: `POST /api/v1/labs`, `DELETE /api/v1/labs/{id}`, `POST /api/v1/labs/{id}/down`, `POST /api/v1/labs/{id}/resume`, `POST /api/v1/labs/{id}/nodes/{node_name}/redeploy`.
- [ ] **ZTP for VMs** — Some ZTP methods bail with "not yet implemented" in `crates/server/src/services/node_ops.rs`.
- [ ] **Rate limiting** — Acknowledged as missing in API.md.

### API
- [ ] **OpenAPI/Swagger export** — Add `build_openapi()` function in `api_spec.rs` that transforms the unified spec into an OpenAPI 3.1 document. Serve at `/api/v1/openapi.json`. No new dependencies — derive from the existing operation registry, not maintained separately.

### Frontend
- [ ] **Dioxus native GUI frontend** — Planned but not yet started.

## Lower Priority
- [ ] **Shared crate test directory** — Inline tests exist but no dedicated test directory.
- [ ] **Install script BATS testing** — VM-based testing per `test-specs/install/HOW-TO-RUN.md`.
