#!/bin/bash

################################################################################
# Sherpa Install Script
################################################################################
#
# This script installs all required dependencies (QEMU/KVM, libvirt, Docker)
# and sets up SurrealDB as a Docker container for the Sherpa application.
# It must be executed with root/sudo privileges on an Ubuntu system.
#
# Requirements:
#   - Ubuntu (uses apt for package installation)
#   - Root/sudo access
#   - curl installed
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
SURREALDB_VERSION="v3.0.0"
SURREALDB_IMAGE="surrealdb/surrealdb:${SURREALDB_VERSION}"
DB_PORT=8000
DB_USER="sherpa"
DB_NAMESPACE="sherpa"
DB_DATABASE="sherpa"
SHERPA_BASE_DIR="/opt/sherpa"
SHERPA_DB_DIR="${SHERPA_BASE_DIR}/db"
SHERPA_CONFIG_DIR="${SHERPA_BASE_DIR}/config"
MIN_PASSWORD_LENGTH=8

# GitHub release configuration
GITHUB_REPO="bwks/sherpa"
GITHUB_API_URL="https://api.github.com/repos/${GITHUB_REPO}/releases/latest"
GITHUB_DOWNLOAD_URL="https://github.com/${GITHUB_REPO}/releases/download"
TARGET="x86_64-unknown-linux-gnu"
SHERPA_VERSION=""
INSTALLED_VERSION=""

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

Install all Sherpa dependencies and set up SurrealDB container.

Installs QEMU/KVM, libvirt, Docker, and supporting tools, then pulls and
starts a SurrealDB container and installs Sherpa binaries from GitHub releases.

Options:
  --db-pass PASSWORD    Set SurrealDB password
  --version VERSION     Install a specific version (e.g. v0.3.3)
                        If omitted, the latest release is used
  -h, --help           Show this help message

Environment Variables:
  SHERPA_DB_PASSWORD   SurrealDB password (alternative to --db-pass)

Examples:
  # Using command line flag
  sudo $0 --db-pass "MySecurePassword123"

  # Install a specific version
  sudo $0 --db-pass "MySecurePassword123" --version v0.3.3

  # Using environment variable
  export SHERPA_DB_PASSWORD="MySecurePassword123"
  sudo -E $0

Requirements:
  - Ubuntu system (uses apt for package installation)
  - curl must be installed
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
# Dependency Installation
################################################################################

install_system_packages() {
    print_header "Installing System Packages"

    local packages=(
        # Supporting tools
        cpu-checker
        genisoimage
        telnet
        ssh
        mtools
        e2tools
        gzip
        unzip
        # QEMU/KVM/libvirt
        qemu-kvm
        libvirt-daemon-system
        libvirt-clients
        libvirt-dev
        bridge-utils
        virtinst
        libosinfo-bin
        libguestfs-tools
        ovmf
    )

    print_info "Updating package lists..."
    apt-get update -qq

    print_info "Installing packages..."
    DEBIAN_FRONTEND=noninteractive apt-get install -y -qq "${packages[@]}"

    print_info "Enabling and starting libvirtd..."
    systemctl enable libvirtd
    systemctl start libvirtd

    print_success "System packages installed"
}

install_docker() {
    print_header "Installing Docker"

    if check_command_exists docker; then
        print_info "Docker is already installed — skipping"
    else
        print_info "Installing Docker via official convenience script..."
        curl -fsSL https://get.docker.com | sh
        print_success "Docker installed"
    fi

    print_info "Enabling and starting Docker service..."
    systemctl enable docker
    systemctl start docker

    print_success "Docker is running"
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
        start --log info --user "${DB_USER}" --pass "${DB_PASSWORD}" rocksdb:/data/sherpa.db \
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
# Install Binaries
################################################################################

install_binaries() {
    print_header "Installing Sherpa Binaries"

    # Stop sherpad so binaries can be overwritten
    if command -v systemctl >/dev/null 2>&1 && systemctl is-active --quiet sherpad 2>/dev/null; then
        print_info "Stopping sherpad service before updating binaries..."
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

    # Determine version to install
    if [ -n "$SHERPA_VERSION" ]; then
        print_info "Using specified version: ${SHERPA_VERSION}"
    else
        print_info "Fetching latest release version..."
        SHERPA_VERSION=$(curl -sf "${GITHUB_API_URL}" | grep '"tag_name"' | sed -E 's/.*"tag_name": *"([^"]+)".*/\1/')
        if [ -z "$SHERPA_VERSION" ]; then
            print_error "Failed to determine latest release version"
            print_error "Check your internet connection or specify a version with --version"
            exit 1
        fi
        print_success "Latest release: ${SHERPA_VERSION}"
    fi

    INSTALLED_VERSION="${SHERPA_VERSION}"

    # Create a temp directory for downloads, clean up on exit
    local tmp_dir
    tmp_dir=$(mktemp -d)
    trap 'rm -rf "${tmp_dir}"' RETURN

    # Create bin directory
    print_info "Creating binary directory..."
    mkdir -p "${SHERPA_BASE_DIR}/bin"

    # Download and install each binary
    # sherpad is required; sherpa and sherpactl are optional
    local binaries="sherpad sherpa sherpactl"
    local required_binaries="sherpad"

    for binary in $binaries; do
        local asset="${binary}-${TARGET}.tar.gz"
        local url="${GITHUB_DOWNLOAD_URL}/${SHERPA_VERSION}/${asset}"
        local is_required=false

        for req in $required_binaries; do
            if [ "$binary" = "$req" ]; then
                is_required=true
                break
            fi
        done

        print_info "Downloading ${asset}..."
        if curl -sfL -o "${tmp_dir}/${asset}" "${url}"; then
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

            cp "${tmp_dir}/${binary}" "${SHERPA_BASE_DIR}/bin/${binary}"
            chmod 755 "${SHERPA_BASE_DIR}/bin/${binary}"
            chown sherpa:sherpa "${SHERPA_BASE_DIR}/bin/${binary}"
            print_success "Binary installed to ${SHERPA_BASE_DIR}/bin/${binary}"

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

    # Create symlinks in /usr/local/bin for all installed binaries
    print_info "Creating symlinks in /usr/local/bin..."

    if [ -x "${SHERPA_BASE_DIR}/bin/sherpad" ]; then
        ln -sf "${SHERPA_BASE_DIR}/bin/sherpad" /usr/local/bin/sherpad
        print_success "Symlink created: /usr/local/bin/sherpad"
    fi

    if [ -x "${SHERPA_BASE_DIR}/bin/sherpa" ]; then
        ln -sf "${SHERPA_BASE_DIR}/bin/sherpa" /usr/local/bin/sherpa
        print_success "Symlink created: /usr/local/bin/sherpa"
    fi

    if [ -x "${SHERPA_BASE_DIR}/bin/sherpactl" ]; then
        ln -sf "${SHERPA_BASE_DIR}/bin/sherpactl" /usr/local/bin/sherpactl
        print_success "Symlink created: /usr/local/bin/sherpactl"
    fi

    # Verify installations
    print_info "Verifying installations..."
    local verification_failed=0

    if [ ! -x "${SHERPA_BASE_DIR}/bin/sherpad" ]; then
        print_error "sherpad binary verification failed"
        verification_failed=1
    fi

    if [ $verification_failed -eq 1 ]; then
        print_error "Binary installation verification failed"
        exit 1
    fi

    print_success "All binaries installed successfully (${SHERPA_VERSION})"
}

################################################################################
# Install Systemd Service
################################################################################

install_systemd_service() {
    print_header "Installing Systemd Service"
    
    # Check if systemd is available
    if ! command -v systemctl >/dev/null 2>&1; then
        print_warning "systemctl not found - skipping systemd service installation"
        print_warning "You'll need to manage sherpad manually"
        return 0
    fi
    
    # Get the script directory to find systemd files
    SCRIPT_DIR="$( cd "$( dirname "${BASH_SOURCE[0]}" )" && pwd )"
    REPO_ROOT="$(dirname "$SCRIPT_DIR")"
    SERVICE_FILE="${REPO_ROOT}/systemd/sherpad.service"
    ENV_EXAMPLE="${REPO_ROOT}/systemd/sherpad.env.example"
    LOGROTATE_FILE="${REPO_ROOT}/systemd/logrotate.d/sherpad"
    
    # Check if service file exists in repo
    if [ ! -f "$SERVICE_FILE" ]; then
        print_error "Service file not found at: ${SERVICE_FILE}"
        print_warning "Skipping systemd service installation"
        return 1
    fi
    
    # Install systemd service file
    print_info "Installing systemd service file..."
    cp "$SERVICE_FILE" /etc/systemd/system/sherpad.service
    chmod 644 /etc/systemd/system/sherpad.service
    print_success "Service file installed to /etc/systemd/system/sherpad.service"
    
    # Install environment file example
    if [ -f "$ENV_EXAMPLE" ]; then
        print_info "Installing environment file example..."
        cp "$ENV_EXAMPLE" "${SHERPA_CONFIG_DIR}/sherpad.env.example"
        chmod 640 "${SHERPA_CONFIG_DIR}/sherpad.env.example"
        chown sherpa:sherpa "${SHERPA_CONFIG_DIR}/sherpad.env.example"
    fi
    
    # Create actual environment file with database password
    print_info "Creating environment file with database password..."
    cat > "${SHERPA_CONFIG_DIR}/sherpad.env" << EOF
# Sherpad Environment Configuration
# Generated by sherpa_install.sh on $(date)

# Database password
SHERPA_DB_PASSWORD=${DB_PASSWORD}

# Rust logging level (uncomment to enable)
# RUST_LOG=info
EOF
    
    chmod 640 "${SHERPA_CONFIG_DIR}/sherpad.env"
    chown sherpa:sherpa "${SHERPA_CONFIG_DIR}/sherpad.env"
    print_success "Environment file created at ${SHERPA_CONFIG_DIR}/sherpad.env"
    
    # Install logrotate configuration
    if [ -f "$LOGROTATE_FILE" ]; then
        print_info "Installing logrotate configuration..."
        cp "$LOGROTATE_FILE" /etc/logrotate.d/sherpad
        chmod 644 /etc/logrotate.d/sherpad
        print_success "Logrotate config installed to /etc/logrotate.d/sherpad"
    else
        print_warning "Logrotate config not found at: ${LOGROTATE_FILE}"
    fi
    
    # Reload systemd to recognize new service
    print_info "Reloading systemd daemon..."
    systemctl daemon-reload
    print_success "Systemd daemon reloaded"
    
    # Enable service to start on boot (don't start yet — sherpa.toml
    # must be created first via 'sherpactl init')
    print_info "Enabling sherpad service..."
    systemctl enable sherpad.service
    print_success "Service enabled (will start on boot after initialization)"

    print_success "Systemd service installation complete"
}

################################################################################
# Success Message
################################################################################

print_success_message() {
    print_header "Sherpa Installation Complete!"

    cat << EOF
Sherpa Version: ${INSTALLED_VERSION:-unknown}

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
  Database:
    Check status:  docker ps | grep ${CONTAINER_NAME}
    View logs:     docker logs ${CONTAINER_NAME}
    Follow logs:   docker logs -f ${CONTAINER_NAME}
    Stop:          docker stop ${CONTAINER_NAME}
    Start:         docker start ${CONTAINER_NAME}
    Restart:       docker restart ${CONTAINER_NAME}
    
  Sherpad Service:
    Start:         sudo systemctl start sherpad
    Stop:          sudo systemctl stop sherpad
    Restart:       sudo systemctl restart sherpad
    Status:        sudo systemctl status sherpad
    Logs:          tail -f /opt/sherpa/logs/sherpad.log
    Enable:        sudo systemctl enable sherpad
    Disable:       sudo systemctl disable sherpad

Next Steps:
  1. Run 'sudo sherpactl init' to initialize the server environment
  2. Run 'sudo systemctl start sherpad' to start the service

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
            --version)
                SHERPA_VERSION="$2"
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
    print_header "Sherpa Installation"
    
    # Run all checks and setup
    check_root_privileges
    check_curl_installed
    get_database_password

    echo ""
    print_info "Starting installation..."
    echo ""

    install_system_packages
    install_docker
    check_port_available
    setup_sherpa_user
    setup_directories
    stop_existing_container
    pull_surrealdb_image
    start_container
    wait_for_database
    
    # Install binaries and systemd service
    install_binaries
    install_systemd_service
    
    echo ""
    print_success_message
    
    # Remove error trap on success
    trap - EXIT
}

# Run main function
main "$@"
