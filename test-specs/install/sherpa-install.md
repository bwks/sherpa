# Install Script — Test Specifications

> **Script:** `scripts/sherpa_install.sh`
> **External Dependencies:** Ubuntu 24.04+, root access, Docker, libvirt, internet (GitHub API + container registry)
> **Existing Tests:** None

---

## CLI & Argument Parsing

**What to test:**
- `--help` prints usage and exits 0 `[unit]` **P0**
- `--version v0.3.4` sets SHERPA_VERSION correctly `[unit]` **P0**
- Unknown flag prints error + usage, exits 1 `[unit]` **P0**
- Env vars populate defaults: `SHERPA_DB_PASSWORD`, `SHERPA_DB_PORT`, `SHERPA_SERVER_IPV4`, `SHERPA_SERVER_WS_PORT`, `SHERPA_SERVER_HTTP_PORT` `[unit]` **P0**

---

## Pre-flight Checks

### Ubuntu Version (`check_ubuntu_version`)

**What to test:**
- Fails on non-Ubuntu (ID != "ubuntu") `[unit]` **P0**
- Fails on Ubuntu < 24.04 `[unit]` **P0**
- Succeeds on Ubuntu 24.04+ `[unit]` **P0**
- Error message includes the detected version number `[unit]` **P1**
- Fails when `/etc/os-release` is absent `[unit]` **P0**

### Root Privileges (`check_root_privileges`)

**What to test:**
- Fails without root (EUID != 0) `[unit]` **P0**
- Captures SUDO_USER when run via sudo `[unit]` **P1**

### Curl Check (`check_curl_installed`)

**What to test:**
- Fails when curl not installed `[unit]` **P0**

### Port Availability (`check_port_available`)

**What to test:**
- Fails when DB_PORT already in use `[unit]` **P1**
- Falls back to netstat when ss not available `[unit]` **P2**
- Warns (but doesn't fail) when neither ss nor netstat available `[unit]` **P2**

### Virtualization (`check_virtualization`)

**What to test:**
- Fails when no vmx/svm in /proc/cpuinfo `[unit]` **P1**
- Detects Intel VT-x (vmx) `[unit]` **P1**
- Detects AMD-V (svm) `[unit]` **P1**

---

## Password Handling (`get_database_password`)

**What to test:**
- Accepts password from `SHERPA_DB_PASSWORD` env var (via `DB_PASSWORD`) `[unit]` **P0**
- Rejects empty password `[unit]` **P0**
- Rejects password shorter than 8 chars `[unit]` **P0**
- Rejects password of exactly 7 chars (boundary) `[unit]` **P0**
- Accepts password of exactly 8 chars `[unit]` **P0**
- Accepts password longer than 8 chars `[unit]` **P1**
- Interactive prompt requires confirmation match `[manual]` **P1**

---

## Server IP Validation (`get_server_ip`)

**What to test:**
- Accepts valid IPv4 from `SHERPA_SERVER_IPV4` env var `[unit]` **P0**
- Accepts `0.0.0.0` (all interfaces) `[unit]` **P0**
- Rejects non-IPv4 string (e.g., "notanip") `[unit]` **P0**
- Rejects a hostname (e.g., "myserver.example.com") `[unit]` **P0**
- Rejects an empty / whitespace-only string `[unit]` **P1**
- Defaults to 0.0.0.0 when no input provided `[manual]` **P1**
- Known gap: accepts out-of-range octets (e.g. `999.999.999.999`) `[unit]` **P1** *(documents existing behaviour — should fail once fixed)*

**Known issue:** Regex `^[0-9]+\.[0-9]+\.[0-9]+\.[0-9]+$` accepts invalid octets like `999.999.999.999`. Consider adding octet range validation.

---

## Package Installation (`install_system_packages`)

**What to test:**
- All expected packages are passed to apt-get install `[integration]` **P1**
- DEBIAN_FRONTEND=noninteractive is set `[unit]` **P1**

---

## Docker Installation (`install_docker`)

**What to test:**
- Skips installation if docker already present (idempotent) `[integration]` **P0**
- Enables and starts Docker service `[integration]` **P1**

---

## Libvirt (`enable_libvirtd`)

**What to test:**
- Enables and starts libvirtd service `[integration]` **P1**

---

## User & Group Setup (`setup_sherpa_user`)

**What to test:**
- Creates sherpa system user if not exists `[integration]` **P0**
- Idempotent — doesn't fail if user already exists `[integration]` **P0**
- Adds sherpa to libvirt, kvm, docker groups `[integration]` **P1**
- Adds SUDO_USER to sherpa group `[integration]` **P1**
- Shell is /usr/sbin/nologin (system account) `[integration]` **P1**

---

## Directory Setup (`setup_directories`)

**What to test:**
- Creates /opt/sherpa/{db,config,env} `[integration]` **P0**
- Idempotent — doesn't fail if dirs already exist `[integration]` **P0**
- Sets correct ownership (sherpa:sherpa) `[integration]` **P0**
- Sets correct permissions: 775 for base/db/config, 750 for env `[integration]` **P0**

---

## Container Management

### Stop Existing (`stop_existing_container`)

**What to test:**
- Stops and removes existing container before creating new one `[integration]` **P1**
- No-op when no existing container `[integration]` **P1**

### Pull Images (`pull_surrealdb_image`, `pull_sherpa_router_image`)

**What to test:**
- Pulls SurrealDB image at correct version (v3.0.0) `[integration]` **P0**
- Pulls Sherpa Router image `[integration]` **P0**
- Exits on pull failure `[integration]` **P1**

### Start Container (`start_container`)

**What to test:**
- Starts container with correct port mapping (DB_PORT:8000) `[integration]` **P0**
- Mounts SHERPA_DB_DIR as /data volume `[integration]` **P0**
- Runs as sherpa UID/GID `[integration]` **P0**
- Uses rocksdb backend `[integration]` **P1**

### Health Check (`wait_for_database`)

**What to test:**
- Succeeds within 30s for healthy DB `[integration]` **P0**
- Fails after 30 attempts if DB never healthy `[integration]` **P1**
- Detects container that stopped unexpectedly `[integration]` **P1**

---

## Binary Installation (`install_binaries`)

**What to test:**
- Fetches latest version from GitHub API when no --version `[integration]` **P0**
- Uses specified version with --version flag `[integration]` **P0**
- Downloads and extracts sherpad tarball `[integration]` **P0**
- Installs to /opt/sherpa/bin/ with 755 permissions `[integration]` **P0**
- Creates symlinks in /usr/local/bin `[integration]` **P0**
- Fails if required binary (sherpad) not in archive `[integration]` **P1**
- Skips optional binary (sherpa) if not available `[integration]` **P1**
- Stops existing sherpad process/service before overwriting `[integration]` **P1**
- Force-kills sherpad if graceful stop fails `[integration]` **P2**
- Temp directory cleaned up on return `[unit]` **P2**

---

## Systemd Service (`install_systemd_service`)

**What to test:**
- Writes valid unit file to /etc/systemd/system/sherpad.service `[integration]` **P0**
- Unit file references correct binary path (/opt/sherpa/bin/sherpad) `[integration]` **P0**
- Unit file requires docker.service and libvirtd.service `[integration]` **P1**
- Creates env file with correct password, IP, ports `[integration]` **P0**
- Env file has restricted permissions (640, sherpa:sherpa) `[integration]` **P0**
- Creates env example file `[integration]` **P2**
- Installs logrotate config for /opt/sherpa/logs/sherpad.log `[integration]` **P1**
- Runs daemon-reload after writing unit file `[integration]` **P1**
- Enables service (but does not start it) `[integration]` **P0**
- Skips gracefully when systemctl not available `[integration]` **P2**

---

## Error Cleanup (`cleanup_on_error`)

**What to test:**
- On failure: container is stopped and removed `[integration]` **P1**
- On failure: directories and users are NOT removed `[integration]` **P1**
- On success: error trap is removed (clean exit) `[unit]` **P2**

---

## Documentation Bugs

These are not test cases but issues found during review:

1. **Dead `--db-pass` flag**: Header comment (line 18) mentions `--db-pass` but the arg parser (lines 1006-1022) only handles `--version` and `--help`. The env var `SHERPA_DB_PASSWORD` works, but the CLI flag does not exist.
2. **Weak IPv4 validation**: Regex `^[0-9]+\.[0-9]+\.[0-9]+\.[0-9]+$` accepts `999.999.999.999`. Should validate each octet is 0-255.
3. **Password complexity gap**: Script only checks length (>= 8 chars). The Rust-side `validate_password_strength()` requires uppercase, lowercase, and special characters. Consider aligning the two.
4. **Port check ordering**: `check_port_available` runs after `install_system_packages` and `install_docker`. If Docker or another package binds the target port, the user has already waited through a long apt install before being told the port is in use. Consider moving the port check to pre-flight.
5. **`cleanup_on_error` calls `docker` without a guard**: When the install fails before Docker is installed (e.g., at the Ubuntu version check), `cleanup_on_error` runs `docker ps -a ...` which prints `docker: command not found` to stderr. The `2>/dev/null` on the pipe only silences `grep`'s stderr, not `docker`'s. Fix: add `2>/dev/null` to the `docker ps` call itself, or guard with `command -v docker >/dev/null 2>&1`.
6. **`check_port_available` breaks idempotent re-runs**: When sherpa-db is already running and bound to `DB_PORT`, a second invocation of the installer fails at `check_port_available` before it can reach `stop_existing_container`. The fix is to check whether the port is held by the sherpa-db container itself and skip the error in that case (or move `stop_existing_container` before the port check).
7. **`setup_sherpa_user` does not enforce shell on existing user**: If a user named `sherpa` already exists (e.g., as a regular login user with `/bin/bash`), the script detects the existing account and skips user creation, leaving the shell unchanged. On a fresh install the system user is created with `/usr/sbin/nologin`. Consider adding a check to update the shell if the existing account has an interactive shell.
