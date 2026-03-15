#!/usr/bin/env bash
set -euo pipefail

TAILWIND_VERSION="4.2.1"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"
TAILWIND_BIN="${PROJECT_DIR}/tailwindcss"
SERVER_DIR="${PROJECT_DIR}/crates/server"

# Download standalone CLI if not present
if [[ ! -x "${TAILWIND_BIN}" ]]; then
    echo "Downloading Tailwind CSS v${TAILWIND_VERSION} standalone CLI..."
    ARCH="$(uname -m)"
    OS="$(uname -s | tr '[:upper:]' '[:lower:]')"

    case "${ARCH}" in
        x86_64)  ARCH="x64" ;;
        aarch64) ARCH="arm64" ;;
        *)       echo "Unsupported architecture: ${ARCH}"; exit 1 ;;
    esac

    URL="https://github.com/tailwindlabs/tailwindcss/releases/download/v${TAILWIND_VERSION}/tailwindcss-${OS}-${ARCH}"
    curl -sL "${URL}" -o "${TAILWIND_BIN}"
    chmod +x "${TAILWIND_BIN}"
    echo "Downloaded to ${TAILWIND_BIN}"
fi

cd "${SERVER_DIR}"

echo "Building Tailwind CSS..."
"${TAILWIND_BIN}" \
    -i web/src/input.css \
    -o web/static/css/tailwind.css \
    --minify

echo "Tailwind CSS build complete: web/static/css/tailwind.css"
