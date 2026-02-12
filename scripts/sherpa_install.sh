#!/bin/bash

################################################################################
# Sherpa Install Script - SurrealDB Container Setup
################################################################################
#
# This script sets up SurrealDB as a Docker container for the Sherpa application.
# It must be executed with root/sudo privileges.
#
# Requirements:
#   - Docker installed and running
#   - Root/sudo access
#   - Port 8000 available
#
# Usage:
#   sudo ./sherpa_install.sh --db-pass "YourPassword"
#   # OR
#   export SHERPA_DB_PASSWORD="YourPassword"
#   sudo -E ./sherpa_install.sh
#
################################################################################

set -e  # Exit on error

# Script configuration
CONTAINER_NAME="sherpa-db"
SURREALDB_VERSION="v2.4"
SURREALDB_IMAGE="surrealdb/surrealdb:${SURREALDB_VERSION}"
DB_PORT=8000
DB_USER="sherpa"
DB_NAMESPACE="sherpa"
DB_DATABASE="sherpa"
SHERPA_BASE_DIR="/opt/sherpa"
SHERPA_DB_DIR="${SHERPA_BASE_DIR}/db"
SHERPA_CONFIG_DIR="${SHERPA_BASE_DIR}/config"
MIN_PASSWORD_LENGTH=8

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

Setup SurrealDB container for Sherpa application.

Options:
  --db-pass PASSWORD    Set SurrealDB password
  -h, --help           Show this help message

Environment Variables:
  SHERPA_DB_PASSWORD   SurrealDB password (alternative to --db-pass)

Examples:
  # Using command line flag
  sudo $0 --db-pass "MySecurePassword123"
  
  # Using environment variable
  export SHERPA_DB_PASSWORD="MySecurePassword123"
  sudo -E $0

Requirements:
  - Docker must be installed and running
  - Script must be run as root/sudo
  - Port ${DB_PORT} must be available
  - Password must be at least ${MIN_PASSWORD_LENGTH} characters

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
    
    # Capture the actual user who ran sudo
    if [ -n "$SUDO_USER" ]; then
        ACTUAL_USER="$SUDO_USER"
        print_success "Running as root (user: ${ACTUAL_USER})"
    else
        ACTUAL_USER="root"
        print_success "Running as root"
    fi
}

check_docker_installed() {
    print_info "Checking Docker installation..."
    
    if ! check_command_exists docker; then
        print_error "Docker is not installed"
        echo ""
        echo "Please install Docker first:"
        echo "  https://docs.docker.com/engine/install/"
        echo ""
        exit 1
    fi
    
    print_success "Docker is installed"
}

check_docker_running() {
    print_info "Checking Docker daemon status..."
    
    if ! docker info >/dev/null 2>&1; then
        print_error "Docker daemon is not running"
        echo ""
        echo "Please start Docker:"
        echo "  sudo systemctl start docker"
        echo ""
        exit 1
    fi
    
    print_success "Docker daemon is running"
}

check_port_available() {
    print_info "Checking if port ${DB_PORT} is available..."
    
    # Check if port is in use (works on most Linux systems)
    if command -v ss >/dev/null 2>&1; then
        if ss -tuln | grep -q ":${DB_PORT} "; then
            print_error "Port ${DB_PORT} is already in use"
            echo ""
            echo "Process using port ${DB_PORT}:"
            ss -tulnp | grep ":${DB_PORT} " || true
            echo ""
            echo "Please stop the conflicting service or change SHERPA_DB_PORT"
            exit 1
        fi
    elif command -v netstat >/dev/null 2>&1; then
        if netstat -tuln | grep -q ":${DB_PORT} "; then
            print_error "Port ${DB_PORT} is already in use"
            echo ""
            echo "Process using port ${DB_PORT}:"
            netstat -tulnp | grep ":${DB_PORT} " || true
            echo ""
            echo "Please stop the conflicting service or change SHERPA_DB_PORT"
            exit 1
        fi
    else
        print_warning "Cannot verify port availability (ss/netstat not found)"
    fi
    
    print_success "Port ${DB_PORT} is available"
}

################################################################################
# Password Handling
################################################################################

get_database_password() {
    print_info "Validating database password..."
    
    # Priority: command line flag > environment variable
    if [ -z "$DB_PASSWORD" ]; then
        print_error "Database password not provided"
        echo ""
        echo "You must provide a password using one of these methods:"
        echo "  1. Command line: --db-pass \"YourPassword\""
        echo "  2. Environment:  export SHERPA_DB_PASSWORD=\"YourPassword\""
        echo ""
        show_usage
        exit 1
    fi
    
    # Validate password length
    if [ ${#DB_PASSWORD} -lt $MIN_PASSWORD_LENGTH ]; then
        print_error "Password must be at least ${MIN_PASSWORD_LENGTH} characters long"
        exit 1
    fi
    
    print_success "Password validated"
}

################################################################################
# User & Group Setup
################################################################################

setup_sherpa_user() {
    print_info "Setting up sherpa user..."
    
    # Create sherpa user if it doesn't exist
    if ! id -u sherpa >/dev/null 2>&1; then
        useradd -r -m -s /bin/bash -d /opt/sherpa \
                -c "Sherpa service user" sherpa
        print_success "Created sherpa user"
    else
        print_info "Sherpa user already exists"
    fi
    
    # Add sherpa user to required groups
    local groups_added=0
    
    if getent group libvirt >/dev/null 2>&1; then
        usermod -aG libvirt sherpa 2>/dev/null || true
        groups_added=1
    fi
    
    if getent group kvm >/dev/null 2>&1; then
        usermod -aG kvm sherpa 2>/dev/null || true
        groups_added=1
    fi
    
    if getent group docker >/dev/null 2>&1; then
        usermod -aG docker sherpa 2>/dev/null || true
        groups_added=1
    fi
    
    if [ $groups_added -eq 1 ]; then
        print_success "Added sherpa to required groups (libvirt, kvm, docker)"
    fi
    
    # Add current user to sherpa group
    if [ -n "$ACTUAL_USER" ] && [ "$ACTUAL_USER" != "root" ]; then
        if ! id -nG "$ACTUAL_USER" | grep -qw sherpa; then
            usermod -aG sherpa "$ACTUAL_USER"
            print_success "Added ${ACTUAL_USER} to sherpa group"
            print_warning "User ${ACTUAL_USER} must log out and back in for group changes to take effect"
        else
            print_info "User ${ACTUAL_USER} is already in sherpa group"
        fi
    fi
}

################################################################################
# Directory Setup
################################################################################

setup_directories() {
    print_info "Creating directory structure..."
    
    # Create base directory
    if [ ! -d "${SHERPA_BASE_DIR}" ]; then
        mkdir -p "${SHERPA_BASE_DIR}"
        print_success "Created ${SHERPA_BASE_DIR}"
    else
        print_info "Directory ${SHERPA_BASE_DIR} already exists"
    fi
    
    # Create database directory
    if [ ! -d "${SHERPA_DB_DIR}" ]; then
        mkdir -p "${SHERPA_DB_DIR}"
        print_success "Created ${SHERPA_DB_DIR}"
    else
        print_info "Directory ${SHERPA_DB_DIR} already exists"
    fi
    
    # Create config directory
    if [ ! -d "${SHERPA_CONFIG_DIR}" ]; then
        mkdir -p "${SHERPA_CONFIG_DIR}"
        print_success "Created ${SHERPA_CONFIG_DIR}"
    else
        print_info "Directory ${SHERPA_CONFIG_DIR} already exists"
    fi
    
    # Set ownership and permissions
    chown -R sherpa:sherpa "${SHERPA_BASE_DIR}"
    chmod 775 "${SHERPA_BASE_DIR}"
    chmod 775 "${SHERPA_DB_DIR}"
    chmod 775 "${SHERPA_CONFIG_DIR}"
    
    print_success "Directory permissions configured"
}

################################################################################
# Container Management
################################################################################

stop_existing_container() {
    print_info "Checking for existing container..."
    
    if docker ps -a --format '{{.Names}}' | grep -q "^${CONTAINER_NAME}$"; then
        print_warning "Found existing ${CONTAINER_NAME} container"
        
        # Stop if running
        if docker ps --format '{{.Names}}' | grep -q "^${CONTAINER_NAME}$"; then
            print_info "Stopping container..."
            docker stop "${CONTAINER_NAME}" >/dev/null
        fi
        
        # Remove container
        print_info "Removing container..."
        docker rm "${CONTAINER_NAME}" >/dev/null
        print_success "Removed old container"
    else
        print_info "No existing container found"
    fi
}

pull_surrealdb_image() {
    print_info "Pulling SurrealDB image (${SURREALDB_VERSION})..."
    
    if docker pull "${SURREALDB_IMAGE}"; then
        print_success "Image pulled successfully"
    else
        print_error "Failed to pull SurrealDB image"
        exit 1
    fi
}

start_container() {
    print_info "Starting SurrealDB container..."
    
    # Get sherpa user UID/GID
    SHERPA_UID=$(id -u sherpa)
    SHERPA_GID=$(id -g sherpa)
    
    # Start container
    if docker run -d \
        --name "${CONTAINER_NAME}" \
        --restart unless-stopped \
        -p ${DB_PORT}:8000 \
        -v "${SHERPA_DB_DIR}:/data" \
        --user "${SHERPA_UID}:${SHERPA_GID}" \
        "${SURREALDB_IMAGE}" \
        start --log info --user "${DB_USER}" --pass "${DB_PASSWORD}" file:/data/sherpa.db \
        >/dev/null; then
        print_success "Container started successfully"
    else
        print_error "Failed to start container"
        echo ""
        echo "Container logs:"
        docker logs "${CONTAINER_NAME}" 2>&1 || true
        exit 1
    fi
}

################################################################################
# Health Check
################################################################################

wait_for_database() {
    print_info "Waiting for database to be ready..."
    
    local max_attempts=30
    local attempt=0
    
    while [ $attempt -lt $max_attempts ]; do
        # Check if container is still running
        if ! docker ps --format '{{.Names}}' | grep -q "^${CONTAINER_NAME}$"; then
            print_error "Container stopped unexpectedly"
            echo ""
            echo "Container logs:"
            docker logs "${CONTAINER_NAME}" 2>&1 || true
            exit 1
        fi
        
        # Check health endpoint
        if curl -sf http://localhost:${DB_PORT}/health >/dev/null 2>&1; then
            echo ""
            print_success "Database is healthy and ready"
            return 0
        fi
        
        attempt=$((attempt + 1))
        printf "."
        sleep 1
    done
    
    echo ""
    print_error "Database failed to become ready within ${max_attempts} seconds"
    echo ""
    echo "Container status:"
    docker ps -a | grep "${CONTAINER_NAME}" || true
    echo ""
    echo "Container logs:"
    docker logs "${CONTAINER_NAME}" 2>&1 || true
    exit 1
}

################################################################################
# Success Message
################################################################################

print_success_message() {
    print_header "SurrealDB Installation Complete!"
    
    cat << EOF
Database Details:
  Container Name: ${CONTAINER_NAME}
  Status:         Running
  Port:           ${DB_PORT}
  Data Location:  ${SHERPA_DB_DIR}/sherpa.db
  Restart Policy: unless-stopped (auto-start on boot)

Connection Info:
  Host:      localhost:${DB_PORT}
  User:      ${DB_USER}
  Namespace: ${DB_NAMESPACE}
  Database:  ${DB_DATABASE}

Useful Commands:
  Check status:  docker ps | grep ${CONTAINER_NAME}
  View logs:     docker logs ${CONTAINER_NAME}
  Follow logs:   docker logs -f ${CONTAINER_NAME}
  Stop:          docker stop ${CONTAINER_NAME}
  Start:         docker start ${CONTAINER_NAME}
  Restart:       docker restart ${CONTAINER_NAME}

Next Steps:
  1. Export SHERPA_DB_PASSWORD or use --db-pass flag when running sherpa
  2. Run 'sherpa init' to initialize the application

EOF

    if [ -n "$ACTUAL_USER" ] && [ "$ACTUAL_USER" != "root" ]; then
        cat << EOF
Important Note:
  User '${ACTUAL_USER}' has been added to the 'sherpa' group.
  You must log out and back in for group changes to take effect.

EOF
    fi
}

################################################################################
# Cleanup on Error
################################################################################

cleanup_on_error() {
    local exit_code=$?
    
    if [ $exit_code -ne 0 ]; then
        print_error "Installation failed (exit code: ${exit_code})"
        
        # Stop and remove container if it exists
        if docker ps -a --format '{{.Names}}' | grep -q "^${CONTAINER_NAME}$" 2>/dev/null; then
            print_info "Cleaning up container..."
            docker stop "${CONTAINER_NAME}" 2>/dev/null || true
            docker rm "${CONTAINER_NAME}" 2>/dev/null || true
            print_info "Container removed"
        fi
        
        echo ""
        print_info "Note: Directories and user accounts were not removed"
        print_info "      Run sherpa_uninstall.sh to complete cleanup"
    fi
}

################################################################################
# Main Script
################################################################################

main() {
    # Parse command line arguments
    DB_PASSWORD="${SHERPA_DB_PASSWORD:-}"
    
    while [ $# -gt 0 ]; do
        case "$1" in
            --db-pass)
                DB_PASSWORD="$2"
                shift 2
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
    
    # Set up error trap
    trap cleanup_on_error EXIT
    
    # Print header
    print_header "Sherpa Installation - SurrealDB Setup"
    
    # Run all checks and setup
    check_root_privileges
    check_docker_installed
    check_docker_running
    check_port_available
    get_database_password
    
    echo ""
    print_info "Starting installation..."
    echo ""
    
    setup_sherpa_user
    setup_directories
    stop_existing_container
    pull_surrealdb_image
    start_container
    wait_for_database
    
    echo ""
    print_success_message
    
    # Remove error trap on success
    trap - EXIT
}

# Run main function
main "$@"
