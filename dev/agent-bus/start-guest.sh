#!/usr/bin/env bash
# Start this guest's agent-bus agent on the Claude subscription. Run it in the guest with two
# lines on stdin: the token from `claude setup-token`, then the agent's NKey seed (from
# `agent-bus key new`). Neither is a command-line argument or written to disk. The seed is
# exported to the agent process only, and the agent keeps it from Claude Code.
# The bus runs on the host at the guest's default gateway, the lab's management bridge.
#
#   start-guest.sh [name]      # name defaults to dev
set -euo pipefail

NAME="${1:-dev}"
IFS= read -r CLAUDE_CODE_OAUTH_TOKEN || true
IFS= read -r SEED || true
if [ -z "${CLAUDE_CODE_OAUTH_TOKEN}" ] || [ -z "${SEED}" ]; then
    echo "Give the Claude subscription token and then the agent's bus seed on stdin, one per line" >&2
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
AGENT_BUS_SEED="${SEED}" nohup target/release/agent-bus guest --server "nats://${GATEWAY}:4222" --name "${NAME}" \
    --workdir "${REPO}" > "${LOG}" 2>&1 < /dev/null &
unset SEED
sleep 2
if pgrep -f "[a]gent-bus guest .*--name ${NAME} " > /dev/null; then
    echo "Guest agent '${NAME}' is running; log: ${LOG}"
else
    cat "${LOG}"
    exit 1
fi
