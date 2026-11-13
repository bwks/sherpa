# Sherpa Scripts

This directory contains utility scripts for Sherpa installation, maintenance, and development.

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
- Creates sherpa user and groups
- Sets up directory structure at `/opt/sherpa/`
- Pulls the Sherpa router container image
- Installs the binaries, the sherpad systemd service and its environment file
- Refuses a server older than 0.5.0 (a `sherpa-db` SurrealDB container exists)

**Requirements:**
- Ubuntu 24.04 or later, curl, virtualization support and internet access
- Root/sudo privileges

### `sherpa_update.sh`
Replaces the Sherpa binaries with the latest, or a given, release and restarts sherpad. It
refuses, before changing anything, a server older than 0.5.0 whose data is still in SurrealDB:
such a server updates to 0.5.12 first with that release's update script, which moves the data.

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

## Utility Scripts

### `create_blank_disks.sh`
Creates blank disk images for various network operating systems.

### `create_iosv_disk.sh`
Creates blank disk images specifically for Cisco IOSv devices.

### `fix-permissions.sh`
Fixes file permissions for Sherpa directories and files.

---

## Development

### Running Scripts from Repository Root
All scripts can be run from the repository root:

```bash
sudo -E ./scripts/sherpa_install.sh --version v0.3.79
```

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
