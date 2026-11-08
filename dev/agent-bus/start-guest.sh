#!/usr/bin/env bash
# Start this guest's agent-bus agent on the Claude subscription. Run it in the guest with two
# lines on stdin: the token from `claude setup-token`, then the agent's NKey seed (from
# `agent-bus key new`). Neither is a command-line argument or written to disk. The seed is
# exported to the agent process only, and the agent keeps it from Claude Code.
# The bus runs on the host at the guest's default gateway, the lab's management bridge. A guest
# in another lab, such as tests/environments/agent, sets AGENT_BUS_URL to where the bus listens.
#
#   start-guest.sh [name]      # name defaults to dev
#   AGENT_BUS_MODEL=<id>       # optional: the Claude model, default the binary's own
#   AGENT_BUS_URL=<url>        # optional: the bus, default nats://<default gateway>:4222
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
URL="${AGENT_BUS_URL:-nats://${GATEWAY}:4222}"
MODEL_ARGS=()
if [ -n "${AGENT_BUS_MODEL:-}" ]; then
    MODEL_ARGS=(--model "${AGENT_BUS_MODEL}")
fi
LOG="${HOME}/agent-bus-${NAME}.log"

cd "${REPO}"
cargo build --release --quiet -p agent-bus
# Stop the old agent and wait until it has exited before starting the new one: the two must
# never share the state file. A stopping agent stops its Claude Code within its own kill grace,
# so it gets longer than that before SIGKILL. The new agent also refuses to start while another
# of its name holds the lock.
PATTERN="[a]gent-bus guest .*--name ${NAME} "
STOP_TIMEOUT=15

# Wait up to $1 seconds for every agent of this name to exit. True once none is left.
wait_gone() {
    local deadline=$((SECONDS + $1))
    while pgrep -f "${PATTERN}" > /dev/null; do
        if [ "${SECONDS}" -ge "${deadline}" ]; then
            return 1
        fi
        sleep 0.2
    done
}

if pgrep -f "${PATTERN}" > /dev/null; then
    pkill -f "${PATTERN}" || true
    if ! wait_gone "${STOP_TIMEOUT}"; then
        echo "Guest agent '${NAME}' did not stop within ${STOP_TIMEOUT}s: sending SIGKILL" >&2
        pkill -KILL -f "${PATTERN}" || true
        if ! wait_gone 5; then
            echo "Unable to stop guest agent '${NAME}' (PIDs: $(pgrep -f "${PATTERN}" | tr '\n' ' ')): not starting a second one" >&2
            exit 1
        fi
    fi
fi
AGENT_BUS_SEED="${SEED}" nohup target/release/agent-bus guest --server "${URL}" --name "${NAME}" \
    --workdir "${REPO}" "${MODEL_ARGS[@]}" > "${LOG}" 2>&1 < /dev/null &
unset SEED
sleep 2
if pgrep -f "[a]gent-bus guest .*--name ${NAME} " > /dev/null; then
    echo "Guest agent '${NAME}' is running; log: ${LOG}"
else
    cat "${LOG}"
    exit 1
fi
