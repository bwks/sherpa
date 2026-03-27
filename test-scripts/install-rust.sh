#!/bin/bash
set -euo pipefail

# Install Rust toolchain (stable) via rustup for the sherpa user.
# Build dependencies should be installed separately via install-dev-dependencies.sh.

echo "Installing Rust toolchain for sherpa user..."
su - sherpa -c 'curl --proto "=https" --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain stable'

# Verify
su - sherpa -c 'source ~/.cargo/env && rustc --version && cargo --version'

echo "Rust installation complete."
