# WebSocket RPC — Test Specifications

> **Crate:** `crates/server/` (`api/websocket/`)
> **External Dependencies:** Running server with SurrealDB, Docker, libvirt
> **Existing Tests:** None

---

## RPC Dispatch

**What to test:**
- Valid JSON-RPC 2.0 request dispatched to correct handler `[integration]` **P0**
- Unknown method returns JSON-RPC error with method-not-found code `[integration]` **P0**
- Malformed JSON request returns parse error `[integration]` **P0**
- Missing required parameters return invalid-params error `[integration]` **P0**
- Response ID matches request ID `[integration]` **P0**

---

## Authentication Methods

**What to test:**
- `auth.login` with valid credentials returns token `[integration]` **P0**
- `auth.login` with invalid credentials returns error `[integration]` **P0**
- `auth.validate` with valid token returns user info `[integration]` **P0**
- `auth.validate` with expired token returns error `[integration]` **P0**

---

## Lab Operations (Streaming)

**What to test:**
- `up` creates lab and streams progress messages `[integration]` **P0**
- `up` with invalid manifest returns validation error `[integration]` **P0**
- `destroy` tears down lab and streams progress `[integration]` **P0**
- `destroy` validates lab ownership `[integration]` **P0**
- `redeploy` recreates specific node with streaming `[integration]` **P1**
- Streaming messages arrive in correct phase order `[integration]` **P1**
- Final response includes summary data `[integration]` **P1**

---

## Lab Operations (Request/Response)

**What to test:**
- `labs.list` returns labs for the authenticated token user `[integration]` **P0**
- `labs.list` rejects missing or invalid tokens `[integration]` **P0**
- `inspect` returns lab state (devices, links, bridges) `[integration]` **P0**
- `inspect` validates lab ownership `[integration]` **P0**
- `down` stops all or specific nodes `[integration]` **P0**
- `resume` starts all or specific nodes `[integration]` **P0**

---

## Image Operations

**What to test:**
- `image.list` returns images (with optional filters) `[integration]` **P0**
- `image.show` returns full NodeConfig for default version of a model `[integration]` **P0**
- `image.show` with version param returns specific version details `[integration]` **P1**
- `image.show` returns error when model has no images `[integration]` **P0**
- `image.show` returns error when specified version not found `[integration]` **P1**
- `image.import` registers image and tracks in DB `[integration]` **P1**
- `image.scan` discovers images on disk/Docker `[integration]` **P1**
- `image.delete` removes image (blocked if in use) `[integration]` **P0**
- `image.set_default` changes default version `[integration]` **P1**
- `image.pull` pulls container image with streaming progress `[integration]` **P1**
- `image.download` downloads VM image with streaming progress `[integration]` **P1**

---

## User Management (Admin Only)

**What to test:**
- `user.create` creates new user `[integration]` **P0**
- `user.list` returns all users `[integration]` **P0**
- `user.delete` removes user `[integration]` **P0**
- `user.passwd` changes user password `[integration]` **P1**
- `user.info` returns user details `[integration]` **P1**
- All user methods reject non-admin callers `[integration]` **P0**

---

## Admin Operations

**What to test:**
- `clean` performs admin lab cleanup without ownership check `[integration]` **P1**
- `clean` rejects non-admin callers `[integration]` **P0**

---

## Connection Management

**What to test:**
- WebSocket connection established with initial "connected" message `[integration]` **P1**
- Multiple concurrent connections handled `[integration]` **P1**
- Client disconnect cleaned up properly `[integration]` **P2**
- Ping/pong keepalive functioning `[integration]` **P2**
