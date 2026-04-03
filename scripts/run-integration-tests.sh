#!/usr/bin/env bash
set -euo pipefail

export PATH="$HOME/.cargo/bin:$PATH"

echo "=== Prerequisites ==="
docker info > /dev/null 2>&1 || { echo "Docker not running"; exit 1; }

cd "$(dirname "$0")/.."

echo "=== Restarting test DB ==="
SHERPA_DEV_DB_USER=sherpa SHERPA_DEV_DB_PASS="Everest1953!" ./dev/testdb restart

echo ""
echo "=== Unit tests ==="
cargo test --workspace 2>&1

echo ""
echo "=== DB integration tests ==="
cargo test -p db -- --ignored 2>&1

echo ""
echo "=== Container integration tests ==="
cargo test -p container -- --ignored --test-threads=1 2>&1

echo ""
echo "=== Network integration tests ==="
sudo -E env PATH="$HOME/.cargo/bin:$PATH" cargo test -p network -- --ignored --test-threads=1 2>&1

if virsh -c qemu:///system version > /dev/null 2>&1; then
    echo ""
    echo "=== Libvirt integration tests ==="
    cargo test -p libvirt -- --ignored --test-threads=1 2>&1
else
    echo ""
    echo "=== Libvirt integration tests SKIPPED (libvirtd not available) ==="
fi

echo ""
echo "=== Server integration tests ==="
sudo -E env PATH="$HOME/.cargo/bin:$PATH" cargo test -p sherpad -- --ignored --test-threads=1 2>&1

echo ""
echo "=== All tests complete ==="
