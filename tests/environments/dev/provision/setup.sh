#!/bin/bash
#
# Builds the Sherpa dev server: toolchains, the sherpa and sherpa-docs checkouts, and a Sherpa
# server installed with the checkout's own installer. Runs as the node's login user through the
# SSH provisioner; safe to run again with `sherpa provision`.

set -euo pipefail

CODE_DIR="${HOME}/code/rust"
DOCS_VENV="${HOME}/.venvs/sherpa-docs"
# The host's version. 0.12 links a system LLVM of a version Ubuntu 24.04 does not ship.
BPF_LINKER_VERSION="0.10.2"
REPOS=(bwks/sherpa bwks/sherpa-docs)
SHERPA_BASE="/opt/sherpa"
# Login shells load ~/.env.d/, so `sherpa login` finds the account here too.
ACCOUNT_ENV="${HOME}/.env.d/sherpa-account.env"
SHERPA_ACCOUNT="dev"
SERVER_IP=""
APT_PACKAGES=(
    build-essential pkg-config libvirt-dev clang llvm libelf-dev
    git gh jq rsync curl python3-venv shellcheck bats
)

log() {
    echo "==> $*"
}

apt_install() {
    local attempt
    # The Ubuntu archive is sometimes mid-sync and serves an index of the wrong size.
    for attempt in 1 2 3 4 5; do
        if sudo apt-get update -qq && \
            sudo DEBIAN_FRONTEND=noninteractive apt-get install -y -qq "${APT_PACKAGES[@]}"; then
            return 0
        fi
        log "apt failed (attempt ${attempt}), retrying in 30s"
        sleep 30
    done
    return 1
}

install_rust() {
    if ! command -v rustup >/dev/null 2>&1 && [ ! -x "${HOME}/.cargo/bin/rustup" ]; then
        log "Installing rustup"
        curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | \
            sh -s -- -y --profile minimal --component rustfmt,clippy
    fi
    # shellcheck source=/dev/null
    source "${HOME}/.cargo/env"
    rustup toolchain install stable --profile minimal --component rustfmt,clippy,llvm-tools
    # The eBPF program builds on nightly with build-std and links with bpf-linker.
    rustup toolchain install nightly --profile minimal --component rust-src
    if ! command -v bpf-linker >/dev/null 2>&1; then
        log "Installing bpf-linker"
        cargo install --locked "bpf-linker@${BPF_LINKER_VERSION}"
    fi
}

setup_git() {
    if [ -z "${GH_TOKEN:-}" ]; then
        echo "GH_TOKEN is not set; bring the lab up with it in your environment" >&2
        exit 1
    fi
    gh auth setup-git
    # Commit as the token's GitHub account, with its no-reply address.
    local login id name
    login="$(gh api user --jq .login)"
    id="$(gh api user --jq .id)"
    name="$(gh api user --jq '.name // .login')"
    git config --global user.name "${name}"
    git config --global user.email "${id}+${login}@users.noreply.github.com"
    git config --global init.defaultBranch main
    git config --global pull.ff only
}

clone_repos() {
    local repo dir
    mkdir -p "${CODE_DIR}"
    for repo in "${REPOS[@]}"; do
        dir="${CODE_DIR}/${repo#*/}"
        if [ -d "${dir}/.git" ]; then
            # Never touch work in progress; only fetch.
            log "Fetching ${repo}"
            git -C "${dir}" fetch --prune origin
        else
            log "Cloning ${repo}"
            git clone "https://github.com/${repo}.git" "${dir}"
        fi
    done
}

setup_docs_venv() {
    if [ ! -x "${DOCS_VENV}/bin/mkdocs" ]; then
        log "Creating the sherpa-docs virtualenv"
        python3 -m venv "${DOCS_VENV}"
    fi
    "${DOCS_VENV}/bin/pip" install -q -r "${CODE_DIR}/sherpa-docs/requirements.txt"
}

install_sherpa_server() {
    if [ -x "${SHERPA_BASE}/bin/sherpad" ]; then
        log "Sherpa server already installed"
        return 0
    fi
    local db_password
    # The guest's own database; the installer keeps it in /opt/sherpa/env/sherpa.env.
    db_password="$(openssl rand -hex 24)"
    log "Installing the Sherpa server on ${SERVER_IP}"
    sudo SHERPA_DB_PASSWORD="${db_password}" SHERPA_SERVER_IPV4="${SERVER_IP}" \
        bash "${CODE_DIR}/sherpa/scripts/sherpa_install.sh"
}

# Random, with every character class the server's password rules ask for.
new_password() {
    echo "$(openssl rand -hex 20)Sx_"
}

# Passwords and an SSH key for the server's accounts, made once and kept only in the guest.
create_account_settings() {
    if [ ! -f "${HOME}/.ssh/id_ed25519" ]; then
        ssh-keygen -q -t ed25519 -N "" -C "${SHERPA_ACCOUNT}@sherpa-dev" -f "${HOME}/.ssh/id_ed25519"
    fi
    if [ -f "${ACCOUNT_ENV}" ]; then
        return 0
    fi
    log "Creating the server account settings in ${ACCOUNT_ENV}"
    mkdir -p "$(dirname "${ACCOUNT_ENV}")"
    (
        umask 077
        cat > "${ACCOUNT_ENV}" << EOF
SHERPA_ADMIN_PASSWORD=$(new_password)
SHERPA_USER_NAME=${SHERPA_ACCOUNT}
SHERPA_USER_PASSWORD=$(new_password)
SHERPA_USER_IS_ADMIN=yes
SHERPA_USER_SSH_KEY=${HOME}/.ssh/id_ed25519.pub
EOF
    )
}

# Creates the server's config and accounts without prompts, from the account settings. `init`
# keeps an existing config, SSH identity and accounts, so it runs every time.
init_sherpa_server() {
    log "Initializing the Sherpa server"
    (
        set -a
        # shellcheck source=/dev/null
        source "${ACCOUNT_ENV}"
        set +a
        sudo -u sherpa -H \
            --preserve-env=SHERPA_ADMIN_PASSWORD,SHERPA_USER_NAME,SHERPA_USER_PASSWORD,SHERPA_USER_IS_ADMIN,SHERPA_USER_SSH_KEY \
            "${SHERPA_BASE}/bin/sherpad" init
    )
    sudo systemctl start sherpad
}

# Points the CLI at this server, trusting its own certificate, and logs in.
login_sherpa_client() {
    mkdir -p "${HOME}/.sherpa/config"
    cat > "${HOME}/.sherpa/config/sherpa.toml" << EOF
server_ipv4 = "${SERVER_IP}"

[server_connection]
timeout_secs = 10
validate_certs = true
insecure = false
ca_cert_path = "${SHERPA_BASE}/.certs/server.crt"
EOF
    local attempt
    for attempt in $(seq 1 30); do
        if (
            set -a
            # shellcheck source=/dev/null
            source "${ACCOUNT_ENV}"
            set +a
            "${SHERPA_BASE}/bin/sherpa" login
        ); then
            return 0
        fi
        log "Server not ready (attempt ${attempt}), retrying in 5s"
        sleep 5
    done
    return 1
}

main() {
    apt_install
    install_rust
    setup_git
    clone_repos
    setup_docs_venv
    SERVER_IP="$(ip -4 -json route get 1.1.1.1 | jq -r '.[0].prefsrc')"
    install_sherpa_server
    create_account_settings
    init_sherpa_server
    login_sherpa_client
    log "Done. Logged in to the Sherpa server on ${SERVER_IP} as ${SHERPA_ACCOUNT}"
}

main "$@"
