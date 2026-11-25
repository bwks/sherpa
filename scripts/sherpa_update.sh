#!/bin/bash

################################################################################
# Sherpa Update Script
################################################################################
#
# Lightweight update script that downloads the latest Sherpa binaries from
# GitHub releases, pulls the release's router image, stops the service,
# replaces the binaries, and restarts the service. Does NOT re-run full
# installation (packages, users, etc.).
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
SHERPA_IMAGES_DIR="${SHERPA_BASE_DIR}/images"
# The blank disk templates sherpad copies for node ZTP disks, see ensure_blank_disks.
SHERPA_BLANK_DISK_DIR="${SHERPA_IMAGES_DIR}/blank_disk"
SHERPA_DB_FILE="${SHERPA_BASE_DIR}/db/sherpa.sqlite"
# The database key; a database restored without the key it was sealed with opens nothing.
SHERPA_DB_KEY_FILE="${SHERPA_BASE_DIR}/.secret/db.key"
# Where the update keeps the copy of the database and its key it takes before the new release
# runs. Root's only, outside /opt/sherpa, so no uninstall mode removes it.
SHERPA_UPDATE_BACKUP_ROOT="/var/backups/sherpa"
# How long the new sherpad must stay running, without systemd restarting it, to count as started.
SHERPAD_START_CHECK_SECONDS=10
# The server config and the env files, which can hold secrets, and sherpad's log.
SHERPA_CONFIG_DIR="${SHERPA_BASE_DIR}/config"
SHERPA_ENV_DIR="${SHERPA_BASE_DIR}/env"
SHERPA_LOG_DIR="${SHERPA_BASE_DIR}/logs"
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

# Kata Containers, which runs privileged network-OS containers each in its own VM; as in
# sherpa_install.sh.
KATA_VERSION="4.4.0"
KATA_SHA256="ccbc0cbc591089d0c57012b78da64b23ef43e0bac0f869dc7bfebe8588c33986"
KATA_URL="https://github.com/kata-containers/kata-containers/releases/download/${KATA_VERSION}/kata-static-${KATA_VERSION}-amd64.tar.zst"
KATA_DIR="/opt/kata"
KATA_DEFAULT_CONFIG="${KATA_DIR}/share/defaults/kata-containers/runtime-rs/configuration-clh-runtime-rs.toml"
KATA_SHERPA_CONFIG="/etc/kata-containers/sherpa-clh.toml"
KATA_SHIM="${KATA_DIR}/runtime-rs/bin/containerd-shim-kata-v2"
KATA_DOCKER_RUNTIME="sherpa-kata"
DOCKER_DAEMON_CONFIG="/etc/docker/daemon.json"
SHERPA_VERSION=""
FORCE=false
SHERPA_ARTIFACT_DIR="${SHERPA_ARTIFACT_DIR:-}"
# What the update has done so far, so that finish_update can undo it on a failure.
SHERPAD_STOPPED=false
BINARIES_REPLACED=false
DOWNLOAD_DIR=""
DB_BACKUP_DIR=""
# The binaries downloaded and extracted into DOWNLOAD_DIR, to install.
DOWNLOADED_BINARIES=()
# The installed binaries moved to <name>.previous; finish_update moves only these back.
PREVIOUS_BINARIES=()

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

This script downloads new binaries from GitHub releases and pulls the router
image the release pins, then stops the sherpad service, backs up the database and its key to
${SHERPA_UPDATE_BACKUP_ROOT}, replaces the binaries, and restarts the service.
If the new release does not start, it puts the database, the key and the
binaries back and starts the release that was installed. It does NOT
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

# Succeed when only root can have written the file $1: a regular file of root's, not a link,
# that its group and others cannot write.
binary_is_root_only() {
    local binary="$1"
    [ ! -L "${binary}" ] && [ -f "${binary}" ] && [ "$(stat -c '%u' "${binary}")" = "0" ] &&
        [ $(( 0$(stat -c '%a' "${binary}") & 022 )) -eq 0 ]
}

# Print the version the installed binary $1 reports, when only root can have written it. Any
# other file is never run, as this script runs as root and sherpa or a member of its group may
# have replaced it; nothing is printed for it.
trusted_binary_version() {
    local binary="$1"
    binary_is_root_only "${binary}" || return 0
    "${binary}" --version 2>/dev/null | awk '{print $NF}' || true
}

check_binary_needs_update() {
    local binary="$1"
    local binary_path="${SHERPA_BIN_DIR}/${binary}"

    if [ ! -e "$binary_path" ] && [ ! -L "$binary_path" ]; then
        # Binary not installed — needs update
        return 0
    fi

    local current_version
    current_version="$(trusted_binary_version "${binary_path}")"

    if [ -z "$current_version" ]; then
        print_warning "${binary_path} is not root's alone, or reports no version: it is not run, and will be replaced"
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

    SHERPAD_STOPPED=true
    print_success "sherpad stopped"
}

# Start sherpad, and fail unless it stays running for SHERPAD_START_CHECK_SECONDS without systemd
# restarting it: a release that cannot migrate the database exits at once, and systemd starts it
# again and again.
start_sherpad() {
    print_info "Starting sherpad..."

    if command -v systemctl >/dev/null 2>&1 && systemctl is-enabled --quiet sherpad 2>/dev/null; then
        systemctl restart sherpad
        local restarts second
        restarts="$(systemctl show -p NRestarts --value sherpad 2>/dev/null)" || true
        for (( second = 0; second < SHERPAD_START_CHECK_SECONDS; second++ )); do
            sleep 1
            if ! systemctl is-active --quiet sherpad 2>/dev/null ||
                [ "$(systemctl show -p NRestarts --value sherpad 2>/dev/null)" != "${restarts}" ]; then
                print_error "sherpad service failed to start"
                echo ""
                echo "Check logs with: journalctl -u sherpad --no-pager -n 20"
                return 1
            fi
        done
        SHERPAD_STOPPED=false
        print_success "sherpad service started"
    else
        SHERPAD_STOPPED=false
        print_warning "systemd service not found or not enabled — skipping auto-start"
        print_info "Start sherpad manually if needed"
    fi
}

################################################################################
# Binary Update
################################################################################

# Download and extract each binary that needs the update into DOWNLOAD_DIR, before sherpad is
# stopped: a release that cannot be fetched, or an archive without its binary, leaves the server
# running the release it has.
download_binaries() {
    print_header "Downloading Binaries"

    DOWNLOAD_DIR=$(mktemp -d)

    # sherpad is required; sherpa is optional
    local binaries="sherpad sherpa"
    local required_binaries="sherpad"

    for binary in $binaries; do
        # Check if this binary needs updating
        if [ "$FORCE" = false ] && ! check_binary_needs_update "$binary"; then
            local current_ver
            current_ver="$(trusted_binary_version "${SHERPA_BIN_DIR}/${binary}")"
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
            if cp "${SHERPA_ARTIFACT_DIR}/${asset}" "${DOWNLOAD_DIR}/${asset}"; then
                downloaded=true
            fi
        elif curl -fsSL --connect-timeout 15 --retry 3 --retry-all-errors -o "${DOWNLOAD_DIR}/${asset}" "${url}"; then
            downloaded=true
        fi

        if [ "$downloaded" = true ]; then
            print_success "Downloaded ${asset}"
            extract_binary "${DOWNLOAD_DIR}/${asset}" "${binary}" "${DOWNLOAD_DIR}" || true

            if [ ! -f "${DOWNLOAD_DIR}/${binary}" ]; then
                if [ "$is_required" = true ]; then
                    print_error "Expected binary '${binary}' not found in archive"
                    exit 1
                else
                    print_warning "Expected binary '${binary}' not found in archive — skipping"
                    continue
                fi
            fi
            DOWNLOADED_BINARIES+=("${binary}")
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

    print_success "Downloaded ${SHERPA_VERSION}"
}

# Install the downloaded binaries, with sherpad stopped, root's and 0755 (install_binary). Each
# installed binary is first moved to <name>.previous, which finish_update moves back when the new
# release does not start. mv moves a link itself and keeps the file's owner, so going back puts
# back exactly what was installed.
install_binaries() {
    print_header "Installing Binaries"

    local binary
    for binary in "${DOWNLOADED_BINARIES[@]}"; do
        if [ "$binary" = "sherpad" ]; then
            check_new_sherpad_can_use_database "${DOWNLOAD_DIR}/sherpad"
        fi
    done

    for binary in "${DOWNLOADED_BINARIES[@]}"; do
        print_info "Installing ${binary}..."
        BINARIES_REPLACED=true
        if [ -e "${SHERPA_BIN_DIR}/${binary}" ] || [ -L "${SHERPA_BIN_DIR}/${binary}" ]; then
            mv -f "${SHERPA_BIN_DIR}/${binary}" "${SHERPA_BIN_DIR}/${binary}.previous"
            PREVIOUS_BINARIES+=("${binary}")
        fi
        install_binary "${DOWNLOAD_DIR}/${binary}" "${binary}"
        print_success "Installed ${SHERPA_BIN_DIR}/${binary}"
    done

    print_success "All binaries updated to ${SHERPA_VERSION}"
}

################################################################################
# Database Backup
################################################################################

# Copy the database, its WAL and shared-memory files, and its key, with sherpad stopped, into a
# new folder of root's under SHERPA_UPDATE_BACKUP_ROOT, and say where. The files are read as
# sherpa, so a link in sherpa's folders reaches nothing sherpa cannot read. A server with no
# database has nothing to back up.
backup_database() {
    if [ ! -f "${SHERPA_DB_FILE}" ]; then
        print_info "No database at ${SHERPA_DB_FILE}; nothing to back up"
        return 0
    fi

    print_info "Backing up the database and its key..."
    path_has_no_symlink "${SHERPA_UPDATE_BACKUP_ROOT}" || return 1
    install -d -o root -g root -m 700 "${SHERPA_UPDATE_BACKUP_ROOT}" || return 1
    local dir
    dir="${SHERPA_UPDATE_BACKUP_ROOT}/update-$(date -u +%Y%m%dT%H%M%SZ)"
    mkdir -m 700 "${dir}" || return 1

    local file
    for file in "${SHERPA_DB_FILE}" "${SHERPA_DB_FILE}-wal" "${SHERPA_DB_FILE}-shm" "${SHERPA_DB_KEY_FILE}"; do
        [ -f "${file}" ] || continue
        (umask 077 && runuser -u sherpa -- cat "${file}" > "${dir}/$(basename "${file}")") || {
            print_error "Could not back up ${file} to ${dir}"
            return 1
        }
    done

    DB_BACKUP_DIR="${dir}"
    print_success "Backup of the database and its key: ${dir}"
}

# Put the database and its key back from DB_BACKUP_DIR: the database's WAL and shared-memory files
# the new release left are removed first, as SQLite would apply them to the restored file. The
# files are written as sherpa, mode 0600.
restore_database_backup() {
    [ -n "${DB_BACKUP_DIR}" ] || return 0

    print_info "Restoring the database and its key from ${DB_BACKUP_DIR}..."
    local file
    for file in "${SHERPA_DB_FILE}" "${SHERPA_DB_FILE}-wal" "${SHERPA_DB_FILE}-shm"; do
        runuser -u sherpa -- rm -f "${file}"
    done
    for file in "${SHERPA_DB_FILE}" "${SHERPA_DB_FILE}-wal" "${SHERPA_DB_FILE}-shm" "${SHERPA_DB_KEY_FILE}"; do
        [ -f "${DB_BACKUP_DIR}/$(basename "${file}")" ] || continue
        # shellcheck disable=SC2016 # $1 expands in the shell runuser starts.
        runuser -u sherpa -- sh -c 'umask 077 && cat > "$1"' sh "${file}" < "${DB_BACKUP_DIR}/$(basename "${file}")"
    done
    print_success "Restored the database and its key from ${DB_BACKUP_DIR}"
}

# Run on every exit. After a failure with sherpad stopped, put back what the update replaced,
# the binaries and then the database and its key, and start the release that was installed.
finish_update() {
    local status=$?
    if [ "${status}" -ne 0 ] && [ "${SHERPAD_STOPPED}" = true ]; then
        print_warning "The update failed with sherpad stopped; going back to the release that was installed"
        if [ "${BINARIES_REPLACED}" = true ]; then
            stop_sherpad
            local binary
            for binary in "${PREVIOUS_BINARIES[@]}"; do
                mv -f "${SHERPA_BIN_DIR}/${binary}.previous" "${SHERPA_BIN_DIR}/${binary}"
                if ! binary_is_root_only "${SHERPA_BIN_DIR}/${binary}"; then
                    print_warning "${SHERPA_BIN_DIR}/${binary} is back as it was, not root's alone: the next update replaces it"
                fi
            done
            restore_database_backup || print_error "Restore it by hand from ${DB_BACKUP_DIR}"
        fi
        if start_sherpad; then
            print_info "The release that was installed is running again"
        else
            print_error "The release that was installed did not start either"
        fi
        [ -z "${DB_BACKUP_DIR}" ] || print_info "Backup of the database and its key: ${DB_BACKUP_DIR}"
    fi
    if [ -n "${DOWNLOAD_DIR}" ]; then
        rm -rf "${DOWNLOAD_DIR}"
    fi
    exit "${status}"
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

################################################################################
# Server Folders
################################################################################
# This section is the same in sherpa_install.sh and sherpa_update.sh, between the BEGIN and END
# lines; tests/bats/install_tests.bats runs its cases against each copy.
# BEGIN server folders

# The folders only root may change, base folder first: the base folder, the folder of the
# binaries systemd runs, and the folder above the storage pool. Each is root's, 0755.
root_folders() {
    printf '%s\n' "${SHERPA_BASE_DIR}" "${SHERPA_BASE_DIR}/bin" "${SHERPA_BASE_DIR}/libvirt"
}

# The folders sherpad writes, owned by sherpa, one per line: path, mode, group, and whether a
# folder that is there gets the mode ("set") or keeps its own ("keep"). sherpad cannot make them
# itself, as the base folder is root's.
sherpa_folder_specs() {
    cat << SPECS
${SHERPA_BASE_DIR}/config 750 sherpa set
${SHERPA_BASE_DIR}/env 750 sherpa set
${SHERPA_BASE_DIR}/logs 750 sherpa set
${SHERPA_BASE_DIR}/db 750 sherpa set
${SHERPA_BASE_DIR}/labs 2750 sherpa set
${SHERPA_BASE_DIR}/libvirt/images 2750 kvm set
${SHERPA_BASE_DIR}/.secret 700 sherpa keep
${SHERPA_BASE_DIR}/.certs 700 sherpa keep
${SHERPA_BASE_DIR}/images 775 sherpa keep
${SHERPA_BASE_DIR}/containers 755 sherpa keep
${SHERPA_BASE_DIR}/bins 755 sherpa keep
${SHERPA_BASE_DIR}/run 755 sherpa keep
SPECS
}

# Succeed when no component of the absolute path $1 is a symlink, and say which one is. Root
# changes nothing at such a path: a link planted by sherpa or a member of its group would hand
# the link's target to them.
path_has_no_symlink() {
    local path="$1" prefix="" part
    local -a parts
    IFS='/' read -r -a parts <<< "${path#/}"
    for part in "${parts[@]}"; do
        [ -n "${part}" ] || continue
        prefix="${prefix}/${part}"
        if [ -L "${prefix}" ]; then
            print_error "${prefix} is a symlink: nothing at ${path} is changed"
            return 1
        fi
    done
}

# Make the base folder and the folders under it root's or sherpa's, with their modes, and create
# those that are missing. Only these folders change, never what is in them. The base folder is
# made root's first, so no one else can put a link in it between the check and the change.
secure_server_folders() {
    print_info "Setting the owners and modes of ${SHERPA_BASE_DIR}..."

    local dir mode group when
    while read -r dir; do
        path_has_no_symlink "${dir}" || return 1
        install -d -o root -g root -m 755 "${dir}" || return 1
    done < <(root_folders)

    while read -r dir mode group when; do
        path_has_no_symlink "${dir}" || return 1
        if [ "${group}" = "kvm" ] && ! getent group kvm >/dev/null 2>&1; then
            print_warning "Group kvm not found: ${dir} left readable by sherpa only"
            group="sherpa"
        fi
        if [ ! -e "${dir}" ] || [ "${when}" = "set" ]; then
            install -d -o sherpa -g "${group}" -m "${mode}" "${dir}" || return 1
        fi
    done < <(sherpa_folder_specs)

    print_success "${SHERPA_BASE_DIR} is root's; sherpa owns only the folders sherpad writes"
}

# Give a file or folder sherpa's ownership and the given mode, when it is there. Only its
# metadata changes, never its contents. A symlink, at the path or above it, is left alone:
# following it as root would hand its target to sherpa. The mode is set as sherpa, so a link
# put in its place after the check changes nothing sherpa does not own.
secure_path() {
    local path="$1"
    local mode="$2"
    if [ -L "${path}" ]; then
        print_warning "${path} is a symlink; its owner and mode are left alone"
        return 0
    fi
    path_has_no_symlink "${path}" || return 0
    if [ -e "${path}" ]; then
        chown -h sherpa:sherpa "${path}"
        runuser -u sherpa -- chmod "${mode}" "${path}"
    fi
}

# Install the binary $1 as $2 in the folder of the binaries, root's and 0755. install replaces
# a file or a link at that name, and never writes through a link.
install_binary() {
    install -o root -g root -m 755 "$1" "${SHERPA_BASE_DIR}/bin/$2"
}

# Extract the binary $2 from the release archive $1 into the folder $3, owned by root whatever
# owner the archive records, so no other account can change it before it is installed.
extract_binary() {
    tar -xzf "$1" -C "$3" --no-same-owner --no-same-permissions "$2"
}

# END server folders

################################################################################
# Blank Disks
################################################################################
# This section is the same in sherpa_install.sh and sherpa_update.sh, between the BEGIN and END
# lines; tests/bats/install_tests.bats checks that it is.
# BEGIN blank disks

# The blank disk templates that sherpad copies for node ZTP disks, one per line:
# file name, size, file system and label ("-" for none). The ZTP code depends on the names, file
# systems and labels: fat32.img (Cumulus Linux USB and config disks), junos.img (Junos, label
# vmm-data), iosv.img (IOSv and IOSvL2), ise.img (ISE, label ISE-ZTP) and ext4-500mb.img
# (Flatcar, label data-disk). iosv.img is 131040 sectors of 512 bytes.
blank_disk_specs() {
    cat << 'SPECS'
base.img 64M raw -
fat16.img 64M fat16 -
fat32.img 64M fat32 -
junos.img 64M vfat vmm-data
iosv.img 67092480 vfat -
ise.img 10M ext4 ISE-ZTP
ext4-100mb.img 100M ext4 data-disk
ext4-200mb.img 200M ext4 data-disk
ext4-500mb.img 500M ext4 data-disk
ext4-1g.img 1G ext4 data-disk
ext4-2g.img 2G ext4 data-disk
ext4-3g.img 3G ext4 data-disk
ext4-4g.img 4G ext4 data-disk
ext4-5g.img 5G ext4 data-disk
SPECS
}

# The tools that make the blank disks, and the Ubuntu packages that hold them.
BLANK_DISK_TOOLS=(qemu-img mkfs.fat mkfs.vfat mkfs.ext4)
BLANK_DISK_PACKAGES=(qemu-utils dosfstools e2fsprogs)

# Print the blank disk tools that are not installed, one per line.
missing_blank_disk_tools() {
    local tool
    for tool in "${BLANK_DISK_TOOLS[@]}"; do
        if ! command -v "${tool}" >/dev/null 2>&1; then
            echo "${tool}"
        fi
    done
}

# Run a command as sherpa. The blank disks and their folder are written by sherpa, never by root:
# the folder is open to the sherpa group, whose members could put a link at a disk's name.
as_sherpa() {
    runuser -u sherpa -- "$@"
}

# Write a blank disk of the given size, file system and label to an existing file, as sherpa.
build_blank_disk() {
    local file="$1"
    local size="$2"
    local fs="$3"
    local label="$4"

    as_sherpa qemu-img create -q -f raw "${file}" "${size}" || return 1
    case "${fs}" in
        raw) ;;
        fat16) as_sherpa mkfs.fat -F 16 "${file}" >/dev/null || return 1 ;;
        fat32) as_sherpa mkfs.fat -F 32 "${file}" >/dev/null || return 1 ;;
        vfat)
            if [ "${label}" = "-" ]; then
                as_sherpa mkfs.vfat -I "${file}" >/dev/null || return 1
            else
                as_sherpa mkfs.vfat -n "${label}" "${file}" >/dev/null || return 1
            fi
            ;;
        ext4) as_sherpa mkfs.ext4 -q -L "${label}" "${file}" || return 1 ;;
        *)
            print_error "Unknown blank disk file system: ${fs}"
            return 1
            ;;
    esac
}

# Create one blank disk, 0400 sherpa, as sherpa, when no file of its name exists: only sherpad, as
# sherpa, reads it, to copy it. An existing file is never changed: nodes' disks may have been
# copied from it. The disk is built under a temporary name and then linked to its own name, which
# fails rather than replace a file made meanwhile.
make_blank_disk() {
    local name="$1"
    local size="$2"
    local fs="$3"
    local label="$4"
    local disk="${SHERPA_BLANK_DISK_DIR}/${name}"
    local tmp

    if [ -e "${disk}" ] || [ -L "${disk}" ]; then
        print_info "Blank disk ${name} exists; left as it is"
        return 0
    fi

    tmp="$(as_sherpa mktemp "${SHERPA_BLANK_DISK_DIR}/.${name}.XXXXXX")" || return 1
    if ! build_blank_disk "${tmp}" "${size}" "${fs}" "${label}" || ! as_sherpa chmod 400 "${tmp}"; then
        as_sherpa rm -f "${tmp}"
        print_error "Could not create blank disk ${name}"
        return 1
    fi
    if as_sherpa ln "${tmp}" "${disk}" 2>/dev/null; then
        print_success "Created blank disk ${name}"
    else
        print_info "Blank disk ${name} exists; left as it is"
    fi
    as_sherpa rm -f "${tmp}"
}

# Create the blank disk folder and every blank disk that is missing. Existing folders and disks
# keep their owner, mode and contents.
ensure_blank_disks() {
    print_info "Creating missing blank disks in ${SHERPA_BLANK_DISK_DIR}..."

    local missing
    missing="$(missing_blank_disk_tools)"
    if [ -n "${missing}" ]; then
        print_error "Blank disks need these tools, which are not installed: $(echo "${missing}" | tr '\n' ' ')"
        echo "  Install them with: sudo apt-get install ${BLANK_DISK_PACKAGES[*]}"
        return 1
    fi

    # The images folder is in the base folder, which is root's; the blank disk folder is made in it
    # as sherpa.
    if [ ! -d "${SHERPA_IMAGES_DIR}" ]; then
        path_has_no_symlink "${SHERPA_IMAGES_DIR}" || return 1
        install -d -o sherpa -g sherpa -m 775 "${SHERPA_IMAGES_DIR}" || return 1
        print_success "Created ${SHERPA_IMAGES_DIR}"
    fi
    if [ ! -d "${SHERPA_BLANK_DISK_DIR}" ]; then
        as_sherpa mkdir -m 775 "${SHERPA_BLANK_DISK_DIR}" || return 1
        print_success "Created ${SHERPA_BLANK_DISK_DIR}"
    fi

    local name size fs label
    while read -r name size fs label; do
        make_blank_disk "${name}" "${size}" "${fs}" "${label}" || return 1
    done < <(blank_disk_specs)

    print_success "Blank disks ready"
}

# END blank disks

# Install the packages of the blank disk tools when one is missing: servers installed by a
# release before the installer made the blank disks may lack them.
install_blank_disk_tools() {
    if [ -z "$(missing_blank_disk_tools)" ]; then
        return 0
    fi
    print_info "Installing the blank disk tools: ${BLANK_DISK_PACKAGES[*]}"
    apt-get update -qq
    DEBIAN_FRONTEND=noninteractive apt-get install -y -qq "${BLANK_DISK_PACKAGES[@]}"
}

# Restrict the server config, the env files and sherpad's log. sherpa.toml and the env files can
# hold secrets, so only sherpa may read them; releases before this left sherpa.toml at the umask
# of whoever ran 'sherpad init', often 0664, and the env files 0640. Their folders are set by
# secure_server_folders.
secure_config_files() {
    local log
    secure_path "${SHERPA_CONFIG_DIR}/sherpa.toml" 600
    secure_path "${SHERPA_ENV_DIR}/sherpa.env" 600
    secure_path "${SHERPA_ENV_DIR}/sherpa.env.example" 600
    # The log and the copies logrotate made of it, which kept its old mode.
    for log in "${SHERPA_LOG_DIR}"/sherpad.log*; do
        # An unmatched pattern stays as it is, naming no file.
        [ -e "${log}" ] || [ -L "${log}" ] || continue
        secure_path "${log}" 640
    done
    print_success "Server config, env files and logs restricted"
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
    if ! path_has_no_symlink "${SERVER_SSH_KEY_DIR}"; then
        print_warning "The server's shared SSH key is left in place"
        return 0
    fi
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
    new_version="$(trusted_binary_version "${SHERPA_BIN_DIR}/sherpad")"

    if [ -n "$new_version" ]; then
        print_success "Updated: sherpad ${new_version}"
    else
        print_success "Binaries replaced with ${SHERPA_VERSION}"
    fi
}

################################################################################
# Router Image
################################################################################
# This section is the same in sherpa_install.sh and sherpa_update.sh, between the BEGIN and END
# lines; tests/bats/update_tests.bats checks that it is.
# BEGIN router image

# The lab router image, pulled by its digest and tagged with the name sherpad runs it by.
SHERPA_ROUTER_REPO="ghcr.io/bwks/sherpa-router"
SHERPA_ROUTER_DIGEST="sha256:c42a07c425de65e315ff8b92143d8a95c183677ffca859124c9476957bff992f"
SHERPA_ROUTER_IMAGE="${SHERPA_ROUTER_REPO}:latest"

# Pull the router image by its digest, so the image is the one this release was made with, and
# tag it with the name sherpad runs it by.
# Install Kata Containers with Cloud Hypervisor, and register it with Docker as the runtime
# Sherpa runs privileged network-OS containers in, each in its own VM.
install_kata() {
    print_header "Installing Kata Containers ${KATA_VERSION}"

    if [[ "$(cat "${KATA_DIR}/VERSION" 2>/dev/null)" == "${KATA_VERSION}" ]]; then
        print_info "Kata Containers ${KATA_VERSION} is already installed — skipping"
    else
        local archive
        archive="$(mktemp)"
        print_info "Downloading Kata Containers ${KATA_VERSION}..."
        curl -fsSL -o "${archive}" "${KATA_URL}"
        if ! echo "${KATA_SHA256}  ${archive}" | sha256sum -c --status; then
            rm -f "${archive}"
            print_error "The Kata Containers archive does not match its pinned SHA-256"
            exit 1
        fi
        rm -rf "${KATA_DIR}"
        tar --zstd -xf "${archive}" -C /
        rm -f "${archive}"
        print_success "Kata Containers installed in ${KATA_DIR}"
    fi

    # virtiofsd passes extended attributes, so file capabilities in an image survive.
    mkdir -p "$(dirname "${KATA_SHERPA_CONFIG}")"
    sed -e 's/^virtio_fs_extra_args = \["-o", "announce_submounts"\]/virtio_fs_extra_args = ["-o", "announce_submounts", "--xattr"]/' \
        -e 's/^default_vcpus = 1$/default_vcpus = 2/' \
        "${KATA_DEFAULT_CONFIG}" > "${KATA_SHERPA_CONFIG}"
    if ! grep -q '"--xattr"' "${KATA_SHERPA_CONFIG}"; then
        print_error "Could not enable extended attributes in ${KATA_SHERPA_CONFIG}"
        exit 1
    fi
    chmod 644 "${KATA_SHERPA_CONFIG}"

    local changed
    changed="$(python3 -I - "${DOCKER_DAEMON_CONFIG}" "${KATA_DOCKER_RUNTIME}" "${KATA_SHIM}" "${KATA_SHERPA_CONFIG}" <<'PY'
import json, os, sys
path, name, shim, config = sys.argv[1:]
daemon = {}
if os.path.exists(path):
    with open(path) as f:
        daemon = json.load(f)
runtime = {"runtimeType": shim, "options": {"TypeUrl": "io.containerd.kata.v2.options", "ConfigPath": config}}
if daemon.get("runtimes", {}).get(name) == runtime:
    print("no")
else:
    daemon.setdefault("runtimes", {})[name] = runtime
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w") as f:
        json.dump(daemon, f, indent=2)
        f.write("\n")
    print("yes")
PY
)"
    if [[ "${changed}" == "yes" ]]; then
        # A reload reads the runtimes again and leaves running containers as they are.
        print_info "Registered the ${KATA_DOCKER_RUNTIME} runtime in ${DOCKER_DAEMON_CONFIG}; reloading Docker..."
        systemctl reload docker
    fi
    if ! docker info --format '{{json .Runtimes}}' | grep -q "\"${KATA_DOCKER_RUNTIME}\""; then
        print_error "Docker does not list the ${KATA_DOCKER_RUNTIME} runtime"
        exit 1
    fi
    print_success "Docker runs privileged network-OS containers in Kata VMs"
}

pull_sherpa_router_image() {
    print_info "Pulling Sherpa Router image..."

    local pinned="${SHERPA_ROUTER_REPO}@${SHERPA_ROUTER_DIGEST}"
    if docker image pull "${pinned}" && docker image tag "${pinned}" "${SHERPA_ROUTER_IMAGE}"; then
        print_success "Sherpa Router image pulled: ${pinned}, tagged ${SHERPA_ROUTER_IMAGE}"
    else
        print_error "Failed to pull Sherpa Router image ${pinned}"
        exit 1
    fi
}

# END router image

################################################################################
# Main Script
################################################################################

main() {
    # Parse command line arguments
    while [ $# -gt 0 ]; do
        case "$1" in
            --version)
                if [ $# -lt 2 ] || [ -z "$2" ] || [[ "$2" == --* ]]; then
                    print_error "--version requires a version"
                    show_usage
                    exit 1
                fi
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

    # The base folder and the folders under it get their owners first, before any binary is
    # looked at, so no one but root can change what is in the base folder from here on.
    secure_server_folders

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
        install_blank_disk_tools
        ensure_blank_disks
        pull_sherpa_router_image
        install_kata
        ensure_symlinks
        secure_config_files
        print_info "Router image: ${SHERPA_ROUTER_REPO}@${SHERPA_ROUTER_DIGEST}"
        print_success "All binaries already at ${SHERPA_VERSION} — nothing to do"
        exit 0
    fi

    # Fill in missing blank disks, refresh the router image and install Kata before sherpad stops:
    # a failure here leaves the server as it was.
    install_blank_disk_tools
    ensure_blank_disks
    pull_sherpa_router_image
    install_kata

    # From here, a failure puts back what was replaced and starts the release that was installed.
    trap finish_update EXIT

    # The release is fetched while the server still runs; then the server stops for the backup and
    # the install.
    download_binaries
    stop_sherpad
    backup_database
    install_binaries
    ensure_symlinks
    secure_config_files
    ensure_runtime_directory
    start_sherpad
    rm -f "${SHERPA_BIN_DIR}/sherpad.previous" "${SHERPA_BIN_DIR}/sherpa.previous"
    remove_server_ssh_key
    verify_update

    [ -z "${DB_BACKUP_DIR}" ] || print_info "Backup of the database and its key: ${DB_BACKUP_DIR}"
    print_info "Router image: ${SHERPA_ROUTER_REPO}@${SHERPA_ROUTER_DIGEST}"
    print_success "Sherpa update complete!"
}

# Run main function
main "$@"
