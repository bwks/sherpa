# Install Script — Test Specifications

> **Script:** `scripts/sherpa_install.sh`
> **External Dependencies:** Ubuntu 24.04+, root access, Docker, libvirt, internet (GitHub API + container registry)
> **Existing Tests:** `test-scripts/install_tests.bats`, `scripts/test_install.sh`

---

## CLI & Argument Parsing

**What to test:**
- `--help` prints usage and exits 0 `[unit]` **P0**
- `--version v0.3.4` sets SHERPA_VERSION correctly `[unit]` **P0**
- Unknown flag prints error + usage, exits 1 `[unit]` **P0**
- Missing `--version` value fails before any system changes `[unit]` **P0**
- `SHERPA_ARTIFACT_DIR` selects local release archives with an explicit version `[e2e]` **P0**
- A missing required local artifact fails before package or container changes `[unit]` **P0**
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
- Rejects out-of-range octets (e.g. `999.999.999.999`) `[unit]` **P1**

---

## Package Installation (`install_system_packages`)

**What to test:**
- All expected packages are passed to apt-get install `[integration]` **P1**
- Select concrete `qemu-system-x86` instead of the ambiguous Ubuntu 26.04 `qemu-kvm` virtual package `[unit]` **P0**
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

## Uninstallation

The VM runner checks the actual script exit status and resulting state. Default
server removal retains the CLI; full removal removes both binaries. Docker,
libvirt, their images and resources, and users/groups are retained in all modes.

- Remove the server service, binary and its owned symlink `[e2e]` **P0**
- Keep data: retain the database, server configuration and SSH identity, then
  reinstall and authenticate the persisted test admin `[e2e]` **P0**
- Remove data: empty the database while retaining configuration and identity `[e2e]` **P0**
- Full removal: remove the installation directory and both owned symlinks `[e2e]` **P0**
- Preserve unrelated files and symlinks at binary locations `[unit]` **P0**
- Repeated full uninstall succeeds when the installation is already absent `[e2e]` **P0**
- Compare package, user/group, Docker image/volume/unrelated container and libvirt
  domain/network/pool inventories before and after each uninstall mode `[e2e]` **P0**
- Record retained libvirt definitions even when full removal deletes their backing
  installation directory; do not assume uninstall restores bare Ubuntu `[e2e]` **P1**

## VM harness evidence and failure handling

### Upgrade from the previous release

- Install explicitly selected previous-release binaries in a fresh disposable
  guest using the recorded checkout installer; record both baseline and candidate
  archive/binary hashes and the installer hash `[e2e]` **P0**
- Reject equal baseline/candidate versions, identical binary bytes and upgrade
  runs without supplied local candidate archives before provisioning `[unit]` **P0**
- Initialize the baseline, authenticate, and create an additional non-admin user
  through the API; retain both user records, configuration, environment, SSH
  identity and TLS certificate fingerprints `[e2e]` **P0**
- Upgrade the same guest with the real installer and verify candidate versions
  and binary hashes; authenticate both persisted users and compare retained state
  before and after upgrade and again after reboot `[e2e]` **P0**
- Environment comparisons ignore generated comments and blank lines but detect
  changed assignments, including database credentials and listen settings `[unit]` **P0**
- Upgrade guests must pass a bounded read-only SSH readiness wait before workspace
  creation; transient refusal can recover, and continued refusal fails before
  any installation commands `[unit]` **P0**
- Missing or changed upgrade evidence fails the run; installer failures/timeouts
  retain a failed receipt and diagnostics with the existing targeted cleanup
  policy `[unit]` **P0**

- Capture server systemd journal entries and database container logs before each
  uninstall; foreground service logging does not require a file log `[e2e]` **P0**
- An actual command timeout stops the guest probe, fails the run and records owned
  resources for verified cleanup `[e2e]` **P0**
- SIGTERM interrupts the runner and stops its active guest probe; retain a failed
  receipt before attempting diagnostics and targeted cleanup `[e2e]` **P0**
- Incomplete provisioning must be rejected before installation, including when
  `sherpa up` reports readiness warnings with exit status zero `[e2e]` **P0**
- Cleanup refuses changed lab identity or domain UUID, handles missing local SSH
  files, verifies owned resource removal and rechecks the base image `[e2e]` **P0**
- Local verification requires all lifecycle steps, pre-uninstall diagnostics,
  matching script/archive/binary hashes and verified cleanup `[unit]` **P0**
- Failed, timed-out, incomplete, published-baseline and dirty-checkout receipts
  cannot pass strict local verification `[unit]` **P0**
- Shared evidence excludes credentials, connection settings and SSH private keys
  while preserving the required inventories and provenance `[unit]` **P0**

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

## Documentation Bugs (resolved)

These issues were found during review and have been fixed:

1. ~~**Dead `--db-pass` flag**: Header comment mentioned `--db-pass` but the arg parser only handles `--version` and `--help`.~~ **Fixed** — comment updated to show correct usage.
2. ~~**Weak IPv4 validation**: Regex accepted `999.999.999.999`.~~ **Fixed** — added octet range validation (0-255).
3. **Password complexity gap**: Script only checks length (>= 8 chars). The Rust-side `validate_password_strength()` requires uppercase, lowercase, and special characters. Consider aligning the two. *(Deferred — requires a design decision on whether to enforce complexity in the installer.)*
4. ~~**Port check ordering**: `check_port_available` ran after `install_system_packages` and `install_docker`.~~ **Fixed** — moved to pre-flight, before package installation.
5. ~~**`cleanup_on_error` calls `docker` without a guard**.~~ **Fixed** — guarded with `command -v docker >/dev/null 2>&1`.
6. ~~**`check_port_available` breaks idempotent re-runs**.~~ **Fixed** — when sherpa-db container owns the port, the check passes and the container is replaced later.
7. ~~**`setup_sherpa_user` does not enforce shell on existing user**.~~ **Fixed** — system users (uid < 1000) now get their shell updated to `/usr/sbin/nologin` if it differs.
