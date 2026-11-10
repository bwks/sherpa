#!/usr/bin/env bash
# Start this guest's agent-bus agent on the Claude subscription. Run it in the guest with, on
# stdin: the token from `claude setup-token` on one line, the GitHub write token on the next (an
# empty line for none), the agent's NKey seed on the next, then the bus's CA certificate (PEM) to
# the end. On the host, dev/agent-bus/start-agent.sh runs it with all four, the seed straight from
# `agent-bus enroll`.
#
# No secret is exported, a command-line argument or written to disk. The three secrets are read
# into unexported variables and reach the agent on its stdin, through a pipe from the printf
# builtin, and the agent is started without AGENT_BUS_SEED or CLAUDE_CODE_OAUTH_TOKEN in its
# environment: a task, a process of the same user, can read /proc/<agent pid>/environ. The agent
# holds them in memory only. It gives the Claude token to each Claude Code run's environment,
# where Claude Code needs it, and the write token as GH_TOKEN to --github tasks alone; the seed
# never leaves it. Every other task has the read-only GH_TOKEN the login shell loads from
# ~/.env.d/github.env. Without a write token the agent runs, and its --github tasks fail. The CA
# certificate is not secret: it is kept as bus-ca.pem in the guest's agent-bus state directory
# and given to the agent as AGENT_BUS_CA, the only CA it trusts.
# The bus runs on the host at the guest's default gateway, the lab's management bridge. A guest
# in another lab, such as tests/environments/agent, sets AGENT_BUS_URL to where the bus listens.
#
#   start-guest.sh [name]      # name defaults to dev
#
# One agent runs per checkout: any agent of this name, and any agent of another name working in
# this checkout (an agent started under its old name before a rename), is stopped first.
#   AGENT_BUS_MODEL=<id>       # optional: the Claude model, default the binary's own
#   AGENT_BUS_URL=<url>        # optional: the bus, default nats://<default gateway>:4222
set -euo pipefail

NAME="${1:-dev}"
# Never exported, even if the login shell exported a variable of the same name.
export -n CLAUDE_TOKEN GITHUB_WRITE_TOKEN SEED
IFS= read -r CLAUDE_TOKEN || true
IFS= read -r GITHUB_WRITE_TOKEN || true
IFS= read -r SEED || true
CA_PEM="$(cat)"
if [ -z "${CLAUDE_TOKEN}" ] || [ -z "${SEED}" ]; then
    echo "Give the Claude subscription token, the GitHub write token (or an empty line) and the agent's bus seed on stdin, one per line, then the bus's CA certificate" >&2
    exit 2
fi
if [[ "${GITHUB_WRITE_TOKEN}" == SU* ]]; then
    echo "The second line on stdin is an NKey seed, not a GitHub token: start the agent with an agent-bus-start-agent of this version" >&2
    exit 2
fi
if [[ "${CA_PEM}" != "-----BEGIN CERTIFICATE-----"*"-----END CERTIFICATE-----" ]]; then
    echo "No CA certificate after the seed on stdin: start the agent with an agent-bus-start-agent of this version" >&2
    exit 2
fi
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

# The bus's CA certificate, replaced atomically, readable: it is not secret.
case "${XDG_STATE_HOME:-}" in
    /*) STATE_DIR="${XDG_STATE_HOME}/agent-bus" ;;
    *) STATE_DIR="${HOME}/.local/state/agent-bus" ;;
esac
mkdir -p "${STATE_DIR}"
chmod 700 "${STATE_DIR}"
CA_FILE="${STATE_DIR}/bus-ca.pem"
printf '%s\n' "${CA_PEM}" > "${CA_FILE}.$$.tmp"
chmod 644 "${CA_FILE}.$$.tmp"
mv -f "${CA_FILE}.$$.tmp" "${CA_FILE}"

cd "${REPO}"
cargo build --release --quiet -p agent-bus
# Stop the old agent and wait until it has exited before starting the new one: the two must
# never share the state file, nor the checkout. That is any agent of this name, and any agent
# working in this checkout under another name. A stopping agent stops its Claude Code within its
# own kill grace, so it gets longer than that before SIGKILL. The new agent also refuses to start
# while another of its name holds the lock.
REPO_PATTERN="$(printf '%s' "${REPO}" | sed 's/[][\.*^$+?(){}|]/\\&/g')"
PATTERN="[a]gent-bus guest (.*--name ${NAME} |.*--workdir ${REPO_PATTERN}"'( |$))'
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
    echo "Stopping guest agent(s) $(pgrep -af "${PATTERN}" | grep -o -- '--name [a-z0-9-]*' | cut -d' ' -f2 | sort -u | tr '\n' ' ')in ${REPO}" >&2
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
# The secrets go in on the agent's stdin, never in its environment: a task can read the agent's
# /proc/<pid>/environ. env -u drops any copy the login shell exported; the agent refuses to start
# with the seed or the Claude token in its environment.
printf '%s\n%s\n%s\n' "${CLAUDE_TOKEN}" "${GITHUB_WRITE_TOKEN}" "${SEED}" |
    AGENT_BUS_CA="${CA_FILE}" nohup env -u AGENT_BUS_SEED -u CLAUDE_CODE_OAUTH_TOKEN \
    target/release/agent-bus guest --server "${URL}" --name "${NAME}" \
    --workdir "${REPO}" "${MODEL_ARGS[@]}" > "${LOG}" 2>&1 &
unset CLAUDE_TOKEN SEED GITHUB_WRITE_TOKEN
sleep 2
if pgrep -f "[a]gent-bus guest .*--name ${NAME} " > /dev/null; then
    echo "Guest agent '${NAME}' is running; log: ${LOG}"
else
    cat "${LOG}"
    exit 1
fi
