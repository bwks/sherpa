# Sherpa Scripts

This directory contains scripts for Sherpa installation, maintenance, and development.

## Installation Scripts

### `sherpa_install.sh`
Installs the Sherpa server binaries, Docker, QEMU/libvirt and the Sherpa router container image.
sherpad keeps its database in `/opt/sherpa/db/sherpa.sqlite`; no database server is installed.

**Usage:**
```bash
sudo ./scripts/sherpa_install.sh

# View help
./scripts/sherpa_install.sh --help
```

**What it does:**
- Installs Docker from Docker's apt repository, after checking Docker's signing key against the
  SHA-256 the script pins
- Checks each release archive against the release's `SHA256SUMS` before using it, and refuses a
  release that publishes none
- Creates sherpa user and groups
- Sets up directory structure at `/opt/sherpa/`
- Creates the blank disk templates in `/opt/sherpa/images/blank_disk/` that node ZTP disks are
  copied from, each only if it is missing (see "Blank disks" in `docs/architecture/ZTP.md`)
- Pulls the Sherpa router container image by the digest the script pins, tagged `:latest`
- Installs the binaries, the sherpad systemd service and its environment file
- Refuses a server older than 0.5.0 (a `sherpa-db` SurrealDB container exists)

**Requirements:**
- Ubuntu 24.04 or later, curl, virtualization support and internet access
- Root/sudo privileges

### `sherpa_update.sh`
Replaces the Sherpa binaries with the latest, or a given, release and restarts sherpad. It
refuses, before changing anything, a server older than 0.5.0 whose data is still in SurrealDB:
such a server updates to 0.5.12 first with that release's update script, which moves the data.

It checks each archive it downloads against the release's `SHA256SUMS`, like the install
script.

On every run, including one with nothing to install, it creates the blank disk templates that
are missing, installing `qemu-utils`, `dosfstools` and `e2fsprogs` first if a tool is missing.
Existing disks are never changed. Running it again is how a server gets a disk that was removed.

### `sherpa_client_install.ps1`
Installs or updates the Sherpa CLI client on Windows. No administrator privileges required.

**One-liner install (PowerShell):**
```powershell
irm https://raw.githubusercontent.com/bwks/sherpa/main/scripts/sherpa_client_install.ps1 -OutFile "$env:TEMP\sherpa_install.ps1"; powershell -ExecutionPolicy Bypass -File "$env:TEMP\sherpa_install.ps1"; Remove-Item "$env:TEMP\sherpa_install.ps1"
```

**Usage (local):**
```powershell
# Install latest version
.\scripts\sherpa_client_install.ps1

# Install specific version
.\scripts\sherpa_client_install.ps1 -Version v0.3.35

# Update existing installation to latest
.\scripts\sherpa_client_install.ps1 -Update

# Update to specific version
.\scripts\sherpa_client_install.ps1 -Update -Version v0.3.35
```

**What it does:**
- Downloads `sherpa.exe` from GitHub Releases
- Installs to `$HOME\.sherpa\bin\`
- Adds install directory to user PATH (no admin required)

**Requirements:**
- PowerShell 5.1+ (built into Windows 10+)
- Internet access to GitHub

### `sherpa_uninstall.sh`
Removes the sherpad service and binaries, the `sherpa-db` container of a release before 0.5.0
if one is left, and optionally the data.

**Usage:**
```bash
# Remove the service, keep the data
sudo ./scripts/sherpa_uninstall.sh

# Remove the service and the database files
sudo ./scripts/sherpa_uninstall.sh --remove-data

# Remove everything without confirmation
sudo ./scripts/sherpa_uninstall.sh --remove-all --force

# View help
./scripts/sherpa_uninstall.sh --help
```

**Options:**
- `--keep-data` - Keep database files (default)
- `--remove-data` - Remove database files
- `--remove-all` - Remove entire `/opt/sherpa/` directory
- `--force` - Skip confirmation prompts

### Test tooling
The VM test harness (`test_env.py`, `test_install.sh` and the release-test runner) lives in
`tests/harness/`. See [running tests](../tests/README.md).

---

## Development

### Running Scripts from Repository Root
All scripts can be run from the repository root:

```bash
sudo -E ./scripts/sherpa_install.sh --version v0.3.79
```

### Blank disks
`sherpa_install.sh` and `sherpa_update.sh` carry the same blank disk section, between
`# BEGIN blank disks` and `# END blank disks`, because each is fetched alone. Change both;
`tests/bats/install_tests.bats` fails when they differ.

### Testing Changes
After modifying installation scripts, run the automated test suite:

```bash
./tests/harness/test_install.sh
```

---

## Troubleshooting

### Docker Not Running
```bash
# Check Docker status
sudo systemctl status docker

# Start Docker
sudo systemctl start docker
```

### Permission Denied
```bash
# Run with sudo
sudo -E ./scripts/sherpa_install.sh

# Make scripts executable
chmod +x scripts/*.sh
```

### Server Not Starting
```bash
# Check the service status and logs
sudo systemctl status sherpad
journalctl -u sherpad --no-pager -n 50
```
