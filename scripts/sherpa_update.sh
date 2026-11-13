#!/bin/bash

################################################################################
# Sherpa Update Script
################################################################################
#
# Lightweight update script that downloads the latest Sherpa binaries from
# GitHub releases, stops the service, replaces the binaries, and restarts
# the service. Does NOT re-run full installation (packages, users, etc.).
#
# Servers older than 0.5.0 kept their data in a SurrealDB container. This
# script cannot move that data, so it refuses such a server before anything
# changes, and tells the user to update to 0.5.12 first with that release's
# update script, which moves the data into sherpad's embedded database.
#
# Requirements:
#   - Root/sudo access
#   - curl installed
#   - Sherpa already installed at /opt/sherpa/bin/sherpad
#
# Usage:
#   sudo ./sherpa_update.sh
#   sudo ./sherpa_update.sh --version v0.3.18
#
################################################################################

set -e  # Exit on error

# Configuration
SHERPA_BASE_DIR="/opt/sherpa"
SHERPA_BIN_DIR="${SHERPA_BASE_DIR}/bin"
SHERPA_LABS_DIR="${SHERPA_BASE_DIR}/labs"
SHERPA_POOL_DIR="${SHERPA_BASE_DIR}/libvirt/images"
SHERPA_DB_FILE="${SHERPA_BASE_DIR}/db/sherpa.sqlite"
# The SurrealDB container of releases before 0.5.0, and the data it kept on the host.
LEGACY_DB_CONTAINER="sherpa-db"
LEGACY_DB_DATA="${SHERPA_BASE_DIR}/db/sherpa.db"
# The last release with 'sherpad migrate-db', which moves the data of a server older than 0.5.0.
LAST_MIGRATING_VERSION="v0.5.12"

# GitHub release configuration
GITHUB_REPO="bwks/sherpa"
GITHUB_API_URL="https://api.github.com/repos/${GITHUB_REPO}/releases/latest"
GITHUB_DOWNLOAD_URL="https://github.com/${GITHUB_REPO}/releases/download"
TARGET="x86_64-unknown-linux-gnu"
SHERPA_VERSION=""
FORCE=false
SHERPA_ARTIFACT_DIR="${SHERPA_ARTIFACT_DIR:-}"

# Color codes for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

################################################################################
# Helper Functions
################################################################################

print_error() {
    echo -e "${RED}[ERROR]${NC} $1" >&2
}

print_success() {
    echo -e "${GREEN}[SUCCESS]${NC} $1"
}

print_info() {
    echo -e "${BLUE}[INFO]${NC} $1"
}

print_warning() {
    echo -e "${YELLOW}[WARNING]${NC} $1"
}

print_header() {
    echo ""
    echo "========================================="
    echo "  $1"
    echo "========================================="
    echo ""
}

show_usage() {
    cat << EOF
Usage: $0 [OPTIONS]

Update Sherpa binaries to the latest (or specified) release.

This script downloads new binaries from GitHub releases, stops the sherpad
service, replaces the binaries, and restarts the service. It does NOT
reinstall system packages, users or Docker. A server older than 0.5.0 is
refused: update it to ${LAST_MIGRATING_VERSION} first with that release's update
script, which moves its data into the embedded database.

Options:
  --version VERSION     Install a specific version (e.g. v0.3.18)
                        If omitted, the latest release is used
  --force               Force update even if already at the target version
  -h, --help            Show this help message

Environment Variables:
  SHERPA_ARTIFACT_DIR  Local release archives instead of GitHub downloads
                      Requires --version; expects the normal release filenames

Examples:
  # Update to latest release
  sudo $0

  # Update to a specific version
  sudo $0 --version v0.3.18

  # Force reinstall of current version
  sudo $0 --force

EOF
}

check_command_exists() {
    command -v "$1" >/dev/null 2>&1
}

################################################################################
# Pre-flight Checks
################################################################################

check_root_privileges() {
    print_info "Checking root privileges..."

    if [ "$EUID" -ne 0 ]; then
        print_error "This script must be run as root or with sudo"
        echo "Please run: sudo $0"
        exit 1
    fi

    print_success "Running as root"
}

check_curl_installed() {
    print_info "Checking curl installation..."

    if ! check_command_exists curl; then
        print_error "curl is not installed"
        echo ""
        echo "Please install curl first:"
        echo "  sudo apt-get install curl"
        echo ""
        exit 1
    fi

    print_success "curl is installed"
}

check_sherpa_installed() {
    print_info "Checking existing Sherpa installation..."

    if [ ! -x "${SHERPA_BIN_DIR}/sherpad" ]; then
        print_error "sherpad not found at ${SHERPA_BIN_DIR}/sherpad"
        echo ""
        echo "Sherpa does not appear to be installed."
        echo "Please run sherpa_install.sh for a full installation."
        echo ""
        exit 1
    fi

    print_success "Sherpa installation found at ${SHERPA_BIN_DIR}"
}

################################################################################
# Version Detection
################################################################################

get_latest_version() {
    print_info "Fetching latest release version..."

    SHERPA_VERSION=$(curl -sf "${GITHUB_API_URL}" | grep '"tag_name"' | sed -E 's/.*"tag_name": *"([^"]+)".*/\1/')
    if [ -z "$SHERPA_VERSION" ]; then
        print_error "Failed to determine latest release version"
        print_error "Check your internet connection or specify a version with --version"
        exit 1
    fi

    print_success "Latest release: ${SHERPA_VERSION}"
}

check_binary_needs_update() {
    local binary="$1"
    local binary_path="${SHERPA_BIN_DIR}/${binary}"

    if [ ! -x "$binary_path" ]; then
        # Binary not installed — needs update
        return 0
    fi

    local current_version
    current_version=$("$binary_path" --version 2>/dev/null | awk '{print $NF}') || true

    if [ -z "$current_version" ]; then
        print_warning "Could not determine current version of ${binary} — will update"
        return 0
    fi

    local current_normalized="${current_version#v}"
    local target_normalized="${SHERPA_VERSION#v}"

    if [ "$current_normalized" = "$target_normalized" ]; then
        return 1  # Already up to date
    fi

    return 0  # Needs update
}

################################################################################
# Service Management
################################################################################

stop_sherpad() {
    print_info "Stopping sherpad..."

    if command -v systemctl >/dev/null 2>&1 && systemctl is-active --quiet sherpad 2>/dev/null; then
        print_info "Stopping sherpad service..."
        systemctl stop sherpad
    fi

    # Kill any remaining sherpad process not managed by systemd
    if pgrep -x sherpad >/dev/null 2>&1; then
        print_info "Stopping running sherpad process..."
        pkill -x sherpad || true
        sleep 1
        # Force kill if still running
        if pgrep -x sherpad >/dev/null 2>&1; then
            pkill -9 -x sherpad || true
            sleep 1
        fi
    fi

    print_success "sherpad stopped"
}

start_sherpad() {
    print_info "Starting sherpad..."

    if command -v systemctl >/dev/null 2>&1 && systemctl is-enabled --quiet sherpad 2>/dev/null; then
        systemctl restart sherpad
        sleep 2

        if systemctl is-active --quiet sherpad 2>/dev/null; then
            print_success "sherpad service started"
        else
            print_error "sherpad service failed to start"
            echo ""
            echo "Check logs with: journalctl -u sherpad --no-pager -n 20"
            exit 1
        fi
    else
        print_warning "systemd service not found or not enabled — skipping auto-start"
        print_info "Start sherpad manually if needed"
    fi
}

################################################################################
# Binary Update
################################################################################

update_binaries() {
    print_header "Downloading Binaries"

    # Create a temp directory for downloads, clean up on exit
    local tmp_dir
    tmp_dir=$(mktemp -d)
    trap 'rm -rf "${tmp_dir}"' EXIT

    # Download and install each binary
    # sherpad is required; sherpa is optional
    local binaries="sherpad sherpa"
    local required_binaries="sherpad"

    for binary in $binaries; do
        # Check if this binary needs updating
        if [ "$FORCE" = false ] && ! check_binary_needs_update "$binary"; then
            local current_ver
            current_ver=$("${SHERPA_BIN_DIR}/${binary}" --version 2>/dev/null | awk '{print $NF}') || true
            print_success "${binary} is already at ${current_ver} — skipping"
            continue
        fi

        local asset="${binary}-${TARGET}.tar.gz"
        local url="${GITHUB_DOWNLOAD_URL}/${SHERPA_VERSION}/${asset}"
        local is_required=false

        for req in $required_binaries; do
            if [ "$binary" = "$req" ]; then
                is_required=true
                break
            fi
        done

        if [ "$FORCE" = true ]; then
            print_info "Force downloading ${asset}..."
        else
            print_info "Downloading ${asset}..."
        fi

        local downloaded=false
        if [ -n "$SHERPA_ARTIFACT_DIR" ]; then
            if cp "${SHERPA_ARTIFACT_DIR}/${asset}" "${tmp_dir}/${asset}"; then
                downloaded=true
            fi
        elif curl -fsSL --connect-timeout 15 --retry 3 --retry-all-errors -o "${tmp_dir}/${asset}" "${url}"; then
            downloaded=true
        fi

        if [ "$downloaded" = true ]; then
            print_success "Downloaded ${asset}"

            # Extract and install
            print_info "Installing ${binary}..."
            tar xzf "${tmp_dir}/${asset}" -C "${tmp_dir}"

            if [ ! -f "${tmp_dir}/${binary}" ]; then
                if [ "$is_required" = true ]; then
                    print_error "Expected binary '${binary}' not found in archive"
                    exit 1
                else
                    print_warning "Expected binary '${binary}' not found in archive — skipping"
                    continue
                fi
            fi

            if [ "$binary" = "sherpad" ]; then
                check_new_sherpad_can_use_database "${tmp_dir}/sherpad"
            fi

            cp "${tmp_dir}/${binary}" "${SHERPA_BIN_DIR}/${binary}"
            chmod 755 "${SHERPA_BIN_DIR}/${binary}"
            chown sherpa:sherpa "${SHERPA_BIN_DIR}/${binary}"
            print_success "Installed ${SHERPA_BIN_DIR}/${binary}"

            # Clean up extracted binary for next iteration
            rm -f "${tmp_dir}/${binary}"
        else
            if [ "$is_required" = true ]; then
                print_error "Failed to download required binary: ${asset}"
                print_error "URL: ${url}"
                exit 1
            else
                print_warning "Optional binary not available: ${asset} — skipping"
            fi
        fi
    done

    print_success "All binaries updated to ${SHERPA_VERSION}"
}

ensure_symlinks() {
    print_info "Ensuring symlinks in /usr/local/bin..."

    if [ -x "${SHERPA_BIN_DIR}/sherpad" ]; then
        ln -sf "${SHERPA_BIN_DIR}/sherpad" /usr/local/bin/sherpad
    fi

    if [ -x "${SHERPA_BIN_DIR}/sherpa" ]; then
        ln -sf "${SHERPA_BIN_DIR}/sherpa" /usr/local/bin/sherpa
    fi

    print_success "Symlinks up to date"
}

################################################################################
# Upgrade Check
################################################################################

# True when a server older than 0.5.0 still keeps its data only in SurrealDB: there is no
# embedded database file, and there is the sherpa-db container or the data it kept on the host.
has_legacy_database() {
    [ ! -f "${SHERPA_DB_FILE}" ] || return 1
    [ -e "${LEGACY_DB_DATA}" ] && return 0
    command -v docker >/dev/null 2>&1 || return 1
    docker ps -a --format '{{.Names}}' 2>/dev/null | grep -qx "${LEGACY_DB_CONTAINER}"
}

# True when release $1 is newer than release $2, such as v0.6.1 and v0.6.0.
version_newer_than() {
    local candidate="${1#v}" reference="${2#v}"
    [ "${candidate}" != "${reference}" ] &&
        [ "$(printf '%s\n%s\n' "${candidate}" "${reference}" | sort -V | tail -n 1)" = "${candidate}" ]
}

# Refuse, before anything changes, to update a server whose data is still in SurrealDB. The
# release it updates to would start on an empty database and leave that data behind.
check_legacy_database() {
    print_info "Checking for a server older than 0.5.0..."

    if has_legacy_database; then
        print_error "This server keeps its data in the SurrealDB database of a Sherpa release older than 0.5.0"
        print_error "(the ${LEGACY_DB_CONTAINER} container or ${LEGACY_DB_DATA}). This script cannot move that data."
        echo ""
        echo "Update to ${LAST_MIGRATING_VERSION} first with that release's update script, which moves the"
        echo "data into the embedded database, then run this script again:"
        echo ""
        echo "  curl -fsSL https://raw.githubusercontent.com/${GITHUB_REPO}/${LAST_MIGRATING_VERSION}/scripts/sherpa_update.sh | sudo bash -s -- --version ${LAST_MIGRATING_VERSION}"
        echo ""
        echo "It moves the data out of the ${LEGACY_DB_CONTAINER} container, so that container must still exist."
        echo ""
        exit 1
    fi

    print_success "No SurrealDB database from an older release"
}

# Restrict lab data to sherpa, its group and QEMU (kvm). Directory modes also cover files
# created by older releases. sherpad enforces the same modes at startup.
secure_lab_storage() {
    print_info "Restricting lab storage..."

    install -d -o sherpa -g sherpa -m 2750 "${SHERPA_LABS_DIR}"
    if getent group kvm >/dev/null 2>&1; then
        install -d -o sherpa -g kvm -m 2750 "${SHERPA_POOL_DIR}"
    else
        print_warning "Group kvm not found: ${SHERPA_POOL_DIR} left readable by sherpa only"
        install -d -o sherpa -g sherpa -m 2750 "${SHERPA_POOL_DIR}"
    fi

    print_success "Lab storage restricted"
}

# Restrict the database directory to sherpa and its group: the database and its backups hold
# password hashes. Releases before this left it world-readable. sherpad enforces the same mode.
secure_database_storage() {
    if [ -d "$(dirname "${SHERPA_DB_FILE}")" ]; then
        chmod 750 "$(dirname "${SHERPA_DB_FILE}")"
        print_success "Database storage restricted"
    fi
}

# Units installed before container secrets lack /run/sherpa, so add it with a drop-in.
ensure_runtime_directory() {
    local unit="/etc/systemd/system/sherpad.service"
    local dropin_dir="/etc/systemd/system/sherpad.service.d"

    if [ ! -f "${unit}" ] || grep -q '^RuntimeDirectory=' "${unit}"; then
        return 0
    fi

    print_info "Adding /run/sherpa to the sherpad service..."
    install -d -m 755 "${dropin_dir}"
    cat > "${dropin_dir}/runtime-directory.conf" << 'UNIT'
[Service]
# RAM-backed /run/sherpa (0700 sherpa) for container secret files. Kept when sherpad
# stops or restarts, so running containers keep their secrets; gone after a reboot.
RuntimeDirectory=sherpa
RuntimeDirectoryMode=0700
RuntimeDirectoryPreserve=yes
UNIT
    chmod 644 "${dropin_dir}/runtime-directory.conf"
    systemctl daemon-reload
    print_success "sherpad service uses /run/sherpa"
}

################################################################################
# Server SSH Key
################################################################################

# The first release without the server's shared SSH key, which labs created before lab keys log
# in to their nodes with.
FIRST_RELEASE_WITHOUT_SERVER_KEY="v0.6.0"
SERVER_SSH_KEY_DIR="${SHERPA_BASE_DIR}/ssh"

# Refuse, before it is installed, a new sherpad that cannot use this server's database. From
# ${FIRST_RELEASE_WITHOUT_SERVER_KEY} it refuses labs that still use the server's key, and names
# them. The running release, stopped for the update, is started again unchanged.
check_new_sherpad_can_use_database() {
    local new_sherpad="$1"
    if [ ! -f "${SHERPA_DB_FILE}" ] ||
        version_newer_than "${FIRST_RELEASE_WITHOUT_SERVER_KEY}" "${SHERPA_VERSION}"; then
        return 0
    fi

    print_info "Checking that ${SHERPA_VERSION} can use the database..."
    # Run it as the service runs, so it creates no file sherpad cannot open. The download folder
    # is root's alone, so open it to sherpa; it holds only the release archives.
    chmod 755 "$(dirname "${new_sherpad}")"
    if ! runuser -u sherpa -- "${new_sherpad}" check-database </dev/null; then
        print_error "${SHERPA_VERSION} cannot use this server's database; nothing was installed."
        print_error "Destroy the labs named above with the running release, then update again."
        start_sherpad
        exit 1
    fi
    print_success "${SHERPA_VERSION} can use the database"
}

# Delete the server's shared SSH key pair, which no release from
# ${FIRST_RELEASE_WITHOUT_SERVER_KEY} uses, and its folder once empty. Says so only when a key
# file or the folder was there to remove: every update from that release on runs this.
remove_server_ssh_key() {
    if version_newer_than "${FIRST_RELEASE_WITHOUT_SERVER_KEY}" "${SHERPA_VERSION}"; then
        return 0
    fi
    local removed=false
    local key
    for key in "${SERVER_SSH_KEY_DIR}"/sherpa_ssh_key*; do
        # An unmatched pattern stays as it is, naming no file.
        [ -e "${key}" ] || continue
        rm -f "${key}"
        removed=true
    done
    if [ -d "${SERVER_SSH_KEY_DIR}" ] && rmdir "${SERVER_SSH_KEY_DIR}" 2>/dev/null; then
        removed=true
    fi
    if [ "${removed}" = true ]; then
        print_success "Removed the server's shared SSH key"
    fi
}

################################################################################
# Verification
################################################################################

verify_update() {
    print_header "Verifying Update"

    if [ ! -x "${SHERPA_BIN_DIR}/sherpad" ]; then
        print_error "sherpad binary verification failed"
        exit 1
    fi

    local new_version
    new_version=$("${SHERPA_BIN_DIR}/sherpad" --version 2>/dev/null) || true

    if [ -n "$new_version" ]; then
        print_success "Updated: ${new_version}"
    else
        print_success "Binaries replaced with ${SHERPA_VERSION}"
    fi
}

################################################################################
# Main Script
################################################################################

main() {
    # Parse command line arguments
    while [ $# -gt 0 ]; do
        case "$1" in
            --version)
                SHERPA_VERSION="$2"
                shift 2
                ;;
            --force)
                FORCE=true
                shift
                ;;
            -h|--help)
                show_usage
                exit 0
                ;;
            *)
                print_error "Unknown option: $1"
                show_usage
                exit 1
                ;;
        esac
    done

    if [ -n "$SHERPA_ARTIFACT_DIR" ]; then
        if [ -z "$SHERPA_VERSION" ]; then
            print_error "Local artifacts require --version"
            exit 1
        fi
        if [ ! -f "${SHERPA_ARTIFACT_DIR}/sherpad-${TARGET}.tar.gz" ]; then
            print_error "Required sherpad artifact not found in ${SHERPA_ARTIFACT_DIR}"
            exit 1
        fi
    fi

    print_header "Sherpa Update"

    # Pre-flight checks
    check_root_privileges
    check_curl_installed
    check_sherpa_installed
    check_legacy_database

    # Determine target version
    if [ -n "$SHERPA_VERSION" ]; then
        print_info "Using specified version: ${SHERPA_VERSION}"
    else
        get_latest_version
    fi

    # Check if any binary needs updating
    local needs_update=false
    if [ "$FORCE" = true ]; then
        needs_update=true
        print_info "Force mode enabled — all binaries will be updated"
    else
        for binary in sherpad sherpa; do
            if check_binary_needs_update "$binary"; then
                needs_update=true
                break
            fi
        done
    fi

    if [ "$needs_update" = false ]; then
        ensure_symlinks
        secure_lab_storage
        secure_database_storage
        print_success "All binaries already at ${SHERPA_VERSION} — nothing to do"
        exit 0
    fi

    # Perform update
    stop_sherpad
    update_binaries
    ensure_symlinks
    secure_lab_storage
    secure_database_storage
    ensure_runtime_directory
    start_sherpad
    remove_server_ssh_key
    verify_update

    print_success "Sherpa update complete!"
}

# Run main function
main "$@"
