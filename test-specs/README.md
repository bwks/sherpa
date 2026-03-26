# Sherpa Test Specifications

This directory contains test specification documents for the Sherpa project. Each spec describes
**what** needs to be tested at a high level — functional areas, behaviors, and expected outcomes.
These are living documents that will be updated as the project evolves.

## Testing Principles

### Red/Green/Refactor (TDD)

All test implementation should follow the Red/Green/Refactor cycle:

1. **Red** — Write a failing test first. The test defines the expected behavior before any implementation exists or changes. Run it and confirm it fails.
2. **Green** — Write the minimum code necessary to make the test pass. No more, no less. Resist the urge to over-engineer at this stage.
3. **Refactor** — Clean up the implementation and the test. Remove duplication, improve naming, simplify logic. The tests must still pass after refactoring.

This cycle applies whether you are adding new functionality, fixing a bug, or extending existing behavior. Every bug fix should start with a test that reproduces the bug.

### General Principles

- **Test behavior, not implementation.** Tests should verify what a function does, not how it does it. This makes tests resilient to refactoring.
- **One assertion per concern.** Each test should validate a single behavior or invariant. If a test name needs "and" in it, consider splitting it.
- **Tests are documentation.** A well-named test communicates intent. Someone reading the test suite should understand the system's expected behavior without reading the source code.
- **Isolate external dependencies.** Unit tests must not require Docker, libvirt, SurrealDB, or network access. Use integration tests (gated behind feature flags or `#[ignore]`) for real service interactions.
- **Deterministic and repeatable.** Tests must produce the same result every run. No reliance on global state, ordering, or timing (except where explicitly testing timeouts).
- **Fast feedback.** Unit tests should run in milliseconds. Keep the unit test suite fast so it can run on every change. Integration tests are slower by nature but should still avoid unnecessary waits.
- **Test at the right level.** Use the test pyramid: many unit tests, fewer integration tests, fewest e2e tests. Push testing as low in the stack as possible.

---

## Conventions

When populating spec files, follow this structure:

- **Scope**: Brief description of what the spec covers
- **External Dependencies**: Services, permissions, or environment required (e.g., SurrealDB, Docker, libvirt, root access)
- **What To Test**: High-level list of functional areas and behaviors grouped by operation or concern
  - Success paths
  - Error/failure paths
  - Edge cases and boundary conditions
- **Test Type**: Label each area as `[unit]`, `[integration]`, or `[e2e]` to indicate the appropriate test level

Priority markers:
- **P0** — Critical: core functionality that must work for the project to be usable
- **P1** — Important: significant functionality that should be tested for confidence
- **P2** — Nice-to-have: edge cases, polish, and defensive coverage

---

## Directory Structure

```
test-specs/
  README.md
  client/
    cli-commands.md
    websocket-client.md
    token-management.md
  container/
    lifecycle.md
    image-management.md
    networking.md
  db/
    schema-and-seeding.md
    crud-operations.md
    relationships.md
  libvirt/
    vm-lifecycle.md
    disk-operations.md
    network-management.md
    storage-pools.md
  network/
    host-networking.md
  server/
    auth.md
    http-routes.md
    websocket-rpc.md
    services.md
    tls.md
    daemon.md
  shared/
    auth-utilities.md
    data-models.md
    networking-utilities.md
    tls.md
  template/
    vendor-configs.md
    cloud-init-ignition.md
    infrastructure.md
  topology/
    manifest-parsing.md
  validate/
    validation-rules.md
  install/
    sherpa-install.md
  integration/
    lab-lifecycle-e2e.md
    image-management-e2e.md
    user-management-e2e.md
```

---

## Folder & File Descriptions

### `client/` — CLI Client

The Sherpa CLI (`sherpad-client`) is the user-facing tool that communicates with the server over WebSocket.

| File | Purpose |
|------|---------|
| `cli-commands.md` | What to test for every CLI command: `up`, `down`, `destroy`, `resume`, `redeploy`, `inspect`, `validate`, `login`, `logout`, `init`, `new`, `console`, `ssh`, `image`, `cert`, and `server` admin subcommands. Covers argument parsing, manifest loading, environment variable expansion, output formatting, and error reporting for each command. |
| `websocket-client.md` | What to test for the WebSocket transport layer: connection establishment (plain and TLS), authentication handshake, request/response message handling, streaming message consumption (for long-running ops like `up` and `destroy`), timeout behavior, reconnection logic, and graceful disconnection. |
| `token-management.md` | What to test for JWT token persistence: saving tokens to disk, loading from disk, detecting expired tokens, handling missing/corrupt token files, and file permission safety. |

### `container/` — Docker Container Management

Manages containers via the Bollard library (Docker API). All operations require a running Docker daemon.

| File | Purpose |
|------|---------|
| `lifecycle.md` | What to test for the full container lifecycle: creating containers (with env vars, volumes, capabilities, networks, privileged mode), starting, stopping, pausing/unpausing, killing, executing commands inside containers (sync and detached), listing, and removing. Covers daemon unavailability, stale state (container already exists/stopped), network timeout, and idempotency. |
| `image-management.md` | What to test for Docker image operations: pulling from registries with progress tracking, listing local images, saving images to tar.gz, handling image-not-found errors, and network failures during pull. |
| `networking.md` | What to test for Docker network operations: creating bridge networks, macvlan networks, macvlan bridge networks, deleting networks, listing networks. Covers duplicate network names, network-in-use deletion, and filter behavior. |

### `db/` — Database Layer (SurrealDB)

CRUD operations and schema management for 6 tables: lab, node, link, bridge, user, node_image. Requires a running SurrealDB instance.

| File | Purpose |
|------|---------|
| `schema-and-seeding.md` | What to test for database initialization: schema application across all tables, schema idempotency (applying twice does not fail or corrupt), field constraints and validation rules defined in schema, index uniqueness enforcement, and admin user seeding (first run and when admin already exists). |
| `crud-operations.md` | What to test for create/read/update/delete across all 6 tables. For each table: successful CRUD, constraint violations (duplicate names, invalid references), edge cases (empty strings, boundary values for numeric fields like index 0-65535), and query variations (by ID, by name, by relationship, list with filters, count). |
| `relationships.md` | What to test for cross-table integrity: foreign key references (lab→nodes, node→image, node→links), cascade delete behavior (deleting a lab removes its nodes/links/bridges), reference rejection (cannot delete an image in use by nodes), orphan prevention, and multi-table query correctness (e.g., listing labs with node counts). |

### `libvirt/` — VM & Unikernel Management

Manages VMs and unikernels via the `virt` crate (libvirt bindings). Requires a running libvirt daemon with QEMU/KVM.

| File | Purpose |
|------|---------|
| `vm-lifecycle.md` | What to test for VM creation and management: defining a VM from XML, starting a defined domain, retrieving management IP addresses (guest agent available vs unavailable), handling connection failures, domain-already-exists scenarios, and lifecycle race conditions (VM destroyed between check and operation). |
| `disk-operations.md` | What to test for disk management: cloning disks across supported formats (qcow2, iso, raw, json, ign, img), rejecting unsupported extensions, cloning from nonexistent source, streaming upload in chunks, resizing disks (valid upsize, rejecting downsize), deleting disks, and storage pool unavailability. |
| `network-management.md` | What to test for libvirt network types: creating bridge, isolated, NAT, and reserved networks, idempotency (network already exists and active, exists but inactive), XML template rendering correctness, autostart configuration, and cleanup/deletion. |
| `storage-pools.md` | What to test for storage pool management: creating directory-based pools, idempotency (pool already exists), directory creation failures (permissions), pool activation, autostart setting, and pool refresh. |

### `network/` — Host Network Management

Linux host-level network operations via rtnetlink. Requires elevated privileges.

| File | Purpose |
|------|---------|
| `host-networking.md` | What to test for host network interface management: creating Linux bridges (with jumbo MTU), creating veth pairs, enslaving interfaces to bridges, deleting interfaces, fuzzy interface name matching (zero/one/many results), handling nonexistent interfaces, duplicate name handling, and netlink connection failures. |

### `server/` — Sherpa Server

The main Axum-based web server with HTTP routes, WebSocket RPC, authentication, services, TLS, and daemon management.

| File | Purpose |
|------|---------|
| `auth.md` | What to test for authentication and authorization: JWT token creation and validation (valid, expired, malformed, missing), cookie-based sessions, auth middleware extractors (`AuthenticatedUser`, `AdminUser`), login/signup flows, password verification, SSH key auth, admin vs non-admin permission boundaries, and session expiry. |
| `http-routes.md` | What to test for HTTP endpoints: public routes (login page, signup page, health check, cert endpoint), protected routes (dashboard, labs, profile, admin pages), API routes (lab list, lab detail), static asset serving, SSE streaming endpoints, WebSocket upgrade. For each: correct status codes, auth requirements, response format, redirect behavior, and error responses. |
| `websocket-rpc.md` | What to test for the JSON-RPC 2.0 WebSocket interface: method dispatch for all ~20 RPC methods, parameter validation, auth/admin requirements per method, success response shapes, error response codes, streaming message sequences (for `up`, `destroy`, `redeploy`, `image.pull`, `image.download`), unknown method handling, and malformed request handling. |
| `services.md` | What to test for the service/orchestration layer: lab startup (`up` — network creation, VM/container provisioning, ZTP config generation, progress reporting), lab shutdown (`down`), lab destruction (`destroy` — resource cleanup ordering), node resume, node redeploy, lab inspection, image import/scan, container image pull, and lab cleanup. Covers partial failure handling, rollback behavior, progress streaming, and idempotency. |
| `tls.md` | What to test for TLS certificate management: self-signed certificate generation, loading certs from disk, handling invalid/expired certs, certificate renewal, and the `/cert` endpoint for client cert distribution. |
| `daemon.md` | What to test for daemon lifecycle management: starting the server, PID file creation and cleanup, stop/restart behavior, stale PID file detection, log output, AppState initialization, and health check (`doctor`). |

### `shared/` — Shared Data Models & Utilities

Common types, utilities, and constants used across all crates.

| File | Purpose |
|------|---------|
| `auth-utilities.md` | What to test for auth primitives: password hashing and verification (argon2), JWT token encode/decode/claims extraction, SSH key generation and validation, and SSH key fingerprint computation. |
| `data-models.md` | What to test for core data types: serialization/deserialization round-trips (serde) for key structs and enums, `NodeModel` enum completeness (60+ variants), `NodeState` transitions, `Display` and `FromStr` implementations, `Default` trait correctness, interface type/name mappings per device model, and enum variant string representations. |
| `networking-utilities.md` | What to test for network utility functions: IPv4/IPv6 address parsing and arithmetic, subnet calculations, MAC address generation and formatting, port availability checks, DNS record generation, DHCP configuration generation, interface naming utilities, SSH config generation, and host resolution helpers. |
| `tls.md` | What to test for TLS utilities: certificate fetching from remote servers, TLS configuration construction, trust store management (adding/removing/listing trusted certs), and certificate validation. |

### `template/` — Configuration Template Generation

Askama-based templates for generating device-specific bootstrap/ZTP configurations.

| File | Purpose |
|------|---------|
| `vendor-configs.md` | What to test for vendor-specific template rendering: correct output for each vendor family (Cisco IOS/IOS-XE/IOS-XR/NX-OS/ASA/FTDv/ISE, Arista EOS, Juniper JunOS, Nokia SR Linux, Palo Alto PAN-OS, Aruba AOS-CX, Cumulus, Mikrotik RouterOS, SONiC, FRR). Covers required fields producing valid config syntax, optional field handling, special character escaping, and interface naming correctness per platform. |
| `cloud-init-ignition.md` | What to test for cloud-init and ignition template rendering: cloud-init user-data, network-config, and meta-data generation; cloudbase-init for Windows; ignition config generation (users, files, systemd units, links, filesystems). Covers YAML validity for cloud-init output and JSON validity for ignition output. |
| `infrastructure.md` | What to test for infrastructure template rendering: libvirt domain XML generation (correct CPU, memory, disk, network interface definitions), dnsmasq DHCP/DNS config, SSH client config (host entries, key paths), pyATS inventory YAML, and HashiCorp Vault config. |

### `topology/` — Lab Topology Configuration

TOML manifest parsing and representation for lab definitions.

| File | Purpose |
|------|---------|
| `manifest-parsing.md` | What to test for manifest handling: parsing valid TOML manifests, round-trip fidelity (parse then serialize), minimal manifest (just a name and one node), full manifest with all optional fields populated, missing required fields, invalid TOML syntax, node/link/bridge struct defaults, volume mount and startup script parsing, example manifest generation, and TOML editing operations (add/remove nodes and links). |

### `validate/` — Validation Primitives

Input validation for user-supplied manifests and configuration data.

| File | Purpose |
|------|---------|
| `validation-rules.md` | What to test for all validation functions: duplicate device name detection, management interface protection (index 0 not used in data links), duplicate interface usage across links and bridges, interface index bounds checking per device model, link device existence validation, node image field validation (CPU count, memory, interface count, MTU, version string, interface prefix), version resolution against database, container image existence verification, VM disk file existence verification, TCP connectivity checks, environment variable validation, and IPv6 address validation. |

### `install/` — Install Script

The main installation script that deploys Sherpa on Ubuntu 24.04+ systems.

| File | Purpose |
|------|---------|
| `sherpa-install.md` | What to test for `scripts/sherpa_install.sh`: CLI argument parsing, pre-flight checks (Ubuntu version, root, curl, port, virtualization), password and IP validation, system package installation, Docker/libvirt setup, sherpa user/group creation, directory structure, SurrealDB container management, health checks, binary download/install from GitHub releases, systemd service/env file/logrotate setup, and error cleanup. |

### `integration/` — Cross-Crate End-to-End Tests

Tests that exercise multiple crates together through realistic workflows.

| File | Purpose |
|------|---------|
| `lab-lifecycle-e2e.md` | What to test for the complete lab lifecycle: client sends manifest via WebSocket, server validates and parses, database records are created, networks are provisioned, VMs and containers are created with correct ZTP configs, progress is streamed back to client, inspect returns accurate state, `down` gracefully stops all nodes, `destroy` cleans up all resources (VMs, containers, networks, DB records). Covers the happy path, partial failure during provisioning, destroying a partially-created lab, and mixed VM+container labs. |
| `image-management-e2e.md` | What to test for image workflows end-to-end: importing a VM image from file, scanning disk for available images, setting a default image version, deleting an image (with and without nodes using it), pulling a container image from registry, and downloading a VM image. Each flow goes from client command through RPC to server service to database state change. |
| `user-management-e2e.md` | What to test for user workflows end-to-end: creating a user (admin and non-admin), login and token issuance, token validation on subsequent requests, password change, SSH key management (add/remove), user deletion (with and without owned labs), admin permission boundaries (non-admin cannot access admin operations), and the last-admin safety check (cannot delete the only admin). |

---

## Current Test Coverage Summary

| Crate | Existing Tests | Status |
|-------|---------------|--------|
| client | None | No coverage |
| container | None | No coverage |
| db | 28 integration test files (CRUD across all tables) | Partial — CRUD covered, schema/seeding/relationships gaps |
| libvirt | None | No coverage |
| network | None | No coverage |
| server | None | No coverage |
| shared | ~48 files with inline `#[cfg(test)]` modules | Partial — utilities well-covered, data models/TLS gaps |
| template | None | No coverage |
| topology | Minimal inline tests | Minimal |
| validate | 23 unit tests (device, image, version) | Partial — link/connection/env/IPv6 gaps |
