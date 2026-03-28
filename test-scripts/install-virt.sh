#!/bin/bash
set -euo pipefail

# Install QEMU/KVM and libvirt on Ubuntu.
# Usage: sudo ./install_virt.sh

if [ "$(id -u)" -ne 0 ]; then
  echo "Error: This script must be run as root (use sudo)."
  exit 1
fi

echo "Installing QEMU/KVM and libvirt..."

apt-get update -qq
apt-get install -y -qq \
  qemu-kvm \
  qemu-utils \
  libvirt-daemon-system \
  libvirt-clients \
  bridge-utils \
  virtinst \
  ovmf \
  > /dev/null

# Enable and start libvirtd
systemctl enable --now libvirtd

# Add the sherpa user to libvirt and kvm groups
usermod -aG libvirt sherpa
usermod -aG kvm sherpa
echo "Added sherpa to libvirt and kvm groups."

# Verify
echo ""
echo "QEMU version:   $(qemu-system-x86_64 --version | head -1)"
echo "libvirtd:       $(virsh version --daemon 2>/dev/null | grep 'Running hypervisor' || echo 'running')"
echo "KVM available:  $([ -e /dev/kvm ] && echo 'yes' || echo 'no (/dev/kvm not found)')"
echo ""
echo "Installation complete."
