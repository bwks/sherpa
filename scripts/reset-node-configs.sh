#!/usr/bin/env bash
#
# Reset node_config records in SurrealDB
# This script deletes all node_config records and lets Sherpa re-seed them
#
# Usage: ./scripts/reset-node-configs.sh

set -euo pipefail

# SurrealDB connection details
SURREAL_HOST="${SURREAL_HOST:-localhost:8000}"
SURREAL_USER="${SURREAL_USER:-sherpa}"
SURREAL_PASS="${SURREAL_PASS:-Everest1953!}"
SURREAL_NS="${SURREAL_NS:-sherpa}"
SURREAL_DB="${SURREAL_DB:-sherpa}"

echo "Connecting to SurrealDB at ${SURREAL_HOST}..."
echo "Namespace: ${SURREAL_NS}, Database: ${SURREAL_DB}"

# Delete all node_config records
echo "Deleting all node_config records..."
curl -X POST "http://${SURREAL_HOST}/sql" \
  -H "Content-Type: text/plain" \
  -H "Accept: application/json" \
  -H "Surreal-NS: ${SURREAL_NS}" \
  -H "Surreal-DB: ${SURREAL_DB}" \
  -u "${SURREAL_USER}:${SURREAL_PASS}" \
  --data "DELETE node_config;"

echo ""
echo "✓ Node configs deleted successfully"
echo ""
echo "Next steps:"
echo "1. Restart your Sherpa server (sherpad)"
echo "2. The node configs will be automatically re-seeded with the correct format"
echo ""
