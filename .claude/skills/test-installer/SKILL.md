---
name: test-installer
description: Run the full sherpa install script test suite in a fresh Ubuntu 24.04 KVM VM
---

# Test Installer

Run the full sherpa install script test suite in a fresh Ubuntu 24.04 KVM VM.

## What this does

1. Creates an SSH keypair for VM access
2. Creates a 20 GB qcow2 disk backed by the cached Ubuntu 24.04 cloud image (downloads it if not present)
3. Builds a cloud-init seed ISO to provision the `ubuntu` user
4. Boots the VM (8 GB RAM, 2 vCPUs) via libvirt
5. Copies `scripts/sherpa_install.sh` and `test-scripts/install_tests.bats` to the VM
6. Installs BATS on the VM
7. Runs the unit tests (no root)
8. Runs the full install script non-interactively
9. Runs the full BATS suite as root (unit + integration)
10. Reports results
11. Destroys the VM and all temporary artifacts

## Instructions

Execute the following steps exactly. Do not skip any step. Report progress to the user as you go.

### Pre-flight

- Check that `virsh`, `virt-install`, `qemu-img`, and `genisoimage` are available on the host.
- Ensure the libvirt default network is active: `virsh net-list --all`
- Ensure `/var/lib/libvirt/images/` is writable (chmod 775 if needed)

### SSH Key

Generate a throwaway keypair:
```
ssh-keygen -t ed25519 -f /tmp/sherpa-test-key -N "" -C "sherpa-install-test"
```

### Cloud Image

Check if the base image is already cached:
```
ls /var/lib/libvirt/images/noble-server-cloudimg-amd64.img
```

If absent, download it to `/tmp` first (it is ~600 MB), then copy with sudo:
```
curl -L -o /tmp/noble-server-cloudimg-amd64.img \
  https://cloud-images.ubuntu.com/noble/current/noble-server-cloudimg-amd64.img
sudo cp /tmp/noble-server-cloudimg-amd64.img \
  /var/lib/libvirt/images/noble-server-cloudimg-amd64.img
rm /tmp/noble-server-cloudimg-amd64.img
```

### VM Disk

```
sudo qemu-img create -f qcow2 \
  -b /var/lib/libvirt/images/noble-server-cloudimg-amd64.img \
  -F qcow2 \
  /var/lib/libvirt/images/sherpa-install-test.qcow2 20G
```

### Cloud-init Seed ISO

```
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

### Boot VM

```
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

Poll for IP (up to 150 s, check every 5 s):
```
for i in $(seq 1 30); do
  VM_IP=$(virsh domifaddr sherpa-install-test 2>/dev/null | grep -oP '(\d+\.){3}\d+' | head -1)
  [ -n "$VM_IP" ] && break
  sleep 5
done
```

Poll for SSH (up to 120 s, check every 5 s):
```
SSH_OPTS="-o StrictHostKeyChecking=no -o ConnectTimeout=5 -i /tmp/sherpa-test-key"
for i in $(seq 1 24); do
  ssh $SSH_OPTS ubuntu@$VM_IP "echo ready" 2>/dev/null && break
  sleep 5
done
```

### Copy Files & Install BATS

```
scp $SSH_OPTS \
  scripts/sherpa_install.sh \
  test-scripts/install_tests.bats \
  ubuntu@$VM_IP:/home/ubuntu/

ssh $SSH_OPTS ubuntu@$VM_IP "
  mkdir -p ~/scripts ~/test-scripts
  cp ~/sherpa_install.sh  ~/scripts/sherpa_install.sh
  cp ~/install_tests.bats ~/test-scripts/install_tests.bats
"

ssh $SSH_OPTS ubuntu@$VM_IP \
  "sudo apt-get update -qq && sudo apt-get install -y bats"
```

### Run Unit Tests (pre-install)

```
ssh $SSH_OPTS ubuntu@$VM_IP "bats ~/test-scripts/install_tests.bats"
```

Expected: 31 ok, 24 skip. Report the result to the user.

### Run Install Script

```
ssh $SSH_OPTS ubuntu@$VM_IP "
  export SHERPA_DB_PASSWORD='TestPassword123!'
  export SHERPA_SERVER_IPV4='0.0.0.0'
  sudo -E bash ~/scripts/sherpa_install.sh
"
```

This takes 3–5 minutes. Show the user a progress note while waiting.

### Run Full Test Suite (post-install)

```
ssh $SSH_OPTS ubuntu@$VM_IP "
  export SHERPA_DB_PASSWORD='TestPassword123!'
  export SHERPA_SERVER_IPV4='0.0.0.0'
  sudo -E bats ~/test-scripts/install_tests.bats
"
```

Expected: 55 ok (test 12 skips when running as root — that is correct behaviour).

### Destroy VM & Clean Up

Always destroy the VM, even if tests fail:
```
virsh destroy sherpa-install-test 2>/dev/null || true
virsh undefine sherpa-install-test --remove-all-storage 2>/dev/null || true
sudo rm -f /var/lib/libvirt/images/cloud-init.iso
rm -rf /tmp/cloud-init /tmp/sherpa-test-key /tmp/sherpa-test-key.pub
```

The base cloud image (`noble-server-cloudimg-amd64.img`) is intentionally kept
so the next run skips the download.

### Report

Show the user a clear pass/fail summary with the BATS output for both the
pre-install and post-install runs.
