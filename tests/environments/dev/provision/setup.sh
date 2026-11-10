#!/bin/bash
#
# Builds the Sherpa dev server: toolchains, the sherpa and sherpa-docs checkouts, and a Sherpa
# server installed with the checkout's own installer. Runs as the node's login user through the
# SSH provisioner; safe to run again with `sherpa provision`. Both dev servers, dev and agent-2,
# run every step.

set -euo pipefail

CODE_DIR="${HOME}/code/rust"
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
    git jq rsync curl
)
# Claude Code plugins for the guest agents, at a pinned commit. The marketplace is a local clone:
# a marketplace source can name a branch or a tag but not a commit, and this repository has no
# tags. Claude Code loads the plugin in place from the clone.
CLAUDE_PLUGIN_REPO="https://github.com/morganlinton/BeautifulRust.git"
CLAUDE_PLUGIN_COMMIT="edd3e86087eda07983251056508fe3b8856d85d7"
CLAUDE_PLUGIN_DIR="${HOME}/.local/share/claude-marketplaces/BeautifulRust"

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

# Runs a command up to 5 times, 30s apart, logging each failure. GitHub release downloads, which
# the mise installer and `mise install` make, sometimes answer with an HTTP 504.
retry() {
    local attempt
    for attempt in 1 2 3 4 5; do
        if "$@"; then
            return 0
        fi
        log "'$*' failed (attempt ${attempt}), retrying in 30s"
        sleep 30
    done
    return 1
}

run_mise_installer() {
    curl -fsSL --retry 5 --retry-all-errors --retry-delay 10 https://mise.run | sh
}

# Installs mise and the tools in ~/.config/mise/config.toml, which the provisioner copies from
# provision/mise.toml, and puts them on the PATH of login shells and of this script.
install_tools() {
    if [ ! -x "${MISE}" ]; then
        log "Installing mise"
        retry run_mise_installer
    fi
    log "Installing tools with mise"
    # provision/sccache.env sets RUSTC_WRAPPER=sccache, but mise builds cargo:bpf-linker with
    # cargo, possibly before sccache is installed, so that build runs without the wrapper.
    retry env -u RUSTC_WRAPPER "${MISE}" install --yes
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

# git authenticates through `gh auth git-credential`, which takes GH_TOKEN from the environment
# of each git command. GH_TOKEN here, and in ~/.env.d/github.env, is the read-only token
# (provision/github.env fills it from GH_TOKEN_RO on the host): fetches and clones work, a push
# is refused by GitHub. A push works only in an agent-bus --github task, whose Claude Code has the
# write token as GH_TOKEN, sent by the host for that agent and held in its memory, never on disk.
setup_git() {
    if [ -z "${GH_TOKEN:-}" ]; then
        echo "GH_TOKEN is not set; bring the lab up with GH_TOKEN_RO (the read-only token) in your environment" >&2
        exit 1
    fi
    gh auth setup-git
    # Commit as the token's GitHub account, with its no-reply address. GET /user needs no
    # permission beyond the token itself, so the read-only token is enough.
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
        # mise refuses to run tools from a checkout with an untrusted mise.toml.
        if [ -f "${dir}/mise.toml" ]; then
            "${MISE}" trust "${dir}/mise.toml"
        fi
    done
}

# mkdocs for building sherpa-docs, as a uv tool in ~/.local/bin, at the versions in the docs'
# requirements.txt. Running it again picks up changed versions. It runs on a uv-managed Python
# 3.15, never the system python3.
install_docs_tools() {
    log "Installing mkdocs for sherpa-docs"
    uv tool install --quiet --managed-python --python 3.15 mkdocs --with-requirements "${CODE_DIR}/sherpa-docs/requirements.txt"
}

# The beautiful-rust skills (rust-best-practices, rust-review, rust-port) for Claude Code, from
# the clone checked out at CLAUDE_PLUGIN_COMMIT. Running it again moves the clone to that commit;
# adding the marketplace and installing the plugin again are no-ops.
install_claude_plugins() {
    log "Installing the beautiful-rust Claude Code plugin"
    if [ ! -d "${CLAUDE_PLUGIN_DIR}/.git" ]; then
        git clone --quiet "${CLAUDE_PLUGIN_REPO}" "${CLAUDE_PLUGIN_DIR}"
    fi
    git -C "${CLAUDE_PLUGIN_DIR}" fetch --quiet origin
    git -C "${CLAUDE_PLUGIN_DIR}" checkout --quiet --detach "${CLAUDE_PLUGIN_COMMIT}"
    claude plugin marketplace add "${CLAUDE_PLUGIN_DIR}"
    claude plugin install beautiful-rust@beautiful-rust
}

# Claude Code attribution as on the host: commits keep the default Co-Authored-By trailer, pull
# requests get no footer and nothing links to a session. Merged into the settings, so the plugin
# entries stay.
configure_claude_attribution() {
    local settings="${HOME}/.claude/settings.json"
    log "Setting the Claude Code attribution"
    mkdir -p "$(dirname "${settings}")"
    if [ ! -f "${settings}" ]; then
        echo '{}' > "${settings}"
    fi
    jq '.attribution = {"pr": "", "sessionUrl": false}' "${settings}" > "${settings}.tmp"
    mv "${settings}.tmp" "${settings}"
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
    # init opens the database, which a running server holds locked.
    sudo systemctl stop sherpad
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

# The SSH config Sherpa writes for each lab reaches its nodes through the server as the Sherpa
# user (`ProxyJump dev@<server>`), which lab tests and SSH provisioners use. As on any Sherpa
# server, that user needs an OS account there with its key, and this login needs to trust the
# server's host key, which is read here rather than scanned over the network.
allow_lab_jump() {
    if ! id "${SHERPA_ACCOUNT}" >/dev/null 2>&1; then
        log "Creating the ${SHERPA_ACCOUNT} OS account for SSH jumps to lab nodes"
        sudo useradd --create-home --shell /bin/bash "${SHERPA_ACCOUNT}"
    fi
    local home
    home="$(getent passwd "${SHERPA_ACCOUNT}" | cut -d: -f6)"
    sudo install -d -m 700 -o "${SHERPA_ACCOUNT}" -g "${SHERPA_ACCOUNT}" "${home}/.ssh"
    sudo install -m 600 -o "${SHERPA_ACCOUNT}" -g "${SHERPA_ACCOUNT}" \
        "${HOME}/.ssh/id_ed25519.pub" "${home}/.ssh/authorized_keys"
    local known_hosts="${HOME}/.ssh/known_hosts"
    local entry
    entry="${SERVER_IP} $(cut -d' ' -f1,2 /etc/ssh/ssh_host_ed25519_key.pub)"
    touch "${known_hosts}"
    if ! grep -qxF "${entry}" "${known_hosts}"; then
        echo "${entry}" >> "${known_hosts}"
    fi
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
    install_docs_tools
    install_claude_plugins
    configure_claude_attribution
    SERVER_IP="$(ip -4 -json route get 1.1.1.1 | jq -r '.[0].prefsrc')"
    install_sherpa_server
    create_account_settings
    init_sherpa_server
    login_sherpa_client
    allow_lab_jump
    log "Done. Logged in to the Sherpa server on ${SERVER_IP} as ${SHERPA_ACCOUNT}"
}

main "$@"
