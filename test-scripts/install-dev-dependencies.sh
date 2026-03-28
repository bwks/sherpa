#!/bin/bash
set -euo pipefail

# Install development dependencies needed to build and test the Sherpa project.
# The virt_server image already has Docker and libvirt installed.
# This script adds the C build toolchain and headers for Rust FFI crates.

if [ "$(id -u)" -ne 0 ]; then
  echo "Error: This script must be run as root."
  exit 1
fi

echo "Installing Sherpa build dependencies..."

apt-get update -qq
apt-get install -y -qq \
  build-essential \
  pkg-config \
  libssl-dev \
  libvirt-dev \
  genisoimage \
  mtools \
  e2fsprogs \
  > /dev/null

echo "All development dependencies installed."
