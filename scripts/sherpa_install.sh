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
SHERPA_ENV_DIR="${SHERPA_BASE_DIR}/env"
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

The script will prompt for the SurrealDB password during installation.
You can also set it via the SHERPA_DB_PASSWORD environment variable.

Options:
  --version VERSION     Install a specific version (e.g. v0.3.4)
                        If omitted, the latest release is used
  -h, --help           Show this help message

Environment Variables:
  SHERPA_DB_PASSWORD   SurrealDB password (skips interactive prompt)
  SHERPA_SERVER_IP4     Server listen IP address (skips interactive prompt)
  SHERPA_SERVER_PORT   Server listen port (default: 3030)
  SHERPA_DB_PORT       SurrealDB port (default: 8000)

Examples:
  # Interactive (will prompt for password)
  sudo $0

  # Install a specific version
  sudo $0 --version v0.3.4

  # Non-interactive via environment variable
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

check_virtualization() {
    print_header "Checking Virtualization Support"

    if grep -q 'vmx' /proc/cpuinfo; then
        print_success "Hardware virtualization supported (Intel VT-x)"
    elif grep -q 'svm' /proc/cpuinfo; then
        print_success "Hardware virtualization supported (AMD-V)"
    else
        print_error "Hardware virtualization extensions not detected"
        echo ""
        echo "Sherpa requires Intel VT-x or AMD-V to run VMs and unikernels via KVM/QEMU."
        echo "Please enable virtualization extensions in your BIOS/UEFI settings and try again."
        exit 1
    fi
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
    DEBIAN_FRONTEND=noninteractive apt-get install -y "${packages[@]}"

    print_success "System packages installed"
}

enable_libvirtd() {
    print_info "Enabling and starting libvirtd..."
    systemctl enable libvirtd
    systemctl start libvirtd
    print_success "libvirtd enabled and started"
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

    # Use environment variable if already set
    if [ -n "$DB_PASSWORD" ]; then
        print_info "Using password from environment"
    else
        # Prompt interactively
        echo ""
        while true; do
            read -r -s -p "Enter SurrealDB password: " DB_PASSWORD
            echo ""
            read -r -s -p "Confirm password: " DB_PASSWORD_CONFIRM
            echo ""

            if [ "$DB_PASSWORD" != "$DB_PASSWORD_CONFIRM" ]; then
                print_error "Passwords do not match, please try again"
                continue
            fi

            if [ -z "$DB_PASSWORD" ]; then
                print_error "Password cannot be empty"
                continue
            fi

            break
        done
    fi

    # Validate password length
    if [ ${#DB_PASSWORD} -lt $MIN_PASSWORD_LENGTH ]; then
        print_error "Password must be at least ${MIN_PASSWORD_LENGTH} characters long"
        exit 1
    fi

    print_success "Password validated"
}

get_server_ip() {
    print_info "Configuring server listen address..."

    # Use environment variable if already set
    if [ -n "$SERVER_IP" ]; then
        print_info "Using server IP from environment: ${SERVER_IP}"
    else
        echo ""
        print_info "The server IP address determines which network interface sherpad listens on."
        print_info "Use 0.0.0.0 to listen on all interfaces (recommended for remote access)."
        echo ""
        read -r -p "Server listen IP address [0.0.0.0]: " SERVER_IP
        SERVER_IP="${SERVER_IP:-0.0.0.0}"
    fi

    # Basic validation: check it looks like an IPv4 address
    if ! echo "$SERVER_IP" | grep -qE '^[0-9]+\.[0-9]+\.[0-9]+\.[0-9]+$'; then
        print_error "Invalid IPv4 address: ${SERVER_IP}"
        exit 1
    fi

    print_success "Server will listen on ${SERVER_IP}"
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

    # Create env directory
    if [ ! -d "${SHERPA_ENV_DIR}" ]; then
        mkdir -p "${SHERPA_ENV_DIR}"
        print_success "Created ${SHERPA_ENV_DIR}"
    else
        print_info "Directory ${SHERPA_ENV_DIR} already exists"
    fi

    # Set ownership and permissions
    chown -R sherpa:sherpa "${SHERPA_BASE_DIR}"
    chmod 775 "${SHERPA_BASE_DIR}"
    chmod 775 "${SHERPA_DB_DIR}"
    chmod 775 "${SHERPA_CONFIG_DIR}"
    chmod 750 "${SHERPA_ENV_DIR}"

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
    
    if docker image pull "${SURREALDB_IMAGE}"; then
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
    # sherpad is required; sherpa is optional
    local binaries="sherpad sherpa"
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

    # Install systemd service file
    print_info "Installing systemd service file..."
    cat > /etc/systemd/system/sherpad.service << 'UNIT'
[Unit]
Description=Sherpa Daemon - VM, Container, and Unikernel Management Server
Documentation=https://github.com/bwks/sherpa
After=network-online.target docker.service libvirtd.service
Wants=network-online.target
Requires=docker.service libvirtd.service
StartLimitBurst=5
StartLimitIntervalSec=60

[Service]
Type=simple

# Run as sherpa user/group
User=sherpa
Group=sherpa
WorkingDirectory=/opt/sherpa

# Capabilities needed for network operations (bridge, raw sockets, etc.)
AmbientCapabilities=CAP_NET_ADMIN CAP_NET_RAW
CapabilityBoundingSet=CAP_NET_ADMIN CAP_NET_RAW

# Run in foreground so systemd manages the process lifecycle
ExecStart=/opt/sherpa/bin/sherpad start --foreground

# Restart policy
Restart=on-failure
RestartSec=5

# Environment configuration (optional file)
EnvironmentFile=-/opt/sherpa/env/sherpa.env

# Security hardening (keep basic options that are compatible with caps)
# NOTE: PrivateTmp is intentionally disabled because the server needs
# to access user-provided file paths (e.g. image imports from /tmp).
NoNewPrivileges=no
PrivateTmp=no

# Resource limits
LimitNOFILE=65536
TasksMax=4096

[Install]
WantedBy=multi-user.target
UNIT
    chmod 644 /etc/systemd/system/sherpad.service
    print_success "Service file installed to /etc/systemd/system/sherpad.service"

    # Install environment file example
    print_info "Installing environment file example..."
    cat > "${SHERPA_ENV_DIR}/sherpa.env.example" << 'ENVEXAMPLE'
# Sherpa Environment Configuration
#
# This is an example file. The actual environment file is at:
#   /opt/sherpa/env/sherpa.env
#
# The environment file is automatically created during installation
# with the database password you provided.

# Database password (required)
SHERPA_DB_PASSWORD=YourSecurePasswordHere

# Server listen IP address (0.0.0.0 for all interfaces)
SHERPA_SERVER_IP4=0.0.0.0

# Server listen port (default: 3030)
# SHERPA_SERVER_PORT=3030

# SurrealDB port (default: 8000)
# SHERPA_DB_PORT=8000

# Libvirt connection URI
LIBVIRT_DEFAULT_URI=qemu:///system

# Rust logging configuration
# Controls the verbosity of sherpad logs written to /opt/sherpa/logs/sherpad.log
#
# Available log levels (from least to most verbose):
#   - error: Only critical errors that prevent operations
#   - warn:  Warnings and errors (non-critical issues)
#   - info:  General operational messages (recommended for production)
#   - debug: Detailed information useful for troubleshooting
#   - trace: Very verbose, includes all internal operations
#
# Default: info (if not set)
# Recommended: info for normal operation, debug for troubleshooting
RUST_LOG=info

# Advanced: Per-module log filtering (optional)
# You can set different log levels for different components to reduce noise
# from dependencies while keeping detailed logs for sherpad itself.
#
# Example: Set sherpad to debug, but reduce dependency verbosity:
# RUST_LOG=sherpad=debug,bollard=warn,surrealdb=warn,virt=warn
#
# Common dependencies to consider filtering:
#   - bollard:    Docker client library
#   - surrealdb:  Database client
#   - virt:       Libvirt client
#   - tower_http: HTTP server middleware

# Custom configuration (add as needed)
# SHERPA_CUSTOM_VAR=value
ENVEXAMPLE
    chmod 640 "${SHERPA_ENV_DIR}/sherpa.env.example"
    chown sherpa:sherpa "${SHERPA_ENV_DIR}/sherpa.env.example"

    # Create actual environment file with database password
    print_info "Creating environment file with database password..."
    cat > "${SHERPA_ENV_DIR}/sherpa.env" << EOF
# Sherpa Environment Configuration
# Generated by sherpa_install.sh on $(date)

# Database password
SHERPA_DB_PASSWORD=${DB_PASSWORD}

# Server listen IP address
SHERPA_SERVER_IP4=${SERVER_IP}

# Server listen port
SHERPA_SERVER_PORT=${SERVER_PORT}

# SurrealDB port
SHERPA_DB_PORT=${DB_PORT}

# Libvirt connection URI
LIBVIRT_DEFAULT_URI=qemu:///system

# Rust logging level (uncomment to enable)
# RUST_LOG=info
EOF

    chmod 640 "${SHERPA_ENV_DIR}/sherpa.env"
    chown sherpa:sherpa "${SHERPA_ENV_DIR}/sherpa.env"
    print_success "Environment file created at ${SHERPA_ENV_DIR}/sherpa.env"

    # Install logrotate configuration
    print_info "Installing logrotate configuration..."
    cat > /etc/logrotate.d/sherpad << 'LOGROTATE'
/opt/sherpa/logs/sherpad.log {
    daily
    rotate 7
    compress
    delaycompress
    missingok
    notifempty
    copytruncate
}
LOGROTATE
    chmod 644 /etc/logrotate.d/sherpad
    print_success "Logrotate config installed to /etc/logrotate.d/sherpad"

    # Reload systemd to recognize new service
    print_info "Reloading systemd daemon..."
    systemctl daemon-reload
    print_success "Systemd daemon reloaded"

    # Enable service to start on boot (don't start yet — sherpa.toml
    # must be created first via 'sherpad init')
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
  1. Run: 'exec sg libvirt -c "newgrp \$(id -gn)"'  -- to load new groups
  2. Run: 'sherpad init'                                               -- to initialize the server environment
  3. Run: 'sudo systemctl start sherpad'                   -- to start the service

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
    SERVER_IP="${SHERPA_SERVER_IP4:-}"
    DB_PORT="${SHERPA_DB_PORT:-${DB_PORT}}"
    SERVER_PORT="${SHERPA_SERVER_PORT:-3030}"

    while [ $# -gt 0 ]; do
        case "$1" in
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
    check_virtualization
    check_curl_installed
    get_database_password
    get_server_ip

    echo ""
    print_info "Starting installation..."
    echo ""

    install_system_packages
    enable_libvirtd
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
