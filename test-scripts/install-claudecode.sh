#!/bin/bash
set -euo pipefail

# Install Claude Code via the native installer.
# https://code.claude.com/docs/en/getting-started

echo "Installing Claude Code..."

curl -fsSL https://claude.ai/install.sh | sh

echo "Claude Code installed successfully"
claude --version
