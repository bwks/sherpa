#!/bin/bash
#
# Builds the Sherpa dev server: toolchains, the sherpa and sherpa-docs checkouts, and a Sherpa
# server installed with the checkout's own installer. Runs as the node's login user through the
# SSH provisioner; safe to run again with `sherpa provision`.

set -euo pipefail

CODE_DIR="${HOME}/code/rust"
DOCS_VENV="${HOME}/.venvs/sherpa-docs"
MISE="${HOME}/.local/bin/mise"
REPOS=(bwks/sherpa bwks/sherpa-docs)
SHERPA_BASE="/opt/sherpa"
# Login shells load ~/.env.d/, so `sherpa login` finds the account here too.
ACCOUNT_ENV="${HOME}/.env.d/sherpa-account.env"
SHERPA_ACCOUNT="dev"
SERVER_IP=""
# System libraries and compilers. Developer tools come from mise, set in provision/mise.toml.
APT_PACKAGES=(
    build-essential pkg-config libvirt-dev clang llvm libelf-dev
    git jq rsync curl python3-venv
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

# Installs mise and the tools in ~/.config/mise/config.toml, which the provisioner copies from
# provision/mise.toml, and puts them on the PATH of login shells and of this script.
install_tools() {
    if [ ! -x "${MISE}" ]; then
        log "Installing mise"
        curl -fsSL https://mise.run | sh
    fi
    log "Installing tools with mise"
    "${MISE}" install --yes
    local profile="${HOME}/.profile"
    # The provisioner's ~/.env.d loader goes in ~/.bash_profile when it exists, so does this.
    if [ -f "${HOME}/.bash_profile" ]; then
        profile="${HOME}/.bash_profile"
    fi
    local activate="eval \"\$(\$HOME/.local/bin/mise activate bash --shims)\""
    if ! grep -qxF "${activate}" "${profile}"; then
        echo "${activate}" >> "${profile}"
    fi
    eval "$("${MISE}" activate bash --shims)"
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
    log "Installing the Sherpa server on ${SERVER_IP}"
    sudo SHERPA_SERVER_IPV4="${SERVER_IP}" bash "${CODE_DIR}/sherpa/scripts/sherpa_install.sh"
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
    install_tools
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
