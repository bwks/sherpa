# Integration Test Implementation Plan

## Current Status

| Phase | Status | Notes |
|-------|--------|-------|
| 0 — VM Setup | **DONE** | libvirtd started, repo cloned, dirs created, images pulled/downloaded |
| 1 — Baseline | **BLOCKED** | Unit tests pass. DB integration tests fail — SurrealDB v3.0.0 port issue (see below) |
| 2 — Test Harness | **DONE** | TestServer, TestWsClient, TestHttpClient created in `crates/server/tests/helpers/` |
| 3 — Auth + HTTP Tests | **DONE** | Written in `crates/server/tests/auth_tests.rs` — not yet run |
| 4 — WebSocket RPC Tests | **DONE** | Written in `crates/server/tests/websocket_tests.rs` — not yet run |
| 5 — User Management E2E | **DONE** | Written in `crates/server/tests/user_management_tests.rs` — not yet run |
| 6 — Image Management E2E | **DONE** | Written in `crates/server/tests/image_management_tests.rs` — not yet run |
| 7 — Lab Lifecycle E2E | **DONE** | Written in `crates/server/tests/lab_lifecycle_tests.rs` — not yet run |
| 8 — Per-Crate Integration | **ALREADY EXISTS** | container, network, libvirt, template, topology all have tests |
| 9 — Test Runner Script | **DONE** | `scripts/run-integration-tests.sh` |

### Known Issue: SurrealDB v3.0.0 Port Change

SurrealDB v3.0.0 changed its default listen port from 42069 to 8000. The `dev/testdb` script was updated to map `host:42069 -> container:8000`, but the DB tests are still getting connection refused. The DB container IS running and port 42069 IS bound on the host (`ss -tlnp` confirms), but something in the SurrealDB startup or the WebSocket handshake is failing.

**To debug on the VM:**
```bash
# Check if SurrealDB responds on the mapped port
curl http://localhost:42069/health

# If that fails, check the container logs
docker logs sherpa-test-db

# Try connecting directly to the container's internal port
docker exec sherpa-test-db curl http://localhost:8000/health
```

### What Was Done (Code Changes)

1. **`crates/server/src/lib.rs`** (NEW) — Exposes server internals for integration tests
2. **`crates/server/src/main.rs`** — Refactored to use `sherpad::` imports from lib.rs
3. **`crates/server/Cargo.toml`** — Added `tokio-tungstenite` and `reqwest` dev-deps
4. **`crates/server/tests/helpers/`** — TestServer, TestWsClient, TestHttpClient
5. **`crates/server/tests/auth_tests.rs`** — 11 tests (JWT, cookie, HTTP routes)
6. **`crates/server/tests/websocket_tests.rs`** — 5 tests (connection, dispatch, auth methods)
7. **`crates/server/tests/user_management_tests.rs`** — 8 tests (CRUD, passwords, permissions)
8. **`crates/server/tests/image_management_tests.rs`** — 7 tests (scan, list, show, import, pull)
9. **`crates/server/tests/lab_lifecycle_tests.rs`** — 7 tests (container/VM up/down/destroy, errors)
10. **`crates/server/src/api/extractors.rs`** — Removed non-compiling docstring code examples
11. **`dev/testdb`** — Updated port mapping for SurrealDB v3.0.0
12. **`scripts/run-integration-tests.sh`** (NEW) — Runs all tests in order

### How to Run on the VM

```bash
export PATH="$HOME/.cargo/bin:$PATH"
cd /home/sherpa/code/rust/sherpa

# 1. Fix the testdb issue first, then:
./dev/testdb restart
sleep 5

# 2. Unit tests
cargo test --workspace

# 3. DB integration tests
cargo test -p db -- --ignored

# 4. Server integration tests (need sudo for network ops in lab lifecycle)
sudo -E $(which cargo) test -p sherpad -- --ignored --test-threads=1

# 5. Per-crate integration tests
cargo test -p container -- --ignored --test-threads=1
sudo -E $(which cargo) test -p network -- --ignored --test-threads=1
cargo test -p libvirt -- --ignored --test-threads=1

# Or run everything via the script:
sudo -E ./scripts/run-integration-tests.sh
```

---

## Context

The Sherpa project has comprehensive test specifications in `test-specs/` but most integration tests are unimplemented. Only the `db` crate has integration tests (28 files). The goal is to implement all integration tests and run them on a dedicated VM.

**VM**: Ubuntu 24.04, 16 CPU, 31GB RAM, Docker running, KVM available, libvirtd startable, GitHub CLI authenticated. User `sherpa` with passwordless sudo.

## VM Setup (Phase 0)

Run on the VM via SSH (`ssh -F /tmp/integrationtest/sherpa_ssh_config 172.31.1.11`):

```bash
# Start libvirtd
sudo systemctl start libvirtd

# Clone repo
cd /home/sherpa && gh repo clone <owner>/sherpa code/rust/sherpa
cd /home/sherpa/code/rust/sherpa

# Start test DB (SurrealDB in Docker on port 42069)
./dev/testdb start

# Pull images needed for container tests
docker pull alpine:latest
docker pull surrealdb/surrealdb:v3.0.0

# Create /opt/sherpa directory structure (services reference these paths)
sudo mkdir -p /opt/sherpa/{config,env,.ssh,images,containers,bins,labs,run,logs,.certs,.secret}
sudo chown -R sherpa:sherpa /opt/sherpa

# Build workspace to catch issues early
export PATH="$HOME/.cargo/bin:$PATH"
cargo build --workspace
```

### Test Node Images

Integration tests need real node images to exercise lab lifecycle flows.

**VM Image — Ubuntu Cloud (model: `ubuntu_linux`)**

Ubuntu Cloud images are small (~700MB), use cloud-init ZTP, and are ideal for VM lifecycle tests.

```bash
# Download Ubuntu 24.04 cloud image
mkdir -p /opt/sherpa/images/ubuntu_linux/24.04
wget -O /opt/sherpa/images/ubuntu_linux/24.04/virtioa.qcow2 \
  https://cloud-images.ubuntu.com/noble/current/noble-server-cloudimg-amd64.img
```

Image storage convention: `/opt/sherpa/images/{model}/{version}/virtioa.qcow2`

The `ubuntu_linux` model uses:
- Kind: VirtualMachine
- ZTP: CloudInit
- HDD Bus: Virtio
- Default memory: 1024 MB

**Container Image — Nokia SR Linux (model: `nokia_srlinux`)**

SR Linux is the primary container-based network OS in the project.

```bash
# Pull Nokia SR Linux
docker pull ghcr.io/nokia/srlinux:latest
```

The `nokia_srlinux` model uses:
- Kind: Container
- ZTP: None (containers don't use ZTP)
- Container repo: `ghcr.io/nokia/srlinux`
- Default memory: 4096 MB, CPU: 2
- 34 data interfaces, prefix: "eth-1/"

**Register images in DB via image.scan**

After the TestServer starts in each test, the test harness should call `image.scan` RPC to auto-discover and register:
- VM images: scans `/opt/sherpa/images/` directory structure, matches `{model}/{version}/virtioa.qcow2`
- Container images: queries Docker daemon for local images matching known model repos

Alternatively, tests can call `image.import` RPC directly for VMs or `image.pull` for containers to register specific images. The scan approach is simpler for bootstrapping since images are already on disk/in Docker.

## Baseline: Run Existing Tests (Phase 1)

```bash
cargo test --workspace                                           # ~90 unit tests
cargo test -p db -- --ignored                                    # 28 DB integration tests
```

## Test Harness (Phase 2) — Critical Foundation

### Files to create

```
crates/server/tests/
  helpers/
    mod.rs              — re-exports
    test_server.rs      — TestServer: in-process server on random port
    ws_client.rs        — TestWsClient: WebSocket RPC client
    http_client.rs      — TestHttpClient: reqwest-based HTTP client
```

### TestServer design

Construct `AppState` directly (all fields are `pub`):

- **DB**: `db::connect("localhost", 42069, &namespace, "test_db", "root")` — unique namespace per test (same pattern as `crates/db/tests/helper.rs`)
- **Schema**: `db::apply_schema(&db)` then `db::seed_admin_user(&db, "TestPass123!")`
- **Docker**: `Docker::connect_with_local_defaults()`
- **Qemu**: `Qemu::default()` (lazy, won't fail if libvirtd is down)
- **Config**: Construct `Config` manually with TLS disabled, images_dir/containers_dir/bins_dir pointing to temp dirs
- **JWT secret**: Random 32 bytes in-memory
- **Metrics**: `Metrics::noop()`
- **Router**: `crate::api::router::build_router()` — already public
- **Bind**: `TcpListener::bind("127.0.0.1:0")` for OS-assigned port, spawn with `axum::serve`
- **ConnectionRegistry**: `crate::api::websocket::connection::create_registry()`

### TestWsClient design

- Connect to `ws://127.0.0.1:{port}/ws` via `tokio_tungstenite`
- Read initial `Connected` message
- `rpc_call(method, params) -> Result<Value>` — send RPC request, wait for RPC response
- `rpc_call_streaming(method, params) -> Result<(Vec<StatusMessage>, Value)>` — collect status messages + final response
- `login(username, password) -> Result<String>` — convenience: call `auth.login`, return token

### TestHttpClient design

- Wrap `reqwest::Client` with cookie jar
- `get(path)`, `post(path, body)` with optional Bearer token
- Base URL from TestServer address

### Key dependencies to add to `crates/server/Cargo.toml` [dev-dependencies]

- `tokio-tungstenite` (already a transitive dep)
- `tempfile` for temp directories

## Server Auth & HTTP Tests (Phase 3)

Spec: `test-specs/server/auth.md`, `test-specs/server/http-routes.md`

```
crates/server/tests/
  auth/
    mod.rs
    jwt_tests.rs          — JWT validation (valid/expired/malformed/missing) via WS RPC
    cookie_tests.rs       — Cookie session flow (login sets cookie, requests use it, logout clears)
    extractor_tests.rs    — AuthenticatedUser/AdminUser extractors via HTTP routes
  http/
    mod.rs
    public_routes.rs      — GET /health 200, GET /login renders, GET /cert
    protected_routes.rs   — Unauthenticated → redirect/401
    admin_routes.rs       — Non-admin → 403
    api_routes.rs         — JSON API endpoints return correct content type
```

All tests use `#[ignore]` and `#[tokio::test]`.

## WebSocket RPC Tests (Phase 4)

Spec: `test-specs/server/websocket-rpc.md`

```
crates/server/tests/
  websocket/
    mod.rs
    dispatch_tests.rs       — Valid dispatch, unknown method error, malformed JSON, response ID matching
    auth_methods_tests.rs   — auth.login valid/invalid, auth.validate valid/expired
    connection_tests.rs     — Initial connected message, multiple concurrent connections
```

## User Management E2E (Phase 5)

Spec: `test-specs/integration/user-management-e2e.md`

```
crates/server/tests/
  e2e/
    mod.rs
    user_management.rs
```

P0 tests:
- Admin creates user via `user.create` RPC → user exists, can login
- Duplicate username rejected
- Non-admin cannot create users
- Password stored as Argon2id hash
- Login → JWT token → use in subsequent calls
- Expired token rejected, invalid credentials rejected
- User changes own password → old password fails
- Admin changes another user's password
- Add SSH key, invalid format rejected, keys stored and retrievable
- Admin access to admin RPC, non-admin rejected
- User only sees own labs, admin sees all
- Admin deletes user, last admin cannot be deleted

## Image Management E2E (Phase 6)

Spec: `test-specs/integration/image-management-e2e.md`

```
crates/server/tests/
  e2e/
    image_management.rs
```

P0 tests:
- Import VM image from local file → DB record + file in images_dir
- First image auto-marked as default
- Import nonexistent file → error
- Container image pull from registry via `image.pull` RPC
- `image.list` returns all, `image.show` returns correct NodeConfig
- `image.set_default` changes default, previous default unset
- `image.delete` removes DB record + files; blocked if nodes reference it
- `image.scan` discovers images on disk

Prerequisites: Docker running, `ubuntu_linux` VM image at `/opt/sherpa/images/ubuntu_linux/24.04/virtioa.qcow2`, `nokia_srlinux` container image in Docker (`ghcr.io/nokia/srlinux:latest`).

## Lab Lifecycle E2E (Phase 7)

Spec: `test-specs/integration/lab-lifecycle-e2e.md`

```
crates/server/tests/
  e2e/
    lab_lifecycle.rs
```

**Test image bootstrap**: Each lab lifecycle test setup should:
1. Call `image.scan` RPC to register `ubuntu_linux` VM images from `/opt/sherpa/images/` and `nokia_srlinux` container images from Docker
2. Verify images registered before proceeding with lab operations

**Container-only tests** (Docker only, no KVM needed) — uses `nokia_srlinux` model:
- Create manifest with SR Linux container nodes → `up` via RPC → verify DB records, Docker containers running
- `inspect` → accurate state
- `down` → containers stopped, DB states updated
- `resume` → containers restarted
- `destroy` → all resources cleaned, DB records gone

**VM tests** (requires libvirtd + KVM) — uses `ubuntu_linux` model:
- Gate behind runtime check for `/dev/kvm` and libvirtd
- Create manifest with Ubuntu cloud VM node → `up` → verify libvirt domain created, VM booting
- CloudInit ZTP generates valid user-data/meta-data
- `down` → VM stopped, `resume` → VM restarted, `destroy` → domain + disk cleaned up

**Mixed VM+Container tests** — uses both `ubuntu_linux` and `nokia_srlinux`:
- Manifest with both VM and container nodes → `up` → both types running
- `destroy` → all resources cleaned for both node types

**Additional P0 tests:**
- Invalid manifest rejected before resource creation
- Missing image (not imported) → error
- Partial failure triggers cleanup
- Node redeploy within running lab

**Privileges**: Lab lifecycle creates Linux bridges/veths. Run with `sudo -E cargo test`.

## Per-Crate Integration Tests (Phase 8)

Fill gaps identified in test-specs for crates that have no integration tests yet.

### Container crate (`test-specs/container/`)
```
crates/container/tests/
  lifecycle_tests.rs       — Create/start/stop/kill/remove containers
  image_tests.rs           — Pull/list/save images
  networking_tests.rs      — Bridge/macvlan network create/delete
```
Requires: Docker running. Use `sherpa-test-` prefix for all resources.

### Network crate (`test-specs/network/`)
```
crates/network/tests/
  bridge_tests.rs          — Create/delete Linux bridges
  veth_tests.rs            — Create veth pairs, enslave to bridges
  interface_tests.rs       — Interface management, fuzzy matching
```
Requires: sudo. Use `st-` prefix for interfaces.

### Libvirt crate (`test-specs/libvirt/`)
```
crates/libvirt/tests/
  vm_lifecycle_tests.rs    — Define/start/manage VMs
  disk_tests.rs            — Clone/resize/delete disks
  network_tests.rs         — Create/delete libvirt networks
  storage_pool_tests.rs    — Create/activate storage pools
```
Requires: libvirtd running, KVM available. Use `sherpa-test-` prefix.

### Template crate (`test-specs/template/`)
```
crates/template/tests/ or inline #[cfg(test)]
  vendor_config_tests.rs   — Verify each vendor template renders valid config
  cloud_init_tests.rs      — Cloud-init YAML validity
  ignition_tests.rs        — Ignition JSON validity
  infra_tests.rs           — Domain XML, dnsmasq, SSH config
```
No external deps — these are unit tests on template rendering.

### Topology crate (`test-specs/topology/`)
Inline tests in `crates/topology/src/manifest.rs` — add round-trip, minimal manifest, full manifest, invalid TOML tests. No external deps.

### Client crate (`test-specs/client/`)
Client integration tests go in `crates/server/tests/e2e/client_integration.rs` (avoids circular dep). Tests WebSocket client connection, RPC request/response, streaming via a real TestServer.

## Test Runner Script (Phase 9)

File: `scripts/run-integration-tests.sh`

```bash
#!/usr/bin/env bash
set -euo pipefail

export PATH="$HOME/.cargo/bin:$PATH"

echo "=== Prerequisites ==="
docker info > /dev/null 2>&1 || { echo "Docker not running"; exit 1; }
./dev/testdb restart

echo "=== Unit tests ==="
cargo test --workspace

echo "=== DB integration tests ==="
cargo test -p db -- --ignored

echo "=== Container integration tests ==="
cargo test -p container -- --ignored --test-threads=1

echo "=== Network integration tests ==="
sudo -E $(which cargo) test -p network -- --ignored --test-threads=1

if virsh -c qemu:///system version > /dev/null 2>&1; then
    echo "=== Libvirt integration tests ==="
    cargo test -p libvirt -- --ignored --test-threads=1
fi

echo "=== Server integration tests ==="
sudo -E $(which cargo) test -p sherpad -- --ignored --test-threads=1

echo "=== Done ==="
```

## Execution Order Summary

| Phase | What | Deps | Est. Tests |
|-------|------|------|-----------|
| 0 | VM setup | None | - |
| 1 | Existing test baseline | Phase 0 | ~118 |
| 2 | Test harness (helpers/) | Phase 0 | 0 (infra) |
| 3 | Auth + HTTP route tests | Phase 2 | ~20 |
| 4 | WebSocket RPC tests | Phase 2 | ~10 |
| 5 | User management E2E | Phase 2 | ~15 |
| 6 | Image management E2E | Phase 2 | ~12 |
| 7 | Lab lifecycle E2E | Phase 2 | ~15 |
| 8 | Per-crate integration | Phase 0 | ~40 |
| 9 | Test runner script | All | - |

## Critical Files Reference

| File | Purpose |
|------|---------|
| `crates/server/src/daemon/state.rs` | `AppState` — all fields `pub`, constructable in tests |
| `crates/server/src/api/router.rs:57` | `build_router()` — public, returns `Router<AppState>` |
| `crates/server/src/api/websocket/connection.rs:41` | `create_registry()` — creates empty `ConnectionRegistry` |
| `crates/server/src/daemon/metrics.rs:59` | `Metrics::noop()` — no-op metrics for tests |
| `crates/server/src/api/websocket/messages.rs` | `ClientMessage`/`ServerMessage` — WS protocol types |
| `crates/server/src/api/websocket/rpc.rs` | RPC method dispatch — defines all ~20 methods |
| `crates/db/tests/helper.rs` | Existing test helper pattern (namespace isolation) |
| `crates/shared/src/data/config.rs:189` | `Config` struct — no Default, must construct manually |
| `crates/libvirt/src/qemu.rs:13` | `Qemu::default()` — lazy, stores URI only |

## Risks

1. **Nested virt / KVM**: Available on this VM (confirmed `/dev/kvm` exists). VM-based tests should work.
2. **Port collisions**: Use port 0 (OS-assigned) for all test servers.
3. **Hardcoded paths**: Some services reference `/opt/sherpa/` paths from `konst.rs`. Phase 0 creates this directory structure. Test `Config` should override `images_dir`/`containers_dir`/`bins_dir` to temp dirs where possible.
4. **Privileges**: Lab lifecycle and network tests need root. Run with `sudo -E cargo test`.
5. **Test DB isolation**: Namespace-per-test pattern is proven. Each TestServer gets its own namespace.
6. **VM images**: Lab lifecycle VM tests need actual qcow2 images imported. Can create small dummy files for basic tests, or skip VM-specific lifecycle tests if no images available.
