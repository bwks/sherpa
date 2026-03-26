# Client CLI Commands — Test Specifications

> **Crate:** `crates/client/`
> **External Dependencies:** Running Sherpa server, WebSocket connectivity, filesystem (~/.sherpa/)
> **Existing Tests:** 6 unit tests in `cmd/up.rs` (ZTP config resolution, startup scripts, env var expansion)

---

## `up` — Start Lab

**What to test:**
- Manifest loaded and parsed from TOML file `[unit]` **P0**
- ZTP config files resolved: read from disk, base64 encoded, attached to nodes `[unit]` **P0**
- Startup script files resolved: read from disk, base64 encoded as StartupScript structs `[unit]` **P0**
- Environment variables with `$VAR` references resolved from host environment `[unit]` **P0**
- Missing ZTP config file produces clear error `[unit]` **P0**
- Empty ZTP config file produces clear error `[unit]` **P1**
- Missing startup script file produces clear error `[unit]` **P0**
- Unresolvable `$VAR` reference produces clear error `[unit]` **P1**
- Authentication token loaded before RPC call `[integration]` **P0**
- Missing token directs user to `sherpa login` `[integration]` **P0**
- Streaming progress messages displayed during lab creation `[integration]` **P1**
- SSH config and private key written locally on success `[integration]` **P1**
- lab-info.toml written on success `[integration]` **P1**
- Results displayed in table format `[integration]` **P2**
- 15-minute extended timeout used for long operations `[unit]` **P1**

**Existing coverage:** 6 tests cover ZTP resolution, startup scripts, env var expansion

---

## `down` — Stop Lab/Node

**What to test:**
- All nodes stopped when no `--node` flag provided `[integration]` **P0**
- Specific node stopped when `--node <name>` provided `[integration]` **P0**
- Missing auth token produces login prompt `[integration]` **P0**
- Per-node results (success/error) displayed `[integration]` **P1**

**Existing coverage:** None

---

## `destroy` — Destroy Lab

**What to test:**
- Lab inspected and displayed before destruction `[integration]` **P0**
- User prompted for confirmation (accepts "y"/"yes") `[unit]` **P0**
- `--yes` flag skips confirmation `[unit]` **P0**
- Streaming progress messages during destruction `[integration]` **P1**
- Local SSH config and key files cleaned up after destruction `[integration]` **P1**
- Missing auth token produces login prompt `[integration]` **P0**

**Existing coverage:** None

---

## `validate` — Validate Manifest

**What to test:**
- Valid manifest passes all checks `[unit]` **P0**
- Duplicate device names detected `[unit]` **P0**
- Interface bounds violations detected per device model `[unit]` **P0**
- Management interface overlap detected (when not dedicated) `[unit]` **P0**
- Duplicate interface usage across links and bridges detected `[unit]` **P0**
- Undefined devices in links detected `[unit]` **P0**
- Undefined devices in bridges detected `[unit]` **P0**
- No server connection required (offline validation) `[unit]` **P0**

**Existing coverage:** None

---

## `inspect` — Inspect Lab

**What to test:**
- Lab info, devices, links, and bridges displayed in tables `[integration]` **P0**
- Inactive devices listed separately `[integration]` **P1**
- Missing auth token produces login prompt `[integration]` **P0**

**Existing coverage:** None

---

## `login` / `logout` / `whoami`

**What to test:**
- Login with valid credentials saves token to disk `[integration]` **P0**
- Login with invalid credentials produces error `[integration]` **P0**
- Empty username/password rejected before RPC call `[unit]` **P0**
- Logout removes token file (idempotent) `[unit]` **P0**
- Whoami displays username, admin status, token expiry `[integration]` **P1**
- Whoami with expired token reports expiry `[integration]` **P1**

**Existing coverage:** None

---

## `init` — Client Initialization

**What to test:**
- Interactive prompts for server IP, IPv6, port `[integration]` **P1**
- Default values applied when user enters nothing `[unit]` **P1**
- Invalid IPv4/IPv6 address rejected with re-prompt `[unit]` **P0**
- Invalid port (0, 65536+) rejected `[unit]` **P0**
- Config written to ~/.sherpa/config `[integration]` **P1**
- `--force` overwrites existing config `[integration]` **P1**
- Without `--force`, existing config not overwritten `[integration]` **P1**

**Existing coverage:** None

---

## `new` — Generate Example Manifest

**What to test:**
- Example manifest.toml written with valid TOML content `[unit]` **P0**
- `--force` overwrites existing manifest `[unit]` **P1**
- Without `--force`, existing file not overwritten `[unit]` **P1**
- Generated manifest is parseable by topology crate `[unit]` **P1**

**Existing coverage:** None

---

## `image` — Image Management

**What to test:**
- List all images without filters `[integration]` **P0**
- Filter by model `[integration]` **P1**
- Filter by kind (container, VM, unikernel) `[integration]` **P1**
- Empty result set produces warning message `[integration]` **P2**
- Show default image details for a model `[integration]` **P0**
- Show specific version image details with `--version` flag `[integration]` **P1**
- Show image for nonexistent model produces error `[integration]` **P0**
- Show image when no default version exists produces error `[integration]` **P1**
- Show image with nonexistent version produces error `[integration]` **P1**
- Show image displays all NodeConfig fields in key-value table `[integration]` **P1**

**Existing coverage:** None

---

## `console` — Serial Console

**What to test:**
- Device name mapped to correct loopback IP `[unit]` **P0**
- "boot-server" maps to device ID 255 `[unit]` **P1**
- Missing lab-info.toml produces clear error `[unit]` **P0**
- Unknown device name produces error `[unit]` **P0**
- Telnet command invoked with correct address `[integration]` **P1**

**Existing coverage:** None

---

## `ssh` — SSH to Node

**What to test:**
- SSH invoked with correct node name and sherpa-ssh-config `[integration]` **P0**
- Missing SSH config file handled gracefully `[integration]` **P1**

**Existing coverage:** None

---

## `resume` / `redeploy`

**What to test:**
- Resume all nodes or specific node `[integration]` **P0**
- Redeploy rebuilds node with fresh config `[integration]` **P0**
- Missing auth token handled `[integration]` **P0**
- Streaming progress for redeploy `[integration]` **P1**

**Existing coverage:** None

---

## `server` — Admin Subcommands

**What to test:**
- `server status` shows daemon state `[integration]` **P1**
- `server clean` sends admin cleanup request `[integration]` **P1**
- `server user` subcommands (create, list, delete) `[integration]` **P1**
- `server image` subcommands (show, import, scan, delete) `[integration]` **P1**
- Admin-only commands fail for non-admin users `[integration]` **P0**

**Existing coverage:** None
