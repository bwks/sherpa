#!/bin/bash
set -euo pipefail

# Install HashiCorp Packer on Debian/Ubuntu systems.
# Usage: sudo ./install_packer.sh

if [ "$(id -u)" -ne 0 ]; then
  echo "Error: This script must be run as root (use sudo)."
  exit 1
fi

echo "Installing HashiCorp Packer..."

# Install dependencies
apt-get update -qq
apt-get install -y -qq gnupg software-properties-common curl > /dev/null

# Add HashiCorp GPG key
curl -fsSL https://apt.releases.hashicorp.com/gpg | gpg --dearmor -o /usr/share/keyrings/hashicorp-archive-keyring.gpg

# Add HashiCorp repository
echo "deb [signed-by=/usr/share/keyrings/hashicorp-archive-keyring.gpg] https://apt.releases.hashicorp.com $(lsb_release -cs) main" \
  > /etc/apt/sources.list.d/hashicorp.list

# Install packer
apt-get update -qq
apt-get install -y -qq packer > /dev/null

# Verify
PACKER_VERSION=$(packer version)
echo "Packer installed successfully: ${PACKER_VERSION}"
