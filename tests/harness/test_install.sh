#!/usr/bin/env bash
# Run installer/uninstaller tests only in a disposable Sherpa-managed VM.
set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
exec uv run --project "${SCRIPT_DIR}/../.." "${SCRIPT_DIR}/vm_release_test.py" "$@"
