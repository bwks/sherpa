# Sherpa Scripts

This directory contains utility scripts for Sherpa installation, maintenance, and development.

## Installation Scripts

### `sherpa_install.sh`
Installs and configures SurrealDB as a Docker container for the Sherpa application.

**Usage:**
```bash
# Using command line flag
sudo ./scripts/sherpa_install.sh --db-pass "YourPassword"

# Using environment variable
export SHERPA_DB_PASSWORD="YourPassword"
sudo -E ./scripts/sherpa_install.sh

# View help
./scripts/sherpa_install.sh --help
```

**What it does:**
- Creates sherpa user and groups
- Sets up directory structure at `/opt/sherpa/`
- Pulls SurrealDB v2.4 container image
- Starts container with persistent storage at `/opt/sherpa/db/`
- Configures restart policy for auto-start on boot
- Verifies database health

**Requirements:**
- Docker installed and running
- Root/sudo privileges
- Port 8000 available
- Password at least 8 characters

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
Automated test suite for installation and uninstallation scripts.

**Usage:**
```bash
sudo ./scripts/test_install.sh
```

**Tests:**
- Help message display
- Root privilege checks
- Password validation
- Fresh installation
- Container health
- Restart policy
- Idempotent re-installation
- Data persistence
- Uninstall with data preservation
- Complete removal

**See also:** `TESTING.md` for detailed test documentation

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
# From /home/bradmin/code/rust/sherpa/
sudo ./scripts/sherpa_install.sh --db-pass "YourPassword"
```

### Testing Changes
After modifying installation scripts, run the automated test suite:

```bash
sudo ./scripts/test_install.sh
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
docker stop surrealdb
docker rm surrealdb
```

### Permission Denied
```bash
# Run with sudo
sudo ./scripts/sherpa_install.sh --db-pass "YourPassword"

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

---

## Files

- `sherpa_install.sh` - Installation script
- `sherpa_uninstall.sh` - Uninstallation script  
- `test_install.sh` - Automated test suite
- `TESTING.md` - Test documentation
- `create_blank_disks.sh` - Disk creation utility
- `create_iosv_disk.sh` - IOSv disk creation utility
- `create_iosv_disk.py` - Python version of IOSv disk creator
- `fix-permissions.sh` - Permission fixing utility
