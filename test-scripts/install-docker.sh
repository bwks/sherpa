#!/bin/bash
set -euo pipefail

# Install Docker Engine on Ubuntu via the official Docker apt repository.
# Executed by cloud-init as root.

if [ "$(id -u)" -ne 0 ]; then
  echo "Error: This script must be run as root."
  exit 1
fi

echo "Installing Docker Engine..."

# Install dependencies
apt-get update -qq
apt-get install -y -qq ca-certificates curl > /dev/null

# Add Docker GPG key
install -m 0755 -d /etc/apt/keyrings
curl -fsSL https://download.docker.com/linux/ubuntu/gpg -o /etc/apt/keyrings/docker.asc
chmod a+r /etc/apt/keyrings/docker.asc

# Add Docker repository
echo "deb [arch=$(dpkg --print-architecture) signed-by=/etc/apt/keyrings/docker.asc] https://download.docker.com/linux/ubuntu $(. /etc/os-release && echo "$VERSION_CODENAME") stable" \
  > /etc/apt/sources.list.d/docker.list

# Install Docker
apt-get update -qq
apt-get install -y -qq \
  docker-ce \
  docker-ce-cli \
  containerd.io \
  docker-buildx-plugin \
  docker-compose-plugin \
  > /dev/null

# Enable and start Docker
systemctl enable --now docker

# Add the sherpa user to the docker group
usermod -aG docker sherpa
echo "Added sherpa to docker group."

# Verify
echo ""
echo "Docker version: $(docker --version)"
echo ""
echo "Installation complete."
