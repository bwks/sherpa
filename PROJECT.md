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
- [x] **REST API for lab management** — All 22 operations wired up as REST endpoints. Streaming operations (create, destroy, redeploy, image import/pull/download) use JSON SSE. Non-streaming operations return JSON directly. Includes lab, image, and user management endpoints.
- [x] **ZTP for VMs** — All ZTP methods explicitly handled: None (skip), Volume (not applicable to VMs), Ipxe (not yet needed). Exhaustive match, no catch-all.
- [ ] **Rate limiting** — Acknowledged as missing in API.md.

### API
- [x] **OpenAPI/Swagger export** — `build_openapi()` in `api_spec.rs` transforms the unified spec into an OpenAPI 3.1 document. Served at `/api/v1/openapi.json`. Embedded Swagger UI at `/api/docs`. No new dependencies — derived from the existing operation registry.

### Observability
- [x] **OpenTelemetry integration (Phase 1 — Traces)** — OTLP trace export via `tracing-opentelemetry`, always compiled in, enabled via `[otel]` section in `sherpa.toml`. Layered subscriber keeps existing log output, adds OTel span export. HTTP request middleware + WebSocket/RPC span instrumentation. See `docs/OTEL.md`.
- [x] **OpenTelemetry (Phase 2 — Metrics)** — Connection count gauge, RPC latency histograms, operation duration by type, error rate counters. `Metrics` struct with noop pattern, `MeterProvider` init, 4 instruments exported via OTLP.
- [x] **OpenTelemetry (Phase 3 — Enhanced spans)** — `#[instrument]` on service functions, DB/Docker/libvirt span wrappers. W3C Trace Context propagation not needed (no outgoing HTTP calls). See `docs/OTEL.md`.

### Frontend
- [ ] **Dioxus native GUI frontend** — Planned but not yet started.

## Lower Priority
- [ ] **Shared crate test directory** — Inline tests exist but no dedicated test directory.
- [ ] **Install script BATS testing** — VM-based testing per `test-specs/install/HOW-TO-RUN.md`.

### P2P Interface
- [x] **eBPF Interface** - Build and eBPF interface that can be used in both libvirt VM's and Containers as point-to-point interfaces. Implements protocol-transparent P2p links using eBPF TC redirect (VM-VM, VM-Container, Container-Container). Includes TC netem link impairment, idempotent eBPF re-attachment for redeploy/resume, REST API endpoint for live impairment updates, and integration tests.