# Install Script — How to Run Tests

## Testing Approach

The install script (`scripts/sherpa_install.sh`) is a bash script, not a Rust crate. Testing requires either:

1. **BATS (Bash Automated Testing System)** — preferred for unit-style function testing
2. **Manual verification** — for full end-to-end install on a clean VM

## Unit-style Tests (BATS)

Individual functions can be sourced and tested in isolation. Many pre-flight checks can be tested by mocking `/etc/os-release`, `$EUID`, and command availability.

### Install BATS

```bash
sudo apt-get install bats
```

### Run Tests

```bash
# Unit tests only (no root required, works on any Linux host)
bats test-scripts/install_tests.bats

# Integration tests (run as root on a host where the install has completed)
sudo bats test-scripts/install_tests.bats
```

The integration tests auto-skip when `/opt/sherpa` is absent, so the same
command works in both contexts.

## Manual End-to-End Verification

> **Requires Ubuntu 24.04+.** The install script explicitly rejects older releases.

Run a full install on a clean Ubuntu 24.04+ VM:

```bash
# Non-interactive install (latest release)
export SHERPA_DB_PASSWORD="TestPassword123!"
export SHERPA_SERVER_IPV4="0.0.0.0"
sudo -E ./scripts/sherpa_install.sh

# Or pin to a specific version
sudo -E ./scripts/sherpa_install.sh --version v0.3.33
```

### Post-install Checklist

```bash
# 1. User exists with correct shell
getent passwd sherpa

# 2. Groups assigned
id sherpa

# 3. Directories exist with correct permissions
ls -la /opt/sherpa/
ls -la /opt/sherpa/env/

# 4. Container running
docker ps | grep sherpa-db

# 5. Database healthy
curl -sf http://localhost:8000/health

# 6. Binaries installed
ls -la /opt/sherpa/bin/
ls -la /usr/local/bin/sherpad

# 7. Systemd service enabled
systemctl is-enabled sherpad

# 8. Env file has correct values and restricted permissions
stat /opt/sherpa/env/sherpa.env
```

## Idempotency Check

Run the installer a second time and verify it completes without errors:

```bash
export SHERPA_DB_PASSWORD="TestPassword123!"
export SHERPA_SERVER_IPV4="0.0.0.0"
sudo -E ./scripts/sherpa_install.sh
```

The BATS integration test `idempotent re-run completes without error` automates
this when `SHERPA_DB_PASSWORD` and `SHERPA_SERVER_IPV4` are exported before
running `sudo bats`.

## Cleanup

To fully remove an install for re-testing:

```bash
# Stop and remove container
docker stop sherpa-db && docker rm sherpa-db

# Remove directories
sudo rm -rf /opt/sherpa

# Remove user
sudo userdel sherpa

# Remove systemd service
sudo rm /etc/systemd/system/sherpad.service
sudo rm /etc/logrotate.d/sherpad
sudo systemctl daemon-reload

# Remove symlinks
sudo rm -f /usr/local/bin/sherpad /usr/local/bin/sherpa
```

## Test Location

- Spec: `test-specs/install/sherpa-install.md`
- Test file: `test-scripts/install_tests.bats`
- Script under test: `scripts/sherpa_install.sh`
