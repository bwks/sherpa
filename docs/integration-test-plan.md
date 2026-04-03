# Integration Test Implementation Plan

## Current Status

| Phase | Status | Notes |
|-------|--------|-------|
| 0 — Environment Setup | **DONE** | libvirtd running, sherpa-pool created, SSH keys generated, images pulled, dirs created |
| 1 — Baseline | **DONE** | Unit tests pass. DB integration tests pass (177 tests) |
| 2 — Test Harness | **DONE** | TestServer, TestWsClient, TestHttpClient in `crates/server/tests/helpers/` |
| 3 — Auth + HTTP Tests | **DONE** | 15 tests passing in `crates/server/tests/auth_tests.rs` |
| 4 — WebSocket RPC Tests | **DONE** | 6 tests passing in `crates/server/tests/websocket_tests.rs` |
| 5 — User Management E2E | **DONE** | 9 tests passing in `crates/server/tests/user_management_tests.rs` |
| 6 — Image Management E2E | **DONE** | 7 tests passing in `crates/server/tests/image_management_tests.rs` |
| 7 — Lab Lifecycle E2E | **DONE** | 7 tests passing in `crates/server/tests/lab_lifecycle_tests.rs` |
| 8 — Per-Crate Integration | **ALREADY EXISTS** | container, network, libvirt, template, topology all have tests |
| 9 — Test Runner Script | **DONE** | `scripts/run-integration-tests.sh` |

**Total server integration tests: 44 passing (0 failing)**

---

## Environment Prerequisites

The following must be set up before running integration tests:

### SurrealDB (test database)
```bash
# Start with sherpa credentials (not default root/root)
SHERPA_DEV_DB_USER=sherpa SHERPA_DEV_DB_PASS="Everest1953!" ./dev/testdb start
```

### Docker infrastructure images
```bash
docker pull ghcr.io/bwks/sherpa-router:latest
docker pull ghcr.io/bwks/webdir:latest
docker pull ghcr.io/nokia/srlinux:latest
docker pull alpine:latest
```

### Libvirt storage pool
```bash
sudo mkdir -p /opt/sherpa/libvirt/images
sudo chown libvirt-qemu:kvm /opt/sherpa/libvirt /opt/sherpa/libvirt/images
sudo chmod 775 /opt/sherpa/libvirt /opt/sherpa/libvirt/images
virsh pool-define-as sherpa-pool dir --target /opt/sherpa/libvirt/images
virsh pool-start sherpa-pool
virsh pool-autostart sherpa-pool
```

### Sherpa SSH key
```bash
sudo mkdir -p /opt/sherpa/ssh
sudo ssh-keygen -t ed25519 -f /opt/sherpa/ssh/sherpa_ssh_key -N "" -C "sherpa@test"
sudo chown -R sherpa:sherpa /opt/sherpa/ssh
```

### Sherpa config file
```bash
# /opt/sherpa/config/sherpa.toml
cat > /opt/sherpa/config/sherpa.toml << 'EOF'
name = "test-server"
server_ipv4 = "127.0.0.1"
ws_port = 8080
http_port = 8081
vm_provider = "libvirt"
qemu_bin = "/usr/bin/qemu-system-x86_64"
management_prefix_ipv4 = "10.200.0.0/16"
images_dir = "/opt/sherpa/images"
containers_dir = "/opt/sherpa/containers"
bins_dir = "/opt/sherpa/bins"
EOF
```
> **Note**: The management prefix MUST NOT overlap with host interfaces. `10.200.0.0/16` avoids the host `enp3s0` (`172.31.1.x`) and Docker (`172.17.0.0/16`).

### Ubuntu VM image
```bash
mkdir -p /opt/sherpa/images/ubuntu_linux/24.04
wget -O /opt/sherpa/images/ubuntu_linux/24.04/virtioa.qcow2 \
  https://cloud-images.ubuntu.com/noble/current/noble-server-cloudimg-amd64.img
```

### Directory structure
```bash
sudo mkdir -p /opt/sherpa/{config,env,ssh,images,containers,bins,labs,run,logs,.certs,.secret}
sudo chown -R sherpa:sherpa /opt/sherpa
```

---

## How to Run Tests

```bash
export PATH="$HOME/.cargo/bin:$PATH"
cd /home/sherpa/code/rust/sherpa

# Ensure testdb is running with correct credentials
SHERPA_DEV_DB_USER=sherpa SHERPA_DEV_DB_PASS="Everest1953!" ./dev/testdb start

# 1. Unit tests (no external deps)
cargo test --workspace

# 2. DB integration tests
cargo test -p db -- --ignored

# 3. Server integration tests (auth, websocket, user, image management)
sudo -E env PATH="$HOME/.cargo/bin:$PATH" cargo test -p sherpad -- --ignored --test-threads=1

# 4. Per-crate integration tests
cargo test -p container -- --ignored --test-threads=1
sudo -E env PATH="$HOME/.cargo/bin:$PATH" cargo test -p network -- --ignored --test-threads=1
cargo test -p libvirt -- --ignored --test-threads=1
```

### Important notes

- Lab lifecycle tests need **sudo** (Linux bridge creation, libvirt network creation)
- Use `--test-threads=1` for all ignored tests to prevent resource conflicts
- The management prefix in `TestServer` is `10.200.0.0/16` — do NOT change to `172.31.0.0/16` as it conflicts with `enp3s0`
- After test failures, clean up lingering resources:
  ```bash
  # Docker containers and networks
  docker ps -a | grep -v "sherpa-test-db" | awk 'NR>1{print $1}' | xargs docker rm -f
  docker network ls | grep -v "bridge\|host\|none" | awk 'NR>1{print $1}' | xargs docker network rm

  # Libvirt networks
  virsh net-list | grep sherpa | awk '{print $1}' | xargs -I{} sh -c 'virsh net-destroy {}; virsh net-undefine {}'
  ```

---

## Context

The Sherpa project has comprehensive test specifications in `test-specs/` but most integration tests are unimplemented. Only the `db` crate had integration tests initially (28 files). All server integration tests are now implemented and passing.

## What Was Done (Code Changes)

1. **`crates/server/src/lib.rs`** (NEW) — Exposes server internals for integration tests
2. **`crates/server/src/main.rs`** — Refactored to use `sherpad::` imports from lib.rs
3. **`crates/server/Cargo.toml`** — Added `tokio-tungstenite` and `reqwest` dev-deps
4. **`crates/server/tests/helpers/`** — TestServer, TestWsClient, TestHttpClient
5. **`crates/server/tests/auth_tests.rs`** — 15 tests (JWT, cookie, HTTP routes)
6. **`crates/server/tests/websocket_tests.rs`** — 6 tests (connection, dispatch, auth methods)
7. **`crates/server/tests/user_management_tests.rs`** — 9 tests (CRUD, passwords, permissions)
8. **`crates/server/tests/image_management_tests.rs`** — 7 tests (scan, list, show, import, pull)
9. **`crates/server/tests/lab_lifecycle_tests.rs`** — 7 tests (container/VM up/down/destroy, errors)
10. **`crates/server/src/api/extractors.rs`** — Removed non-compiling docstring code examples
11. **`dev/testdb`** — Updated port mapping for SurrealDB v3.0.0
12. **`scripts/run-integration-tests.sh`** (NEW) — Runs all tests in order
13. **`crates/db/src/seed/admin_user.rs`** — Fixed hardcoded port 8000 → env var (SHERPA_DEV_DB_PORT)

## Key Design Decisions

- **Management prefix**: `10.200.0.0/16` avoids conflict with host `enp3s0` (`172.31.1.0/24`)
- **lab_id format**: Must be exactly 8 characters (DB constraint). Tests use `tc{pid%1M:06}`, `dr{pid%1M:06}`, etc.
- **Login response**: Uses HTMX pattern — 200 OK with `hx-redirect` header (not 302 redirect)
- **user.info response**: Wrapped as `{ user: UserInfo }` (GetUserInfoResponse)
- **user.list response**: Wrapped as `{ users: [UserInfo] }` (ListUsersResponse)
- **image.pull**: Requires `model`, `repo`, and `tag` fields (ContainerPullRequest)
- **image.scan**: Does NOT require status messages — empty scan is valid on clean environments

## Baseline Test Counts

| Suite | Tests |
|-------|-------|
| Unit tests (workspace) | ~600 |
| DB integration tests | 177 |
| Auth + HTTP (Phase 3) | 15 |
| WebSocket RPC (Phase 4) | 6 |
| User Management E2E (Phase 5) | 9 |
| Image Management E2E (Phase 6) | 7 |
| Lab Lifecycle E2E (Phase 7) | 7 |
| **Server integration total** | **44** |

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
| `crates/shared/src/konst.rs:56` | `SHERPA_STORAGE_POOL = "sherpa-pool"` (libvirt pool name) |
| `crates/shared/src/konst.rs:57` | `SHERPA_STORAGE_POOL_PATH = "/opt/sherpa/libvirt/images"` |
| `crates/shared/src/konst.rs:49` | `SHERPA_SSH_PUBLIC_KEY_PATH = "/opt/sherpa/ssh/sherpa_ssh_key.pub"` |

## Test Phases (Detail)

### Phase 3 — Auth + HTTP Tests (`auth_tests.rs`)

Tests: auth.login (valid/invalid/missing), auth.validate, cookie login (HTMX pattern), HTTP route protection, API spec endpoint, admin RPC access control.

### Phase 4 — WebSocket RPC Tests (`websocket_tests.rs`)

Tests: WS connection message, multiple connections, RPC dispatch, unknown method error, ID matching, auth.login, auth.validate.

### Phase 5 — User Management E2E (`user_management_tests.rs`)

Tests: create user, duplicate username, non-admin create denied, user info (GetUserInfoResponse.user), password change, user list (ListUsersResponse.users), delete user, last admin protection, non-admin delete denied.

### Phase 6 — Image Management E2E (`image_management_tests.rs`)

Tests: image scan (no status messages required on clean env), image list, image show (ShowImageResponse.image), set default, import nonexistent file error, pull container image (repo+tag required), admin-only access.

### Phase 7 — Lab Lifecycle E2E (`lab_lifecycle_tests.rs`)

Tests: container lab up+inspect+destroy, down+resume cycle, destroy verification, VM lab up+destroy, invalid manifest error, missing image error, auth required.

Prerequisites met by `bootstrap_images()` calling `image.scan` before each test.

## Per-Crate Integration Tests (Phase 8)

Existing tests in the following crates (already implemented, not new work):
- `crates/container/tests/` — container lifecycle, image, networking
- `crates/network/tests/` — bridge, veth, interface tests (needs sudo)
- `crates/libvirt/tests/` — VM lifecycle, disk, network, storage pool (needs libvirtd + KVM)
- `crates/template/` — template rendering tests
- `crates/topology/` — manifest parsing tests

## Test Runner Script (Phase 9)

File: `scripts/run-integration-tests.sh`

```bash
#!/usr/bin/env bash
set -euo pipefail

export PATH="$HOME/.cargo/bin:$PATH"

echo "=== Prerequisites ==="
docker info > /dev/null 2>&1 || { echo "Docker not running"; exit 1; }
SHERPA_DEV_DB_USER=sherpa SHERPA_DEV_DB_PASS="Everest1953!" ./dev/testdb restart

echo "=== Unit tests ==="
cargo test --workspace

echo "=== DB integration tests ==="
cargo test -p db -- --ignored

echo "=== Container integration tests ==="
cargo test -p container -- --ignored --test-threads=1

echo "=== Network integration tests ==="
sudo -E env PATH="$HOME/.cargo/bin:$PATH" cargo test -p network -- --ignored --test-threads=1

if virsh -c qemu:///system version > /dev/null 2>&1; then
    echo "=== Libvirt integration tests ==="
    cargo test -p libvirt -- --ignored --test-threads=1
fi

echo "=== Server integration tests ==="
sudo -E env PATH="$HOME/.cargo/bin:$PATH" cargo test -p sherpad -- --ignored --test-threads=1

echo "=== Done ==="
```
