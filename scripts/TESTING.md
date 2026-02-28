# Install Script Testing Guide

## Prerequisites
- Docker installed and running ✓
- Port 8000 in use by existing `surrealdb` container
- User: bradmin with sudo access ✓
- Already member of sherpa, docker, libvirt, kvm groups ✓

## Test Scenarios

### 1. Help and Usage Tests
```bash
# Test help message
./scripts/sherpa_install.sh --help
./scripts/sherpa_uninstall.sh --help
```
**Status:** ✓ PASSED - Help messages display correctly

### 2. Error Handling Tests

#### Test 2.1: No Root Privileges
```bash
# Should fail with clear error message
./scripts/sherpa_install.sh --db-pass "TestPassword123"
```
**Expected:** Error message about needing root/sudo
**Status:** ✓ PASSED

#### Test 2.2: No Password Provided
```bash
# Should fail with helpful message
sudo ./scripts/sherpa_install.sh
```
**Expected:** Error about missing password with usage instructions

#### Test 2.3: Password Too Short
```bash
# Should fail with minimum length error
sudo ./scripts/sherpa_install.sh --db-pass "short"
```
**Expected:** Error about password being too short (< 8 chars)

#### Test 2.4: Port Already in Use
```bash
# Current surrealdb container is using port 8000
sudo ./scripts/sherpa_install.sh --db-pass "TestPassword123"
```
**Expected:** Should detect port in use, show details, and fail gracefully

### 3. Successful Installation Test

#### Test 3.1: Stop Existing Container First
```bash
# Stop and remove the old container
docker stop surrealdb
docker rm surrealdb

# OR use --remove-data to clean slate
sudo ./scripts/sherpa_uninstall.sh --remove-data --force
```

#### Test 3.2: Fresh Install
```bash
# Run install with proper password
sudo ./scripts/sherpa_install.sh --db-pass "Everest1953!"
```
**Expected:**
- Creates/verifies sherpa user and groups
- Creates directories with correct permissions
- Pulls surrealdb:v2.4 image
- Starts sherpa-db container
- Waits for health check
- Shows success message

#### Test 3.3: Verify Installation
```bash
# Check container is running
docker ps | grep sherpa-db

# Check health endpoint
curl http://localhost:8000/health

# Check logs
docker logs sherpa-db

# Check directory permissions
ls -la /opt/sherpa/
ls -la /opt/sherpa/db/

# Verify restart policy
docker inspect sherpa-db --format='{{.HostConfig.RestartPolicy.Name}}'
```

#### Test 3.4: Verify Database Files
```bash
# Check if database file was created
ls -la /opt/sherpa/db/
```
**Expected:** Should see sherpa.db file owned by sherpa:sherpa

### 4. Re-run Tests (Idempotency)

#### Test 4.1: Run Install Again
```bash
# Should handle existing container gracefully
sudo ./scripts/sherpa_install.sh --db-pass "Everest1953!"
```
**Expected:**
- Detects existing container
- Stops and removes it
- Creates new container with same name
- Preserves existing data in /opt/sherpa/db/

#### Test 4.2: Verify Data Persistence
```bash
# Data from previous install should still exist
ls -la /opt/sherpa/db/
```

### 5. Uninstall Tests

#### Test 5.1: Uninstall Keep Data (Default)
```bash
# Should remove container but keep data
sudo ./scripts/sherpa_uninstall.sh
```
**Expected:**
- Prompts for confirmation
- Stops and removes sherpa-db container
- Keeps /opt/sherpa/db/ intact

#### Test 5.2: Verify Container Removed
```bash
docker ps -a | grep sherpa-db || echo "Container removed"
ls -la /opt/sherpa/db/  # Should still exist
```

#### Test 5.3: Reinstall After Uninstall
```bash
# Should work with existing data
sudo ./scripts/sherpa_install.sh --db-pass "Everest1953!"
```

#### Test 5.4: Uninstall with Data Removal
```bash
sudo ./scripts/sherpa_uninstall.sh --remove-data --force
```
**Expected:**
- No prompt (--force flag)
- Removes container
- Removes database files in /opt/sherpa/db/
- Keeps /opt/sherpa/ directory structure

#### Test 5.5: Uninstall Everything
```bash
sudo ./scripts/sherpa_uninstall.sh --remove-all --force
```
**Expected:**
- Removes container
- Removes entire /opt/sherpa/ directory

### 6. Multi-user Access Test

#### Test 6.1: Access as bradmin
```bash
# As bradmin (after re-login if needed)
ls -la /opt/sherpa/db/
touch /opt/sherpa/db/test_bradmin.txt
```
**Expected:** Should have write access due to sherpa group membership

#### Test 6.2: Access as sherpa user
```bash
# Switch to sherpa user
sudo -u sherpa ls -la /opt/sherpa/db/
sudo -u sherpa touch /opt/sherpa/db/test_sherpa.txt
```
**Expected:** Should have full access as owner

### 7. Restart/Reboot Test

#### Test 7.1: System Restart Simulation
```bash
# Restart Docker daemon
sudo systemctl restart docker

# Wait a moment
sleep 5

# Check if container auto-restarted
docker ps | grep sherpa-db
```
**Expected:** Container should automatically restart due to `--restart unless-stopped`

### 8. Environment Variable Test

#### Test 8.1: Using Environment Variable
```bash
export SHERPA_DB_PASSWORD="Everest1953!"
sudo -E ./scripts/sherpa_install.sh
```
**Expected:** Should accept password from environment variable

## Test Results Summary

| Test | Status | Notes |
|------|--------|-------|
| Help messages | ✓ PASSED | Clear and informative |
| No root privileges | ✓ PASSED | Proper error handling |
| No password | PENDING | Need sudo access to test |
| Password too short | PENDING | Need sudo access to test |
| Port in use detection | PENDING | Current container blocks port |
| Fresh install | PENDING | Need to stop existing container |
| Re-run install | PENDING | Test idempotency |
| Uninstall (keep data) | PENDING | |
| Uninstall (remove data) | PENDING | |
| Multi-user access | PENDING | |
| Auto-restart | PENDING | |
| Env var password | PENDING | |

## Manual Testing Commands

To run through the full test suite manually, you can use this sequence:

```bash
# 1. Clean slate
docker stop surrealdb 2>/dev/null || true
docker rm surrealdb 2>/dev/null || true

# 2. Test error cases (no password)
sudo ./scripts/sherpa_install.sh

# 3. Test password too short
sudo ./scripts/sherpa_install.sh --db-pass "abc"

# 4. Successful install
sudo ./scripts/sherpa_install.sh --db-pass "Everest1953!"

# 5. Verify
docker ps | grep sherpa-db
curl http://localhost:8000/health
docker logs sherpa-db
ls -la /opt/sherpa/db/

# 6. Re-run (test idempotency)
sudo ./scripts/sherpa_install.sh --db-pass "Everest1953!"

# 7. Uninstall (keep data)
sudo ./scripts/sherpa_uninstall.sh

# 8. Verify
docker ps -a | grep sherpa-db || echo "Container removed"
ls -la /opt/sherpa/db/  # Should still exist

# 9. Reinstall
sudo ./scripts/sherpa_install.sh --db-pass "Everest1953!"

# 10. Full uninstall
sudo ./scripts/sherpa_uninstall.sh --remove-all --force
```

## Automated Testing Script

For convenience, here's an automated test script:

```bash
#!/bin/bash
# test_install.sh - Automated testing of install/uninstall scripts

set -e

echo "=== Starting Automated Tests ==="

# Test 1: Help
echo "Test 1: Help messages"
./scripts/sherpa_install.sh --help > /dev/null
./scripts/sherpa_uninstall.sh --help > /dev/null
echo "✓ Help tests passed"

# Test 2: No root (should fail)
echo "Test 2: No root privileges"
if ./scripts/sherpa_install.sh --db-pass "test" 2>&1 | grep -q "must be run as root"; then
    echo "✓ Root check passed"
else
    echo "✗ Root check failed"
    exit 1
fi

# Test 3: Clean environment
echo "Test 3: Cleaning environment"
docker stop surrealdb sherpa-db 2>/dev/null || true
docker rm surrealdb sherpa-db 2>/dev/null || true
echo "✓ Environment cleaned"

# Test 4: Fresh install
echo "Test 4: Fresh installation"
if sudo ./scripts/sherpa_install.sh --db-pass "Everest1953!" | grep -q "Installation Complete"; then
    echo "✓ Installation successful"
else
    echo "✗ Installation failed"
    exit 1
fi

# Test 5: Container health
echo "Test 5: Container health check"
sleep 2
if docker ps | grep -q "sherpa-db"; then
    echo "✓ Container is running"
else
    echo "✗ Container not found"
    exit 1
fi

# Test 6: Database health
echo "Test 6: Database health endpoint"
if curl -sf http://localhost:8000/health > /dev/null; then
    echo "✓ Database is healthy"
else
    echo "✗ Database health check failed"
    exit 1
fi

# Test 7: Uninstall
echo "Test 7: Uninstall (keep data)"
if sudo ./scripts/sherpa_uninstall.sh --force | grep -q "Uninstall Complete"; then
    echo "✓ Uninstall successful"
else
    echo "✗ Uninstall failed"
    exit 1
fi

# Test 8: Container removed
echo "Test 8: Verify container removed"
if ! docker ps -a | grep -q "sherpa-db"; then
    echo "✓ Container removed"
else
    echo "✗ Container still exists"
    exit 1
fi

echo ""
echo "=== All Tests Passed! ==="
```
