#!/bin/bash

################################################################################
# Sherpa Install Script
################################################################################
#
# This script installs all required dependencies (QEMU/KVM, libvirt, Docker)
# and the Sherpa binaries. sherpad keeps its database in a file it manages
# itself, so no database server is installed.
# It must be executed with root/sudo privileges on an Ubuntu system.
#
# Requirements:
#   - Ubuntu (uses apt for package installation)
#   - Root/sudo access
#   - curl installed
#
# Usage:
#   sudo ./sherpa_install.sh
#
#   # Or install a specific version:
#   sudo ./sherpa_install.sh --version v0.3.4
#
################################################################################

set -e  # Exit on error

# Script configuration
# The SurrealDB container of releases before 0.5.0. sherpa_update.sh moves its data.
LEGACY_DB_CONTAINER="sherpa-db"
SHERPA_ROUTER_IMAGE="ghcr.io/bwks/sherpa-router:latest"
SHERPA_BASE_DIR="/opt/sherpa"
SHERPA_DB_DIR="${SHERPA_BASE_DIR}/db"
SHERPA_DB_FILE="${SHERPA_DB_DIR}/sherpa.sqlite"
SHERPA_CONFIG_DIR="${SHERPA_BASE_DIR}/config"
SHERPA_ENV_DIR="${SHERPA_BASE_DIR}/env"
SHERPA_LABS_DIR="${SHERPA_BASE_DIR}/labs"
SHERPA_POOL_DIR="${SHERPA_BASE_DIR}/libvirt/images"
THP_ENABLED_FILE="/sys/kernel/mm/transparent_hugepage/enabled"
THP_TMPFILES_CONF="/etc/tmpfiles.d/sherpa-thp.conf"

# GitHub release configuration
GITHUB_REPO="bwks/sherpa"
GITHUB_API_URL="https://api.github.com/repos/${GITHUB_REPO}/releases/latest"
GITHUB_DOWNLOAD_URL="https://github.com/${GITHUB_REPO}/releases/download"
TARGET="x86_64-unknown-linux-gnu"
SHERPA_VERSION=""
INSTALLED_VERSION=""
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

Install all Sherpa dependencies and the Sherpa binaries.

Installs QEMU/KVM, libvirt, Docker, and supporting tools, then installs the
Sherpa binaries from GitHub releases. sherpad keeps its database in
${SHERPA_DB_FILE}; no database server is installed.

To upgrade a server older than 0.5.0, use sherpa_update.sh instead: it moves
the data out of the old SurrealDB container.

Options:
  --version VERSION     Install a specific version (e.g. v0.3.4)
                        If omitted, the latest release is used
  -h, --help            Show this help message

Environment Variables:
  SHERPA_SERVER_IPV4   Server listen IP address
  SHERPA_SERVER_WS_PORT   Server listen port (default: 3030)
  SHERPA_SERVER_HTTP_PORT     HTTP certificate endpoint port (default: 3031)
  SHERPA_ARTIFACT_DIR  Local release archives instead of GitHub downloads
                      Requires --version; expects the normal release filenames

Examples:
  # Install the latest release
  sudo $0

  # Install a specific version
  sudo $0 --version v0.3.4

Requirements:
  - Ubuntu system (uses apt for package installation)
  - curl must be installed
  - Script must be run as root/sudo

EOF
}

check_command_exists() {
    command -v "$1" >/dev/null 2>&1
}

################################################################################
# Pre-flight Checks
################################################################################

check_ubuntu_version() {
    print_info "Checking Ubuntu version..."

    if [ ! -f /etc/os-release ]; then
        print_error "Cannot determine OS: /etc/os-release not found"
        exit 1
    fi

    . /etc/os-release

    if [ "$ID" != "ubuntu" ]; then
        print_error "This script requires Ubuntu (detected: ${ID:-unknown})"
        exit 1
    fi

    # Extract major and minor version (e.g. "24.04" -> 2404)
    local version_num
    version_num=$(echo "$VERSION_ID" | tr -d '.')

    if [ "$version_num" -lt 2404 ] 2>/dev/null; then
        print_error "Ubuntu 24.04 or later is required (detected: ${VERSION_ID})"
        exit 1
    fi

    print_success "Ubuntu ${VERSION_ID} detected"
}

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
        # qemu-kvm is ambiguous on Ubuntu 26.04 (standard and HWE providers).
        qemu-system-x86
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

# QEMU asks for transparent hugepages for guest RAM, which the kernel only grants
# when THP is madvise or always. Set madvise now and with a tmpfiles rule on boot,
# unless THP is already always.
configure_transparent_hugepages() {
    print_header "Configuring Transparent Hugepages"

    if [ ! -f "${THP_ENABLED_FILE}" ]; then
        print_warning "Transparent hugepages are not available in this kernel — skipping"
        return 0
    fi

    if grep -q '\[always\]' "${THP_ENABLED_FILE}"; then
        print_info "Transparent hugepages are already always — leaving them as they are"
        return 0
    fi

    mkdir -p "$(dirname "${THP_TMPFILES_CONF}")"
    cat > "${THP_TMPFILES_CONF}" << EOF
# Installed by Sherpa: let QEMU back guest RAM with transparent hugepages.
w ${THP_ENABLED_FILE} - - - - madvise
EOF
    echo madvise > "${THP_ENABLED_FILE}"

    print_success "Transparent hugepages set to madvise (boot rule: ${THP_TMPFILES_CONF})"
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
# Upgrade Check
################################################################################

check_legacy_database() {
    print_info "Checking for a server older than 0.5.0..."

    if command -v docker >/dev/null 2>&1 && \
        docker ps -a --format '{{.Names}}' 2>/dev/null | grep -qx "${LEGACY_DB_CONTAINER}"; then
        print_error "Found the ${LEGACY_DB_CONTAINER} SurrealDB container of a Sherpa server older than 0.5.0"
        echo ""
        echo "Its users, images and labs must be moved into the embedded database, and"
        echo "reinstalling would replace the settings that move needs. Upgrade with"
        echo "sherpa_update.sh instead, which moves the data and keeps the container"
        echo "for rollback."
        echo ""
        exit 1
    fi

    print_success "No SurrealDB container from an older release"
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

    # Trim whitespace
    SERVER_IP="$(echo "$SERVER_IP" | xargs)"

    # Validate IPv4 address format
    if ! echo "$SERVER_IP" | grep -qE '^[0-9]+\.[0-9]+\.[0-9]+\.[0-9]+$'; then
        print_error "Invalid IPv4 address: ${SERVER_IP}"
        exit 1
    fi

    # Validate each octet is 0-255
    local IFS='.'
    read -r o1 o2 o3 o4 <<< "$SERVER_IP"
    if [ "$o1" -gt 255 ] || [ "$o2" -gt 255 ] || [ "$o3" -gt 255 ] || [ "$o4" -gt 255 ] 2>/dev/null; then
        print_error "Invalid IPv4 address: ${SERVER_IP} (each octet must be 0-255)"
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
        useradd -r -M -s /usr/sbin/nologin \
                -c "Sherpa service user" sherpa
        print_success "Created sherpa user"
    else
        print_info "Sherpa user already exists"
        # Ensure system user has nologin shell (may differ if user was created manually)
        local current_shell
        current_shell=$(getent passwd sherpa | cut -d: -f7)
        if [ "$current_shell" != "/usr/sbin/nologin" ] && [ "$(id -u sherpa)" -lt 1000 ]; then
            usermod -s /usr/sbin/nologin sherpa
            print_info "Updated sherpa shell to /usr/sbin/nologin (was: ${current_shell})"
        fi
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

    # Lab data (node configs, user data, ZTP media, VM disks) is readable only by sherpa, its
    # group and QEMU, which runs in the kvm group. sherpad enforces the same modes at startup.
    install -d -o sherpa -g sherpa -m 2750 "${SHERPA_LABS_DIR}"
    if getent group kvm >/dev/null 2>&1; then
        install -d -o sherpa -g kvm -m 2750 "${SHERPA_POOL_DIR}"
    else
        print_warning "Group kvm not found: ${SHERPA_POOL_DIR} left readable by sherpa only"
        install -d -o sherpa -g sherpa -m 2750 "${SHERPA_POOL_DIR}"
    fi

    print_success "Directory permissions configured"
}

################################################################################
# Container Images
################################################################################

pull_sherpa_router_image() {
    print_info "Pulling Sherpa Router image..."

    if docker image pull "${SHERPA_ROUTER_IMAGE}"; then
        print_success "Sherpa Router image pulled successfully"
    else
        print_error "Failed to pull Sherpa Router image"
        exit 1
    fi
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
# CAP_BPF + CAP_PERFMON required for eBPF P2p link redirect programs
AmbientCapabilities=CAP_NET_ADMIN CAP_NET_RAW CAP_BPF CAP_PERFMON
CapabilityBoundingSet=CAP_NET_ADMIN CAP_NET_RAW CAP_BPF CAP_PERFMON

# Run in foreground so systemd manages the process lifecycle
ExecStart=/opt/sherpa/bin/sherpad start --foreground

# Restart policy
Restart=on-failure
RestartSec=5

# Environment configuration (optional file)
EnvironmentFile=-/opt/sherpa/env/sherpa.env

# RAM-backed /run/sherpa (0700 sherpa) for container secret files. Kept when sherpad
# stops or restarts, so running containers keep their secrets; gone after a reboot.
RuntimeDirectory=sherpa
RuntimeDirectoryMode=0700
RuntimeDirectoryPreserve=yes

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
# The environment file is automatically created during installation.

# Server listen IP address (0.0.0.0 for all interfaces)
SHERPA_SERVER_IPV4=0.0.0.0

# Server listen port (default: 3030)
# SHERPA_SERVER_WS_PORT=3030

# HTTP certificate endpoint port (default: 3031)
# SHERPA_SERVER_HTTP_PORT=3031

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
# RUST_LOG=sherpad=debug,bollard=warn,turso_core=warn,virt=warn
#
# Common dependencies to consider filtering:
#   - bollard:    Docker client library
#   - turso_core: Embedded database
#   - virt:       Libvirt client
#   - tower_http: HTTP server middleware

# Custom configuration (add as needed)
# SHERPA_CUSTOM_VAR=value
ENVEXAMPLE
    chmod 640 "${SHERPA_ENV_DIR}/sherpa.env.example"
    chown sherpa:sherpa "${SHERPA_ENV_DIR}/sherpa.env.example"

    # Create actual environment file
    print_info "Creating environment file..."
    cat > "${SHERPA_ENV_DIR}/sherpa.env" << EOF
# Sherpa Environment Configuration
# Generated by sherpa_install.sh on $(date)

# Server listen IP address
SHERPA_SERVER_IPV4=${SERVER_IP}

# Server listen port
SHERPA_SERVER_WS_PORT=${WS_PORT}

# HTTP certificate endpoint port
SHERPA_SERVER_HTTP_PORT=${HTTP_PORT}

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

Database:
  File:     ${SHERPA_DB_FILE} (created by 'sherpad init', used only by sherpad)
  Backups:  sherpa server backup  -- writes a copy to ${SHERPA_DB_DIR}/backups

Useful Commands:
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
    SERVER_IP="${SHERPA_SERVER_IPV4:-}"
    WS_PORT="${SHERPA_SERVER_WS_PORT:-3030}"
    HTTP_PORT="${SHERPA_SERVER_HTTP_PORT:-3031}"

    while [ $# -gt 0 ]; do
        case "$1" in
            --version)
                if [ $# -lt 2 ] || [ -z "$2" ] || [[ "$2" == --* ]]; then
                    print_error "--version requires a version"
                    exit 1
                fi
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

    # Set up error trap
    trap cleanup_on_error EXIT
    
    # Print header
    print_header "Sherpa Installation"
    
    # Run all checks and setup
    check_ubuntu_version
    check_root_privileges
    check_virtualization
    check_curl_installed
    check_legacy_database
    get_server_ip

    echo ""
    print_info "Starting installation..."
    echo ""

    install_system_packages
    enable_libvirtd
    configure_transparent_hugepages
    install_docker
    setup_sherpa_user
    setup_directories
    pull_sherpa_router_image
    
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
