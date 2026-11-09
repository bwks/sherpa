#!/usr/bin/env bash
# Enroll a lab node's guest agent on the bus and start it, from the host. The agent is named
# after the node. Run it on the host, where the bus runs. The host has no checkout and builds
# nothing: the agent-bus binary and this script are built and copied from the dev guest, and
# installed as ~/.local/bin/agent-bus and ~/.local/bin/agent-bus-start-agent.
#
#   agent-bus-start-agent <lab dir> <node>
#
# It needs, in its environment, CLAUDE_CODE_OAUTH_TOKEN (from `claude setup-token`) and
# AGENT_BUS_SEED, the HOST's seed; a mise task loads both from the secret store. Optional:
# AGENT_BUS_MODEL, the guest's default model, passed on to start-guest.sh, and AGENT_BUS_BIN,
# the agent-bus binary to use (default: `agent-bus` on PATH).
#
# It finds the node's SSH alias (`<node>` or `<node>.<lab id>`) in the lab's .sherpa/ssh/config
# (with the lab id from .sherpa/lab-info.toml), and the bus's URL in the bus record that
# `agent-bus bus start` wrote. It then runs `agent-bus enroll <node>`, which gives the agent a fresh key and refuses its previous one, and
# pipes the Claude token, the new seed and the bus's CA certificate over SSH stdin into
# dev/agent-bus/start-guest.sh in the guest. The seed only travels through that pipe: it is never
# in a variable, a command line, a file or a log. The CA certificate (not secret) is the one
# `agent-bus bus start` made, tls/ca.pem in the host's state directory; the guest agent trusts
# only it.
#
# The lab folder must be in the .sherpa/ layout of sherpa 0.5.15. A folder from an older client
# (a loose lab-info.toml and sherpa_ssh_config) is refused: run any sherpa 0.5.15 command in it,
# such as `sherpa inspect`, which moves its files into .sherpa/.
set -euo pipefail

if [ "$#" -ne 2 ]; then
    echo "Usage: $0 <lab dir> <node>" >&2
    exit 2
fi
LAB_DIR="$1"
NODE="$2"
# Where the checkout is in the guest, relative to the home directory SSH starts in;
# provisioning clones it there.
GUEST_REPO="code/rust/sherpa"

# The node name is the agent's name and goes into the remote command: hold it to the agent name
# rule.
if ! [[ "${NODE}" =~ ^[a-z0-9-]{1,32}$ ]]; then
    echo "Node '${NODE}' is not a valid agent name: 1 to 32 lowercase letters, digits and hyphens" >&2
    exit 2
fi
if [ -z "${CLAUDE_CODE_OAUTH_TOKEN:-}" ] || [ -z "${AGENT_BUS_SEED:-}" ]; then
    echo "Set CLAUDE_CODE_OAUTH_TOKEN and AGENT_BUS_SEED (the host's seed) in the environment" >&2
    exit 2
fi

SSH_CONFIG="${LAB_DIR}/.sherpa/ssh/config"
LAB_INFO="${LAB_DIR}/.sherpa/lab-info.toml"
if [ ! -d "${LAB_DIR}/.sherpa" ] && [ -f "${LAB_DIR}/lab-info.toml" ]; then
    echo "${LAB_DIR} has a lab-info.toml but no .sherpa/ folder: it is in the old lab layout." >&2
    echo "Run any sherpa 0.5.15 command in that folder (such as \`sherpa inspect\`) to move its files into .sherpa/, then try again" >&2
    exit 1
fi
for file in "${SSH_CONFIG}" "${LAB_INFO}"; do
    if [ ! -f "${file}" ]; then
        echo "${file} not found: is ${LAB_DIR} a lab directory that is up?" >&2
        exit 1
    fi
done
case "${XDG_STATE_HOME:-}" in
    /*) STATE_HOME="${XDG_STATE_HOME}" ;;
    *) STATE_HOME="${HOME}/.local/state" ;;
esac
BUS_RECORD="${STATE_HOME}/agent-bus/bus.toml"
BUS_CA="${STATE_HOME}/agent-bus/tls/ca.pem"
for file in "${BUS_RECORD}" "${BUS_CA}"; do
    if [ ! -f "${file}" ]; then
        echo "${file} not found: start the bus with \`agent-bus bus start\` first" >&2
        exit 1
    fi
done

# The first top-level `key = "value"` string in a TOML file.
toml_string() {
    sed -n "s/^$1[[:space:]]*=[[:space:]]*\"\([^\"]*\)\".*/\1/p" "$2" | head -n 1
}

LAB_ID="$(toml_string id "${LAB_INFO}")"
URL="$(toml_string url "${BUS_RECORD}")"
if ! [[ "${URL}" =~ ^nats://[0-9.]+:[0-9]+$ ]]; then
    echo "The bus URL in ${BUS_RECORD} is not nats://<ipv4>:<port>" >&2
    exit 1
fi

ALIAS=""
HOSTS="$(awk 'tolower($1) == "host" { for (i = 2; i <= NF; i++) print $i }' "${SSH_CONFIG}")"
for candidate in "${NODE}" "${NODE}.${LAB_ID}"; do
    if grep -qxF -- "${candidate}" <<< "${HOSTS}"; then
        ALIAS="${candidate}"
        break
    fi
done
if [ -z "${ALIAS}" ]; then
    echo "No host '${NODE}' or '${NODE}.${LAB_ID}' in ${SSH_CONFIG}" >&2
    exit 1
fi

MODEL_ENV=""
if [ -n "${AGENT_BUS_MODEL:-}" ]; then
    if ! [[ "${AGENT_BUS_MODEL}" =~ ^[a-z0-9.-]+$ ]]; then
        echo "AGENT_BUS_MODEL '${AGENT_BUS_MODEL}' is not a model alias or ID" >&2
        exit 2
    fi
    MODEL_ENV="AGENT_BUS_MODEL=${AGENT_BUS_MODEL} "
fi
REMOTE="AGENT_BUS_URL=${URL} ${MODEL_ENV}bash -lc '${GUEST_REPO}/dev/agent-bus/start-guest.sh ${NODE}'"

AGENT_BUS="$(command -v "${AGENT_BUS_BIN:-agent-bus}" || true)"
if [ -z "${AGENT_BUS}" ] || [ ! -f "${AGENT_BUS}" ] || [ ! -x "${AGENT_BUS}" ]; then
    echo "agent-bus binary '${AGENT_BUS_BIN:-agent-bus}' not found or not executable: install it to ~/.local/bin or set AGENT_BUS_BIN" >&2
    exit 1
fi

# The lab's SSH config names its key and known hosts by absolute path; ssh runs from the lab
# directory so the config's path is the one `sherpa` documents. ssh never gets the secrets in
# its environment.
cd "${LAB_DIR}"
lab_ssh() {
    env -u AGENT_BUS_SEED -u CLAUDE_CODE_OAUTH_TOKEN ssh -F .sherpa/ssh/config "${ALIAS}" "$@"
}

# Reach the guest before enrolling: enrolling refuses the agent's previous key at once.
if ! lab_ssh true < /dev/null; then
    echo "Unable to reach ${ALIAS} over SSH; nothing was enrolled" >&2
    exit 1
fi

echo "Enrolling '${NODE}' on ${URL} and starting it on ${ALIAS}" >&2
# On the guest script's stdin: the token, then the seed straight from enroll, each on a line,
# then the CA certificate to the end. If enroll fails, the guest script gets no seed and exits
# without touching the running agent.
{
    printf '%s\n' "${CLAUDE_CODE_OAUTH_TOKEN}"
    "${AGENT_BUS}" enroll "${NODE}" || exit 1
    cat "${BUS_CA}"
} | lab_ssh "${REMOTE}"
