# Install Script — How to Run Tests

## Testing Approach

The install script (`scripts/sherpa_install.sh`) is a bash script, not a Rust crate. Testing requires either:

1. **BATS (Bash Automated Testing System)** — preferred for unit-style function testing
2. **Full end-to-end in a VM** — required for integration tests (needs a clean Ubuntu 24.04 install)

---

## VM-based End-to-End Testing (Recommended)

This is the canonical way to run both unit and integration tests. It spins up a
fresh Ubuntu 24.04 VM using libvirt/QEMU/KVM, runs the installer, then runs the
full BATS suite against the installed system.

### Prerequisites

The following must be installed on the host:

```bash
sudo apt-get install -y \
  qemu-kvm libvirt-daemon-system libvirt-clients \
  genisoimage curl bats
```

Your user must be in the `libvirt` and `kvm` groups:

```bash
sudo usermod -aG libvirt,kvm $USER
# Log out and back in, or: newgrp libvirt
```

Allow writes to the libvirt images directory:

```bash
sudo chmod 775 /var/lib/libvirt/images/
```

---

### Step 1 — Download Ubuntu 24.04 Cloud Image

```bash
curl -L -o /tmp/noble-server-cloudimg-amd64.img \
  https://cloud-images.ubuntu.com/noble/current/noble-server-cloudimg-amd64.img

sudo cp /tmp/noble-server-cloudimg-amd64.img \
  /var/lib/libvirt/images/noble-server-cloudimg-amd64.img
```

The image is ~600 MB. It only needs to be downloaded once and can be reused
across test runs as a read-only backing file.

---

### Step 2 — Generate an SSH Key

```bash
ssh-keygen -t ed25519 -f /tmp/sherpa-test-key -N "" -C "sherpa-install-test"
```

---

### Step 3 — Create the VM Disk

Create a 20 GB copy-on-write overlay backed by the cloud image:

```bash
sudo qemu-img create -f qcow2 \
  -b /var/lib/libvirt/images/noble-server-cloudimg-amd64.img \
  -F qcow2 \
  /var/lib/libvirt/images/sherpa-install-test.qcow2 20G
```

---

### Step 4 — Build a cloud-init Seed ISO

cloud-init configures the VM on first boot (creates user, injects SSH key).

```bash
mkdir -p /tmp/cloud-init

PUB_KEY=$(cat /tmp/sherpa-test-key.pub)

cat > /tmp/cloud-init/user-data << EOF
#cloud-config
users:
  - name: ubuntu
    sudo: ALL=(ALL) NOPASSWD:ALL
    shell: /bin/bash
    ssh_authorized_keys:
      - ${PUB_KEY}
chpasswd:
  expire: false
EOF

cat > /tmp/cloud-init/meta-data << 'EOF'
instance-id: sherpa-install-test
local-hostname: sherpa-install-test
EOF

genisoimage -output /tmp/cloud-init/cloud-init.iso \
  -volid cidata -joliet -rock \
  /tmp/cloud-init/user-data /tmp/cloud-init/meta-data

sudo cp /tmp/cloud-init/cloud-init.iso /var/lib/libvirt/images/cloud-init.iso
```

---

### Step 5 — Start the VM

```bash
virt-install \
  --name sherpa-install-test \
  --memory 8192 \
  --vcpus 2 \
  --disk path=/var/lib/libvirt/images/sherpa-install-test.qcow2,format=qcow2 \
  --disk path=/var/lib/libvirt/images/cloud-init.iso,device=cdrom \
  --os-variant ubuntu24.04 \
  --network network=default \
  --import \
  --graphics none \
  --noautoconsole
```

Wait for the VM to get a DHCP lease (~20–30 s):

```bash
VM_IP=""
for i in $(seq 1 30); do
  VM_IP=$(virsh domifaddr sherpa-install-test 2>/dev/null | grep -oP '(\d+\.){3}\d+' | head -1)
  [ -n "$VM_IP" ] && break
  sleep 5
done
echo "VM IP: $VM_IP"
```

Wait for SSH to become available:

```bash
SSH_OPTS="-o StrictHostKeyChecking=no -o ConnectTimeout=5 -i /tmp/sherpa-test-key"
for i in $(seq 1 24); do
  ssh $SSH_OPTS ubuntu@$VM_IP "echo ready" 2>/dev/null && break
  sleep 5
done
```

---

### Step 6 — Copy Test Files to the VM

The BATS file expects `scripts/sherpa_install.sh` relative to its own location,
so mirror the repo layout inside `~/`:

```bash
SSH_OPTS="-o StrictHostKeyChecking=no -i /tmp/sherpa-test-key"

scp $SSH_OPTS \
  scripts/sherpa_install.sh \
  test-scripts/install_tests.bats \
  ubuntu@$VM_IP:/home/ubuntu/

ssh $SSH_OPTS ubuntu@$VM_IP "
  mkdir -p ~/scripts ~/test-scripts
  cp ~/sherpa_install.sh  ~/scripts/sherpa_install.sh
  cp ~/install_tests.bats ~/test-scripts/install_tests.bats
"
```

---

### Step 7 — Install BATS on the VM

```bash
ssh $SSH_OPTS ubuntu@$VM_IP "sudo apt-get update -qq && sudo apt-get install -y bats"
```

---

### Step 8 — Run Unit Tests (no root required)

```bash
ssh $SSH_OPTS ubuntu@$VM_IP "bats ~/test-scripts/install_tests.bats"
```

The integration tests auto-skip until the installer has run (they check for
`/opt/sherpa`). Expected output: 31 ok, 24 skip.

---

### Step 9 — Run the Install Script

```bash
ssh $SSH_OPTS ubuntu@$VM_IP "
  export SHERPA_DB_PASSWORD='TestPassword123!'
  export SHERPA_SERVER_IPV4='0.0.0.0'
  sudo -E bash ~/scripts/sherpa_install.sh
"
```

This takes 3–5 minutes (apt packages + Docker install + image pulls).

---

### Step 10 — Run the Full Test Suite (unit + integration)

```bash
ssh $SSH_OPTS ubuntu@$VM_IP "
  export SHERPA_DB_PASSWORD='TestPassword123!'
  export SHERPA_SERVER_IPV4='0.0.0.0'
  sudo -E bats ~/test-scripts/install_tests.bats
"
```

Expected: **55/55 ok** (test 12 skips because it runs as root).

---

### Step 11 — Destroy the VM

```bash
virsh destroy sherpa-install-test
virsh undefine sherpa-install-test --remove-all-storage
sudo rm -f /var/lib/libvirt/images/cloud-init.iso
rm -rf /tmp/cloud-init /tmp/sherpa-test-key /tmp/sherpa-test-key.pub
```

The base cloud image (`noble-server-cloudimg-amd64.img`) is kept so future
runs skip the download.

---

## Unit-only Tests (host, no VM)

Individual functions can be tested directly on the host without root or a VM.
The integration tests auto-skip when `/opt/sherpa` is absent.

### Install BATS

```bash
sudo apt-get install bats
```

### Run

```bash
# From the repo root
bats test-scripts/install_tests.bats
```

---

## Manual Post-install Checklist

Run these inside the VM after the installer completes to spot-check the result:

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

---

## Idempotency Check

Run the installer a second time and verify it completes without errors:

```bash
ssh $SSH_OPTS ubuntu@$VM_IP "
  # Known issue (#6 in spec): stop the container first so the port check passes
  sudo docker stop sherpa-db && sudo docker rm sherpa-db
  export SHERPA_DB_PASSWORD='TestPassword123!'
  export SHERPA_SERVER_IPV4='0.0.0.0'
  sudo -E bash ~/scripts/sherpa_install.sh
"
```

The BATS integration test `idempotent re-run completes without error` automates
this when `SHERPA_DB_PASSWORD` and `SHERPA_SERVER_IPV4` are exported before
running `sudo bats`.

---

## Cleanup (full reset for re-testing)

To fully remove an install inside the VM:

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

---

## File Locations

| File | Purpose |
|------|---------|
| `scripts/sherpa_install.sh` | Script under test |
| `test-scripts/install_tests.bats` | BATS test suite |
| `test-specs/install/sherpa-install.md` | Test specification |
| `test-specs/install/HOW-TO-RUN.md` | This file |
