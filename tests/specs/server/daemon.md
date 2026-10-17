# Server Daemon — Test Specifications

> **Crate:** `crates/server/` (`daemon/`)
> **External Dependencies:** Filesystem, network port binding
> **Existing Tests:** None

---

## Daemon Lifecycle

**What to test:**
- `start` launches server and binds to configured port `[integration]` **P0**
- `stop` terminates running server process `[integration]` **P0**
- `restart` stops then starts server `[integration]` **P1**
- `status` reports whether daemon is running `[integration]` **P0**
- `logs` tails daemon output `[integration]` **P2**

---

## PID File Management

**What to test:**
- PID file created on daemon start `[integration]` **P0**
- PID file removed on clean shutdown `[integration]` **P0**
- Stale PID file detected (process no longer running) `[integration]` **P1**
- Stale PID file cleaned up before new start `[integration]` **P1**

---

## AppState Initialization

**What to test:**
- AppState initialized with DB connection, JWT secret, config `[integration]` **P0**
- AppState shared across all route handlers `[integration]` **P1**
- DB connection failure during init produces clear error `[integration]` **P0**

---

## Health Check / Doctor

**What to test:**
- Doctor checks: DB connectivity, Docker daemon, libvirt daemon `[integration]` **P0**
- Doctor reports pass/fail per dependency `[integration]` **P1**
- Partial availability reported (e.g., Docker down but libvirt up) `[integration]` **P1**

---

## Server Initialization

**What to test:**
- TLS configured when certs available `[integration]` **P0**
- Plain HTTP when no certs (development mode) `[integration]` **P1**
- Port already in use produces clear error `[integration]` **P0**
