#!/bin/bash

################################################################################
# Sherpa Uninstall Script
################################################################################
#
# This script removes the sherpad service and binary, the SurrealDB container
# left by releases before 0.5.0 if there is one, and optionally the data.
# It must be executed with root/sudo privileges. It refuses, before it changes
# anything, while a lab runs.
#
# Usage:
#   sudo ./sherpa_uninstall.sh [OPTIONS]
#
# Options:
#   --keep-data      Keep the database and its backups (default)
#   --remove-data    Remove the database and its backups
#   --remove-all     Remove everything including /opt/sherpa, /run/sherpa, the
#                    update backups in /var/backups/sherpa and the sherpa account
#   --force          Don't prompt for confirmation
#   -h, --help       Show help message
#
################################################################################

set -e  # Exit on error

# Script configuration
# The SurrealDB container of releases before 0.5.0.
CONTAINER_NAME="sherpa-db"
SHERPA_BASE_DIR="/opt/sherpa"
SHERPA_DB_DIR="${SHERPA_BASE_DIR}/db"
SHERPA_LABS_DIR="${SHERPA_BASE_DIR}/labs"
SHERPA_RUN_DIR="/run/sherpa"
SHERPA_UPDATE_BACKUP_DIR="/var/backups/sherpa"
SHERPA_ACCOUNT="sherpa"
THP_TMPFILES_CONF="/etc/tmpfiles.d/sherpa-thp.conf"
# What sherpad itself uses to find a lab's resources: the libvirt connection, the labels of a
# Tailscale gateway, the storage pool and the gateway's state volume.
LIBVIRT_URI="qemu:///system"
GATEWAY_ROLE_LABEL="sh.erpa.role"
GATEWAY_ROLE="tailnet-gateway"
GATEWAY_LAB_LABEL="sh.erpa.tailscale.lab"
STORAGE_POOL="sherpa-pool"
TAILNET_VOLUME_PREFIX="sherpa-tailnet-state-"
# Accounts below this UID are system accounts, as the install script creates `sherpa`.
SYSTEM_UID_LIMIT=1000

# Color codes for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Default options
KEEP_DATA=true
REMOVE_ALL=false
FORCE=false
# The labs' resources on the host, read once before anything is removed.
LAB_LEFTOVERS=""

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

Remove the sherpad service and binary, the SurrealDB container of a release
before 0.5.0 if there is one, and optionally the data.

Options:
  --keep-data      Keep the database and its backups in ${SHERPA_DB_DIR} (default)
  --remove-data    Remove the database and its backups
  --remove-all     Remove everything including ${SHERPA_BASE_DIR}, ${SHERPA_RUN_DIR}, the update
                   backups in ${SHERPA_UPDATE_BACKUP_DIR} and the ${SHERPA_ACCOUNT} account
  --force          Don't prompt for confirmation
  -h, --help       Show this help message

While a lab runs (a VM, a container or a Tailscale gateway), nothing is changed,
in any mode and with --force too: destroy the labs first, or stop them with
\`sherpa down\` when the data is kept.

Examples:
  # Remove the service (keep data)
  sudo $0

  # Remove the service and the database
  sudo $0 --remove-data
  
  # Remove everything without confirmation
  sudo $0 --remove-all --force

EOF
}

confirm_action() {
    local message="$1"
    
    if [ "$FORCE" = true ]; then
        return 0
    fi
    
    echo ""
    echo -e "${YELLOW}${message}${NC}"
    read -p "Continue? (yes/no): " -r
    echo ""
    
    if [[ ! $REPLY =~ ^[Yy][Ee][Ss]$ ]]; then
        print_info "Uninstall cancelled by user"
        exit 0
    fi
}

################################################################################
# Pre-flight Checks
################################################################################

check_root_privileges() {
    if [ "$EUID" -ne 0 ]; then
        print_error "This script must be run as root or with sudo"
        echo "Please run: sudo $0"
        exit 1
    fi
}

check_docker_available() {
    if ! command -v docker >/dev/null 2>&1; then
        print_warning "Docker is not installed or not in PATH"
        print_info "Skipping container removal"
        return 1
    fi
    return 0
}

################################################################################
# Labs
################################################################################

# Print the ID of each lab sherpad has a folder for in SHERPA_LABS_DIR.
lab_ids() {
    local dir name
    [ -d "${SHERPA_LABS_DIR}" ] || return 0
    for dir in "${SHERPA_LABS_DIR}"/*/; do
        [ -d "${dir}" ] || continue
        name="$(basename "${dir}")"
        if [[ "${name}" =~ ^[A-Za-z0-9-]{8}$ ]]; then
            echo "${name}"
        fi
    done
}

# Succeed when $1 is the name sherpad gives a resource of lab $2: a non-empty name, a hyphen
# and the lab ID.
is_lab_resource() {
    local name="$1" lab_id="$2" prefix
    prefix="${name%-"${lab_id}"}"
    [ "${prefix}" != "${name}" ] && [ -n "${prefix}" ]
}

# Print "<lab ID><tab><kind><tab><name>" for each name on stdin that is a resource of a lab
# whose ID is an argument after the kind $1.
match_lab_resources() {
    local kind="$1" name lab_id
    shift
    while IFS= read -r name; do
        [ -n "${name}" ] || continue
        for lab_id in "$@"; do
            if is_lab_resource "${name}" "${lab_id}"; then
                printf '%s\t%s\t%s\n' "${lab_id}" "${kind}" "${name}"
            fi
        done
    done
}

# Print the running VMs, containers and Tailscale gateways of Sherpa's labs, one
# "<lab ID><tab><kind><tab><name>" line each. Fails, naming the runtime, when one that is
# installed cannot be asked while labs exist.
running_lab_resources() {
    local -a ids
    local names rows name role lab_id failed=0
    mapfile -t ids < <(lab_ids)

    if [ "${#ids[@]}" -gt 0 ] && command -v virsh >/dev/null 2>&1; then
        if names="$(virsh -c "${LIBVIRT_URI}" list --name 2>&1)"; then
            match_lab_resources "VM" "${ids[@]}" <<< "${names}"
        else
            print_error "Cannot list the running libvirt domains: ${names}"
            failed=1
        fi
    fi

    if command -v docker >/dev/null 2>&1; then
        if rows="$(docker ps --format "{{.Names}}\t{{.Label \"${GATEWAY_ROLE_LABEL}\"}}\t{{.Label \"${GATEWAY_LAB_LABEL}\"}}" 2>&1)"; then
            while IFS=$'\t' read -r name role lab_id; do
                [ -n "${name}" ] || continue
                if [ "${role}" = "${GATEWAY_ROLE}" ]; then
                    printf '%s\t%s\t%s\n' "${lab_id:-unknown}" "Tailscale gateway" "${name}"
                elif [ "${#ids[@]}" -gt 0 ]; then
                    match_lab_resources "container" "${ids[@]}" <<< "${name}"
                fi
            done <<< "${rows}"
        elif [ "${#ids[@]}" -gt 0 ]; then
            print_error "Cannot list the running Docker containers: ${rows}"
            failed=1
        else
            print_warning "Cannot list the running Docker containers: ${rows}"
        fi
    fi

    return "${failed}"
}

# Exit 1, before anything is changed, when a lab runs, naming each lab and its nodes.
refuse_running_labs() {
    local resources lab_id kind name previous=""
    if ! resources="$(running_lab_resources)"; then
        print_error "Nothing was changed: the script cannot tell whether a lab is running."
        echo "Start Docker and libvirt, or destroy the labs in ${SHERPA_LABS_DIR}, then run it again." >&2
        exit 1
    fi
    [ -n "${resources}" ] || return 0

    print_error "Labs are running. Nothing was changed."
    while IFS=$'\t' read -r lab_id kind name; do
        if [ "${lab_id}" != "${previous}" ]; then
            echo "  Lab ${lab_id}:" >&2
            previous="${lab_id}"
        fi
        if [ "${kind}" = "Tailscale gateway" ]; then
            echo "    ${kind} ${name}" >&2
        else
            echo "    ${kind} ${name%-"${lab_id}"} (${name})" >&2
        fi
    done < <(sort <<< "${resources}")
    if [ "$REMOVE_ALL" = true ]; then
        echo "Destroy each lab with \`sherpa destroy\` in its lab folder, then run this script again." >&2
    else
        echo "Destroy each lab with \`sherpa destroy\`, or stop it with \`sherpa down\`, in its lab folder, then run this script again." >&2
    fi
    exit 1
}

# Print, one per line, the VMs, containers, networks and volumes of the labs that are left on
# the host, and the storage pool.
lab_leftovers() {
    local -a ids
    local names name lab_id
    mapfile -t ids < <(lab_ids)

    if command -v virsh >/dev/null 2>&1; then
        if [ "${#ids[@]}" -gt 0 ]; then
            if names="$(virsh -c "${LIBVIRT_URI}" list --all --name 2>/dev/null)"; then
                match_lab_resources "libvirt domain" "${ids[@]}" <<< "${names}"
            else
                printf '%s\t%s\t%s\n' "-" "libvirt domains" "(cannot be listed)"
            fi
            if names="$(virsh -c "${LIBVIRT_URI}" net-list --all --name 2>/dev/null)"; then
                match_lab_resources "libvirt network" "${ids[@]}" <<< "${names}"
            else
                printf '%s\t%s\t%s\n' "-" "libvirt networks" "(cannot be listed)"
            fi
        fi
        if virsh -c "${LIBVIRT_URI}" pool-info "${STORAGE_POOL}" >/dev/null 2>&1; then
            printf '%s\t%s\t%s\n' "-" "libvirt storage pool" "${STORAGE_POOL}"
        fi
    fi

    if [ "${#ids[@]}" -gt 0 ] && command -v docker >/dev/null 2>&1; then
        if names="$(docker ps -a --format '{{.Names}}' 2>/dev/null)"; then
            match_lab_resources "Docker container" "${ids[@]}" <<< "${names}"
        else
            printf '%s\t%s\t%s\n' "-" "Docker containers" "(cannot be listed)"
        fi
        if names="$(docker network ls --format '{{.Name}}' 2>/dev/null)"; then
            match_lab_resources "Docker network" "${ids[@]}" <<< "${names}"
        else
            printf '%s\t%s\t%s\n' "-" "Docker networks" "(cannot be listed)"
        fi
        if names="$(docker volume ls --format '{{.Name}}' 2>/dev/null)"; then
            while IFS= read -r name; do
                for lab_id in "${ids[@]}"; do
                    if [ "${name}" = "${TAILNET_VOLUME_PREFIX}${lab_id}" ]; then
                        printf '%s\t%s\t%s\n' "${lab_id}" "Docker volume" "${name}"
                    fi
                done
            done <<< "${names}"
        else
            printf '%s\t%s\t%s\n' "-" "Docker volumes" "(cannot be listed)"
        fi
    fi
}

################################################################################
# Container Removal
################################################################################

stop_container() {
    print_info "Checking for ${CONTAINER_NAME} container..."
    
    if ! docker ps -a --format '{{.Names}}' | grep -q "^${CONTAINER_NAME}$"; then
        print_info "Container ${CONTAINER_NAME} not found"
        return 0
    fi
    
    # Stop if running
    if docker ps --format '{{.Names}}' | grep -q "^${CONTAINER_NAME}$"; then
        print_info "Stopping container..."
        if docker stop "${CONTAINER_NAME}" >/dev/null 2>&1; then
            print_success "Container stopped"
        else
            print_warning "Failed to stop container (it may already be stopped)"
        fi
    else
        print_info "Container is not running"
    fi
}

remove_container() {
    print_info "Removing container..."
    
    if docker ps -a --format '{{.Names}}' | grep -q "^${CONTAINER_NAME}$"; then
        if docker rm "${CONTAINER_NAME}" >/dev/null 2>&1; then
            print_success "Container removed"
        else
            print_error "Failed to remove container"
            return 1
        fi
    else
        print_info "Container already removed"
    fi
}

################################################################################
# Systemd Service Removal
################################################################################

remove_systemd_service() {
    print_header "Removing Systemd Service"
    
    # Check if systemctl is available
    if ! command -v systemctl >/dev/null 2>&1; then
        print_info "systemctl not found - skipping systemd service removal"
        return 0
    fi
    
    # Check if service exists
    if ! systemctl list-unit-files | grep -q "^sherpad.service"; then
        print_info "Systemd service not found"
        return 0
    fi
    
    # Stop the service if running
    print_info "Stopping sherpad service..."
    if systemctl is-active --quiet sherpad.service; then
        if systemctl stop sherpad.service 2>/dev/null; then
            print_success "Service stopped"
        else
            print_warning "Failed to stop service (may already be stopped)"
        fi
    else
        print_info "Service is not running"
    fi
    
    # Disable the service
    print_info "Disabling sherpad service..."
    if systemctl is-enabled --quiet sherpad.service 2>/dev/null; then
        if systemctl disable sherpad.service 2>/dev/null; then
            print_success "Service disabled"
        else
            print_warning "Failed to disable service"
        fi
    else
        print_info "Service is not enabled"
    fi
    
    # Remove service file
    print_info "Removing service file..."
    if [ -f /etc/systemd/system/sherpad.service ]; then
        if rm -f /etc/systemd/system/sherpad.service; then
            print_success "Service file removed"
        else
            print_error "Failed to remove service file"
            return 1
        fi
    fi
    if [ -d /etc/systemd/system/sherpad.service.d ]; then
        if rm -rf /etc/systemd/system/sherpad.service.d; then
            print_success "Service drop-ins removed"
        else
            print_warning "Failed to remove service drop-ins"
        fi
    fi
    
    # Remove logrotate config
    print_info "Removing logrotate configuration..."
    if [ -f /etc/logrotate.d/sherpad ]; then
        if rm -f /etc/logrotate.d/sherpad; then
            print_success "Logrotate config removed"
        else
            print_warning "Failed to remove logrotate config"
        fi
    fi
    
    # Reload systemd
    print_info "Reloading systemd daemon..."
    systemctl daemon-reload 2>/dev/null || true
    systemctl reset-failed 2>/dev/null || true
    print_success "Systemd daemon reloaded"
    
    print_success "Systemd service removal complete"
}

# Succeed when no component of the absolute path $1 is a symlink, and say which one is. Nothing
# is removed through a link: rm would delete what is at the link's target.
path_has_no_symlink() {
    local path="$1" prefix="" part
    local -a parts
    IFS='/' read -r -a parts <<< "${path#/}"
    for part in "${parts[@]}"; do
        [ -n "${part}" ] || continue
        prefix="${prefix}/${part}"
        if [ -L "${prefix}" ]; then
            print_error "${prefix} is a symlink: nothing at ${path} is removed"
            return 1
        fi
    done
}

remove_sherpad_binary() {
    print_info "Removing sherpad binary..."

    if ! path_has_no_symlink "${SHERPA_BASE_DIR}/bin"; then
        return 0
    fi
    if [ -f "${SHERPA_BASE_DIR}/bin/sherpad" ]; then
        if rm -f "${SHERPA_BASE_DIR}/bin/sherpad"; then
            print_success "Binary removed"
        else
            print_warning "Failed to remove binary"
        fi
    else
        print_info "Binary not found"
    fi

    # The copy sherpa_update.sh keeps while it installs, for going back when the new release does
    # not start.
    rm -f "${SHERPA_BASE_DIR}/bin/sherpad.previous"

    remove_binary_symlink /usr/local/bin/sherpad "${SHERPA_BASE_DIR}/bin/sherpad"
    if [ "$REMOVE_ALL" = true ]; then
        remove_binary_symlink /usr/local/bin/sherpa "${SHERPA_BASE_DIR}/bin/sherpa"
    fi
}

remove_binary_symlink() {
    local link="$1"
    local target="$2"

    # Preserve files and links belonging to other installations, including when
    # our target binary has already been removed and the link is dangling.
    if [ -L "$link" ] && [ "$(readlink "$link")" = "$target" ]; then
        rm -f "$link"
        print_success "Removed binary symlink: ${link}"
    fi
}

# The installer's boot rule for transparent hugepages. The running setting is
# left as it is; the kernel's own default applies from the next boot.
remove_thp_boot_rule() {
    if [ -f "${THP_TMPFILES_CONF}" ]; then
        if rm -f "${THP_TMPFILES_CONF}"; then
            print_success "Removed transparent hugepages boot rule: ${THP_TMPFILES_CONF}"
        else
            print_warning "Failed to remove ${THP_TMPFILES_CONF}"
        fi
    else
        print_info "Transparent hugepages boot rule not found"
    fi
}

################################################################################
# Data Removal
################################################################################

remove_database_data() {
    if ! path_has_no_symlink "${SHERPA_DB_DIR}"; then
        return 1
    fi
    if [ ! -d "${SHERPA_DB_DIR}" ]; then
        print_info "Database directory ${SHERPA_DB_DIR} not found"
        return 0
    fi
    
    print_info "Removing database data from ${SHERPA_DB_DIR}..."
    
    # Check if directory has files
    if [ -n "$(ls -A "${SHERPA_DB_DIR}" 2>/dev/null)" ]; then
        if rm -rf "${SHERPA_DB_DIR:?}"/*; then
            print_success "Database data removed"
        else
            print_error "Failed to remove database data"
            return 1
        fi
    else
        print_info "Database directory is empty"
    fi
}

remove_all_data() {
    if [ ! -d "${SHERPA_BASE_DIR}" ]; then
        print_info "Directory ${SHERPA_BASE_DIR} not found"
        return 0
    fi
    
    print_info "Removing all data from ${SHERPA_BASE_DIR}..."
    
    # Safety check: make sure we're not trying to remove /
    if [ "${SHERPA_BASE_DIR}" = "/" ] || [ -z "${SHERPA_BASE_DIR}" ]; then
        print_error "Safety check failed: refusing to remove root directory"
        exit 1
    fi
    
    if rm -rf "${SHERPA_BASE_DIR:?}"; then
        print_success "All data removed"
    else
        print_error "Failed to remove ${SHERPA_BASE_DIR}"
        return 1
    fi
}

# Remove the RAM-backed folder of container secret files that sherpad's unit keeps.
remove_run_dir() {
    if ! path_has_no_symlink "${SHERPA_RUN_DIR}"; then
        return 1
    fi
    if [ ! -d "${SHERPA_RUN_DIR}" ]; then
        print_info "${SHERPA_RUN_DIR} not found"
        return 0
    fi
    if rm -rf "${SHERPA_RUN_DIR:?}"; then
        print_success "Removed ${SHERPA_RUN_DIR}"
    else
        print_error "Failed to remove ${SHERPA_RUN_DIR}"
        return 1
    fi
}

# Remove the update script's backups of the database and its key.
remove_update_backups() {
    if ! path_has_no_symlink "${SHERPA_UPDATE_BACKUP_DIR}"; then
        return 1
    fi
    if [ ! -d "${SHERPA_UPDATE_BACKUP_DIR}" ]; then
        print_info "${SHERPA_UPDATE_BACKUP_DIR} not found"
        return 0
    fi
    if rm -rf "${SHERPA_UPDATE_BACKUP_DIR:?}"; then
        print_success "Removed ${SHERPA_UPDATE_BACKUP_DIR}"
    else
        print_error "Failed to remove ${SHERPA_UPDATE_BACKUP_DIR}"
        return 1
    fi
}

# Print the accounts whose primary group is the sherpa group, other than the sherpa account.
group_primary_users() {
    local gid
    gid="$(getent group "${SHERPA_ACCOUNT}" | cut -d: -f3)"
    [ -n "${gid}" ] || return 0
    getent passwd | awk -F: -v gid="${gid}" -v account="${SHERPA_ACCOUNT}" \
        '$4 == gid && $1 != account { print $1 }'
}

# Succeed when the sherpa account exists and is a system account, which --remove-all removes.
sherpa_account_removable() {
    local uid
    uid="$(id -u "${SHERPA_ACCOUNT}" 2>/dev/null)" || return 1
    [ "${uid}" -lt "${SYSTEM_UID_LIMIT}" ]
}

# Remove the sherpa account, which drops it from every group, and its group when no other
# account has it as its primary group.
remove_sherpa_account() {
    local users
    if ! id -u "${SHERPA_ACCOUNT}" >/dev/null 2>&1; then
        print_info "Account ${SHERPA_ACCOUNT} not found"
    elif ! sherpa_account_removable; then
        print_warning "Account ${SHERPA_ACCOUNT} is not a system account (UID ${SYSTEM_UID_LIMIT} or more): it is kept"
    elif userdel "${SHERPA_ACCOUNT}"; then
        print_success "Removed the ${SHERPA_ACCOUNT} account and its group memberships"
    else
        print_error "Failed to remove the ${SHERPA_ACCOUNT} account"
        return 1
    fi

    getent group "${SHERPA_ACCOUNT}" >/dev/null 2>&1 || return 0
    id -u "${SHERPA_ACCOUNT}" >/dev/null 2>&1 && return 0
    users="$(group_primary_users | paste -sd, -)"
    if [ -n "${users}" ]; then
        print_info "Group ${SHERPA_ACCOUNT} is kept: it is the primary group of ${users}"
    elif groupdel "${SHERPA_ACCOUNT}"; then
        print_success "Removed the ${SHERPA_ACCOUNT} group"
    else
        print_warning "Failed to remove the ${SHERPA_ACCOUNT} group"
    fi
}

################################################################################
# Summary
################################################################################

# Print what the mode removes.
print_removed() {
    echo "Removed, where present:"
    echo "  - The sherpad service, its log rotation rule and the transparent hugepages boot rule"
    echo "  - The sherpad binary and its link in /usr/local/bin"
    echo "  - The ${CONTAINER_NAME} container of a release before 0.5.0, if present"
    if [ "$REMOVE_ALL" = true ]; then
        echo "  - Everything in ${SHERPA_BASE_DIR}: the database and its backups, the database key,"
        echo "    the configuration, the certificates, the lab folders, the images and the VM disks"
        echo "  - The sherpa CLI's link in /usr/local/bin"
        echo "  - ${SHERPA_RUN_DIR}, the container secret files"
        echo "  - The update script's backups of the database and its key: ${SHERPA_UPDATE_BACKUP_DIR}"
        echo "  - The ${SHERPA_ACCOUNT} account, which leaves the docker, libvirt and kvm groups, and the"
        echo "    ${SHERPA_ACCOUNT} group unless it is another account's primary group"
    elif [ "$KEEP_DATA" = false ]; then
        echo "  - The database and its backups in ${SHERPA_DB_DIR}"
    fi
}

# Print every path, account and resource the mode leaves in place, and where it is.
print_kept() {
    local lab_id kind name
    echo "Kept:"
    if [ "$REMOVE_ALL" = false ]; then
        if [ "$KEEP_DATA" = true ]; then
            echo "  - The database and its backups: ${SHERPA_DB_DIR}"
        fi
        echo "  - The login token secret and the database key: ${SHERPA_BASE_DIR}/.secret"
        echo "  - The server's certificate and key: ${SHERPA_BASE_DIR}/.certs"
        echo "  - The server's settings: ${SHERPA_BASE_DIR}/config/sherpa.toml"
        echo "  - The service's environment: ${SHERPA_BASE_DIR}/env/sherpa.env"
        echo "  - The lab folders: ${SHERPA_LABS_DIR}"
        echo "  - The node images and the VM disks: ${SHERPA_BASE_DIR}/images, ${SHERPA_BASE_DIR}/libvirt/images"
        echo "  - The rest of ${SHERPA_BASE_DIR}: the logs, the sherpa CLI and its link /usr/local/bin/sherpa"
        echo "  - The container secret files, until the next reboot: ${SHERPA_RUN_DIR}"
        echo "  - The ${SHERPA_ACCOUNT} account and group, which own the kept files; ${SHERPA_ACCOUNT} stays in the"
        echo "    docker, libvirt and kvm groups (--remove-all removes it)"
    elif id -u "${SHERPA_ACCOUNT}" >/dev/null 2>&1 && ! sherpa_account_removable; then
        echo "  - The ${SHERPA_ACCOUNT} account, which is not a system account"
    fi
    if [ "$REMOVE_ALL" = true ] && getent group "${SHERPA_ACCOUNT}" >/dev/null 2>&1 \
        && [ -n "$(group_primary_users)" ]; then
        echo "  - The ${SHERPA_ACCOUNT} group, the primary group of $(group_primary_users | paste -sd, -)"
    fi
    if [ "$REMOVE_ALL" = false ] && [ -d "${SHERPA_UPDATE_BACKUP_DIR}" ]; then
        echo "  - The update script's backups of the database and its key: ${SHERPA_UPDATE_BACKUP_DIR}"
    fi
    echo "  - The packages the install script installed, Docker and libvirt among them"

    if [ -n "${LAB_LEFTOVERS}" ]; then
        if [ "$REMOVE_ALL" = true ]; then
            echo "  - These lab resources, left pointing at the removed disks and folders"
            echo "    (destroy the labs before --remove-all to remove them):"
        else
            echo "  - These lab resources:"
        fi
        while IFS=$'\t' read -r lab_id kind name; do
            if [ "${lab_id}" = "-" ]; then
                echo "      ${kind} ${name}"
            else
                echo "      ${kind} ${name} (lab ${lab_id})"
            fi
        done <<< "${LAB_LEFTOVERS}"
    fi
}

# Print what the uninstall will remove and keep, before the confirmation prompt.
print_plan() {
    print_header "Uninstall Plan"
    print_removed | sed 's/^Removed, where present:/To be removed, where present:/'
    echo ""
    print_kept | sed 's/^Kept:/To be kept:/'
    echo ""
}

# Print the closing summary: what was removed and what was kept, and where.
print_completion_message() {
    print_header "Uninstall Complete"
    print_removed
    echo ""
    print_kept
    echo ""
    if [ "$REMOVE_ALL" = false ]; then
        echo "To remove the kept data, the update backups and the ${SHERPA_ACCOUNT} account later, run:"
        echo "  sudo $0 --remove-all"
        echo ""
    fi
}

################################################################################
# Main Script
################################################################################

main() {
    # Parse command line arguments
    while [ $# -gt 0 ]; do
        case "$1" in
            --keep-data)
                KEEP_DATA=true
                shift
                ;;
            --remove-data)
                KEEP_DATA=false
                shift
                ;;
            --remove-all)
                REMOVE_ALL=true
                KEEP_DATA=false
                shift
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
    
    print_header "Sherpa Uninstall"
    check_root_privileges
    refuse_running_labs

    LAB_LEFTOVERS="$(lab_leftovers)"
    print_plan
    if [ "$REMOVE_ALL" = true ]; then
        confirm_action "WARNING: This will remove ALL data in ${SHERPA_BASE_DIR}, the update backups in ${SHERPA_UPDATE_BACKUP_DIR} and the ${SHERPA_ACCOUNT} account!"
    elif [ "$KEEP_DATA" = false ]; then
        confirm_action "WARNING: This will remove the database and its backups in ${SHERPA_DB_DIR}!"
    else
        confirm_action "This will remove the service but keep your data."
    fi

    echo ""
    print_info "Starting uninstall..."
    echo ""

    remove_systemd_service
    remove_sherpad_binary
    remove_thp_boot_rule
    if check_docker_available; then
        stop_container
        remove_container
    fi

    if [ "$REMOVE_ALL" = true ]; then
        remove_all_data
        remove_run_dir
        remove_update_backups
        remove_sherpa_account
    elif [ "$KEEP_DATA" = false ]; then
        remove_database_data
    fi

    echo ""
    print_completion_message
}

# Run main function
main "$@"
