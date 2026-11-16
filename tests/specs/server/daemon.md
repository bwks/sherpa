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
- A missing built-in `admin` account stops startup with a pointer to `sherpad init`, unless `SHERPA_ADMIN_PASSWORD` is set, which creates it `[integration]` **P0** (`startup_requires_the_admin_account`)
- A `sherpa.toml` with the removed `[ztp_server]` table still loads, and startup logs one warning that it is ignored and can be deleted; a file without it logs nothing `[unit]` **P1** (`a_config_with_the_removed_ztp_server_table_still_loads`, `daemon::removed_settings`)

---

## `sherpad init` Accounts

**What to test:**
- Refuses to run as any account but `sherpa`, root included, before writing anything, and names `sudo -u sherpa -H sherpad init` `[unit]` **P0** (`validate::init_account`)
- `default_config()` writes no `[ztp_server]` table and no password `[unit]` **P0** (`the_default_config_has_no_ztp_server_table`)
- Creates `admin` with the password from the `SHERPA_ADMIN_PASSWORD` environment variable or a prompt `[installer]` **P0**
- Asks for the first user account: username, password, administrator flag, optional SSH key `[installer]` **P0**
- Re-running keeps existing accounts unchanged and skips the first-user step when a user exists `[installer]` **P0**
