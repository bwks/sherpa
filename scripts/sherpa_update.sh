#!/bin/bash

################################################################################
# Sherpa Update Script
################################################################################
#
# Lightweight update script that downloads the latest Sherpa binaries from
# GitHub releases, stops the service, replaces the binaries, and restarts
# the service. Does NOT re-run full installation (packages, users, etc.).
#
# Servers older than 0.5.0 kept their data in a SurrealDB container. The first
# update to 0.5.0 or later moves that data into sherpad's embedded database
# with 'sherpad migrate-db'. The container and its data are kept, stopped, for
# rollback. If the move fails, the previous sherpad is put back and started.
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
SHERPA_ENV_FILE="${SHERPA_BASE_DIR}/env/sherpa.env"
SHERPA_DB_FILE="${SHERPA_BASE_DIR}/db/sherpa.sqlite"
# The SurrealDB container of releases before 0.5.0, and its default port.
LEGACY_DB_CONTAINER="sherpa-db"
LEGACY_DB_PORT=8000

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
reinstall system packages, users or Docker. A server older than 0.5.0 has its
data moved from the SurrealDB container into the embedded database.

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
        elif curl -sfL -o "${tmp_dir}/${asset}" "${url}"; then
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

            # Keep the running sherpad so a failed database move can put it back.
            if [ "$binary" = "sherpad" ] && [ -x "${SHERPA_BIN_DIR}/sherpad" ]; then
                cp -p "${SHERPA_BIN_DIR}/sherpad" "${SHERPA_BIN_DIR}/sherpad.previous"
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
# Database Migration
################################################################################

# True when the data of a pre-0.5.0 server is still only in its SurrealDB container.
needs_database_migration() {
    [ ! -f "${SHERPA_DB_FILE}" ] || return 1
    command -v docker >/dev/null 2>&1 || return 1
    docker ps -a --format '{{.Names}}' 2>/dev/null | grep -qx "${LEGACY_DB_CONTAINER}"
}

legacy_db_port() {
    local port=""
    if [ -f "${SHERPA_ENV_FILE}" ]; then
        port=$(grep -E '^SHERPA_DB_PORT=' "${SHERPA_ENV_FILE}" | tail -n 1 | cut -d= -f2-)
    fi
    echo "${port:-${LEGACY_DB_PORT}}"
}

wait_for_legacy_database() {
    local port
    port=$(legacy_db_port)
    local attempt=0
    while [ $attempt -lt 30 ]; do
        if curl -sf "http://localhost:${port}/health" >/dev/null 2>&1; then
            return 0
        fi
        attempt=$((attempt + 1))
        sleep 1
    done
    return 1
}

# Put back the sherpad that ran before this update, after a failed database move.
restore_previous_sherpad() {
    if [ -x "${SHERPA_BIN_DIR}/sherpad.previous" ]; then
        cp -p "${SHERPA_BIN_DIR}/sherpad.previous" "${SHERPA_BIN_DIR}/sherpad"
        print_warning "Restored the previous sherpad"
    else
        print_warning "No previous sherpad to restore"
    fi
}

# Remove the database files that a failed 'sherpad migrate-db' created. It runs only when
# there was no database file before, so nothing that existed is removed.
remove_failed_database() {
    rm -f "${SHERPA_DB_FILE}" "${SHERPA_DB_FILE}-wal" "${SHERPA_DB_FILE}-shm"
}

migrate_database() {
    if ! needs_database_migration; then
        return 0
    fi

    print_header "Moving the Database"
    print_info "Found the ${LEGACY_DB_CONTAINER} SurrealDB container of a server older than 0.5.0"

    if ! docker ps --format '{{.Names}}' | grep -qx "${LEGACY_DB_CONTAINER}"; then
        print_info "Starting ${LEGACY_DB_CONTAINER}..."
        docker start "${LEGACY_DB_CONTAINER}" >/dev/null
    fi
    if ! wait_for_legacy_database; then
        print_error "SurrealDB in ${LEGACY_DB_CONTAINER} did not become ready"
        restore_previous_sherpad
        start_sherpad
        exit 1
    fi

    print_info "Importing users, images and labs into ${SHERPA_DB_FILE}..."
    if runuser -u sherpa -- "${SHERPA_BIN_DIR}/sherpad" migrate-db; then
        docker stop "${LEGACY_DB_CONTAINER}" >/dev/null
        docker update --restart=no "${LEGACY_DB_CONTAINER}" >/dev/null
        print_success "Database moved to ${SHERPA_DB_FILE}"
        print_info "${LEGACY_DB_CONTAINER} is stopped and kept, with its data in ${SHERPA_BASE_DIR}/db/sherpa.db,"
        print_info "for rollback. Remove both once the server works: docker rm ${LEGACY_DB_CONTAINER}"
    else
        print_error "Database move failed; the SurrealDB data is unchanged"
        remove_failed_database
        restore_previous_sherpad
        start_sherpad
        exit 1
    fi
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
        if needs_database_migration; then
            stop_sherpad
            migrate_database
            start_sherpad
        fi
        print_success "All binaries already at ${SHERPA_VERSION} — nothing to do"
        exit 0
    fi

    # Perform update
    stop_sherpad
    update_binaries
    ensure_symlinks
    secure_lab_storage
    ensure_runtime_directory
    migrate_database
    start_sherpad
    verify_update

    print_success "Sherpa update complete!"
}

# Run main function
main "$@"
