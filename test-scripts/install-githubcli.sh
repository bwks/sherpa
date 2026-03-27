#!/bin/bash
set -euo pipefail

# Install GitHub CLI (gh) via official APT repository
# Requires: Debian/Ubuntu-based system

echo "Installing GitHub CLI..."

# Install wget if not present
if ! command -v wget &> /dev/null; then
    echo "wget not found, installing..."
    sudo apt-get update
    sudo apt-get install -y wget
fi

# Set up the GitHub CLI APT repository keyring
echo "Adding GitHub CLI GPG key..."
sudo mkdir -p -m 755 /etc/apt/keyrings
tmpkey=$(mktemp)
wget -nv -O "$tmpkey" https://cli.github.com/packages/githubcli-archive-keyring.gpg
sudo tee /etc/apt/keyrings/githubcli-archive-keyring.gpg < "$tmpkey" > /dev/null
sudo chmod go+r /etc/apt/keyrings/githubcli-archive-keyring.gpg
rm -f "$tmpkey"

# Add the GitHub CLI APT repository
echo "Adding GitHub CLI APT repository..."
sudo mkdir -p -m 755 /etc/apt/sources.list.d
echo "deb [arch=$(dpkg --print-architecture) signed-by=/etc/apt/keyrings/githubcli-archive-keyring.gpg] https://cli.github.com/packages stable main" \
    | sudo tee /etc/apt/sources.list.d/github-cli.list > /dev/null

# Install GitHub CLI
echo "Updating package list and installing gh..."
sudo apt-get update
sudo apt-get install -y gh

echo "GitHub CLI installed successfully"
gh --version
