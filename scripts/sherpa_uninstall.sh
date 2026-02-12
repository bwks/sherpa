#!/bin/bash

################################################################################
# Sherpa Uninstall Script - SurrealDB Container Removal
################################################################################
#
# This script removes the SurrealDB container and optionally removes data.
# It must be executed with root/sudo privileges.
#
# Usage:
#   sudo ./sherpa_uninstall.sh [OPTIONS]
#
# Options:
#   --keep-data      Keep database files (default)
#   --remove-data    Remove database files
#   --remove-all     Remove everything including /opt/sherpa
#   --force          Don't prompt for confirmation
#   -h, --help       Show help message
#
################################################################################

set -e  # Exit on error

# Script configuration
CONTAINER_NAME="sherpa-db"
SHERPA_BASE_DIR="/opt/sherpa"
SHERPA_DB_DIR="${SHERPA_BASE_DIR}/db"

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

Remove SurrealDB container and optionally clean up data.

Options:
  --keep-data      Keep database files in ${SHERPA_DB_DIR} (default)
  --remove-data    Remove database files
  --remove-all     Remove everything including ${SHERPA_BASE_DIR}
  --force          Don't prompt for confirmation
  -h, --help       Show this help message

Examples:
  # Remove container only (keep data)
  sudo $0
  
  # Remove container and database files
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
# Data Removal
################################################################################

remove_database_data() {
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

################################################################################
# Summary
################################################################################

print_summary() {
    local actions="$1"
    
    print_header "Uninstall Summary"
    
    cat << EOF
The following will be removed:
  ${actions}

EOF
}

print_completion_message() {
    print_header "Uninstall Complete"
    
    cat << EOF
SurrealDB container has been removed.

EOF

    if [ "$KEEP_DATA" = true ] && [ "$REMOVE_ALL" = false ]; then
        cat << EOF
Database files were preserved:
  Location: ${SHERPA_DB_DIR}
  
To remove data later, run:
  sudo $0 --remove-data

EOF
    fi
    
    if [ "$REMOVE_ALL" = false ]; then
        cat << EOF
Note: The sherpa user and groups were not removed.
      To remove the user manually:
        sudo userdel sherpa

EOF
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
    
    # Print header
    print_header "Sherpa Uninstall - SurrealDB Removal"
    
    # Check prerequisites
    check_root_privileges
    
    # Build summary of actions
    local actions=""
    actions="${actions}\n  - Stop and remove ${CONTAINER_NAME} container"
    
    if [ "$REMOVE_ALL" = true ]; then
        actions="${actions}\n  - Remove all data in ${SHERPA_BASE_DIR}"
    elif [ "$KEEP_DATA" = false ]; then
        actions="${actions}\n  - Remove database data in ${SHERPA_DB_DIR}"
    else
        actions="${actions}\n  - Keep database data (--keep-data)"
    fi
    
    # Show summary and confirm
    echo -e "${actions}"
    echo ""
    
    if [ "$REMOVE_ALL" = true ]; then
        confirm_action "WARNING: This will remove ALL data in ${SHERPA_BASE_DIR}!"
    elif [ "$KEEP_DATA" = false ]; then
        confirm_action "WARNING: This will remove database files in ${SHERPA_DB_DIR}!"
    else
        confirm_action "This will remove the container but keep your data."
    fi
    
    # Perform uninstall
    echo ""
    print_info "Starting uninstall..."
    echo ""
    
    # Handle Docker operations
    if check_docker_available; then
        stop_container
        remove_container
    fi
    
    # Handle data removal
    if [ "$REMOVE_ALL" = true ]; then
        remove_all_data
    elif [ "$KEEP_DATA" = false ]; then
        remove_database_data
    fi
    
    echo ""
    print_completion_message
}

# Run main function
main "$@"
