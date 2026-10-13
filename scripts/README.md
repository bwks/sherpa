# Sherpa Scripts

This directory contains utility scripts for Sherpa installation, maintenance, and development.

## Installation Scripts

### `sherpa_install.sh`
Installs the Sherpa server binaries, Docker, QEMU/libvirt and a SurrealDB container.

**Usage:**
```bash
# Using environment variable
export SHERPA_DB_PASSWORD="YourPassword"
sudo -E ./scripts/sherpa_install.sh

# View help
./scripts/sherpa_install.sh --help
```

**What it does:**
- Creates sherpa user and groups
- Sets up directory structure at `/opt/sherpa/`
- Pulls SurrealDB v3.0.0 and the Sherpa router container images
- Starts container with persistent storage at `/opt/sherpa/db/`
- Configures restart policy for auto-start on boot
- Verifies database health

**Requirements:**
- Ubuntu 24.04 or later, curl, virtualization support and internet access
- Root/sudo privileges
- Port 8000 available
- Password at least 8 characters

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
Removes the SurrealDB container and optionally cleans up data.

**Usage:**
```bash
# Remove container only (keep data)
sudo ./scripts/sherpa_uninstall.sh

# Remove container and database files
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

### `test_install.sh`
Tests the real installer and uninstaller in a fresh Sherpa-managed Ubuntu VM.
Run as the user with the authenticated CLI; guest operations use sudo internally.

**Usage:**
```bash
./scripts/test_install.sh
```

This wraps `vm_release_test.py`, which installs, exercises and uninstalls a candidate
build inside a disposable Sherpa-managed VM. Run the harness self-tests with:

```bash
python3 scripts/vm_release_test.py --self-test
```

**See also:** [VM release-test guide](../dev/release-test/README.md) and
[TOML settings](../dev/release-test/config.toml).

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
./scripts/test_install.sh
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

### Port 8000 Already in Use
```bash
# Find what's using port 8000
sudo ss -tulnp | grep :8000

# OR
sudo netstat -tulnp | grep :8000

# Stop existing SurrealDB container
docker stop sherpa-db
docker rm sherpa-db
```

### Permission Denied
```bash
# Run with sudo
sudo -E ./scripts/sherpa_install.sh

# Make scripts executable
chmod +x scripts/*.sh
```

### Database Not Starting
```bash
# Check container logs
docker logs sherpa-db

# Check container status
docker ps -a | grep sherpa-db

# Restart container
docker restart sherpa-db
```
