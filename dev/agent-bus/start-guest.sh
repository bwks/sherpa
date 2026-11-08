#!/usr/bin/env bash
# Start this guest's agent-bus agent on the Claude subscription. Run it in the guest with the
# token from `claude setup-token` on stdin; the token is kept only in the agent's environment.
# The bus runs on the host at the guest's default gateway, the lab's management bridge.
#
#   start-guest.sh [name]      # name defaults to dev
set -euo pipefail

NAME="${1:-dev}"
IFS= read -r CLAUDE_CODE_OAUTH_TOKEN
if [ -z "${CLAUDE_CODE_OAUTH_TOKEN}" ]; then
    echo "Give the Claude subscription token on stdin" >&2
    exit 2
fi
export CLAUDE_CODE_OAUTH_TOKEN
# The subscription, never API billing.
unset ANTHROPIC_API_KEY

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
GATEWAY="$(ip -4 route show default | awk '{print $3; exit}')"
LOG="${HOME}/agent-bus-${NAME}.log"

cd "${REPO}"
cargo build --release --quiet -p agent-bus
pkill -f "[a]gent-bus guest .*--name ${NAME} " || true
nohup target/release/agent-bus guest --server "nats://${GATEWAY}:4222" --name "${NAME}" \
    --workdir "${REPO}" > "${LOG}" 2>&1 < /dev/null &
sleep 2
if pgrep -f "[a]gent-bus guest .*--name ${NAME} " > /dev/null; then
    echo "Guest agent '${NAME}' is running; log: ${LOG}"
else
    cat "${LOG}"
    exit 1
fi
