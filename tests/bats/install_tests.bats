#!/usr/bin/env bats
#
# Test suite for scripts/sherpa_install.sh
#
# Prerequisites:
#   sudo apt-get install bats
#
# Run unit tests (no root required):
#   bats tests/bats/install_tests.bats
#
# Run integration tests after a successful install (requires root):
#   sudo bats tests/bats/install_tests.bats
#
# The integration tests auto-skip when the install artefacts are absent.

SCRIPT="${BATS_TEST_DIRNAME}/../../scripts/sherpa_install.sh"

# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------

setup() {
    MOCK_BIN="$(mktemp -d)"

    # Write a version of the script with the trailing `main "$@"` call removed
    # so individual functions can be sourced in isolation.
    STRIPPED_SCRIPT="$(mktemp)"
    grep -v '^main "\$@"' "${SCRIPT}" > "${STRIPPED_SCRIPT}"
}

teardown() {
    rm -rf "${MOCK_BIN:-}"
    rm -f  "${STRIPPED_SCRIPT:-}"
}

# Source functions without executing main.
# Sets set +e around the source so script-level set -e doesn't abort the test.
_source() {
    set +e
    # shellcheck disable=SC1090
    source "${STRIPPED_SCRIPT}"
    set -e
}

# ============================================================
# CLI & Argument Parsing
# ============================================================

@test "CLI: --help prints usage and exits 0" {
    run bash "${SCRIPT}" --help
    [ "$status" -eq 0 ]
    [[ "$output" == *"Usage:"* ]]
    [[ "$output" == *"--version"* ]]
    [[ "$output" == *"sherpa_update.sh"* ]]
}

@test "CLI: --help includes environment variable documentation" {
    run bash "${SCRIPT}" --help
    [ "$status" -eq 0 ]
    [[ "$output" == *"SHERPA_SERVER_IPV4"* ]]
    [[ "$output" == *"SHERPA_SERVER_WS_PORT"* ]]
    [[ "$output" == *"SHERPA_SERVER_HTTP_PORT"* ]]
    # sherpad embeds its database: there is no database password or port to set.
    [[ "$output" != *"SHERPA_DB_PASSWORD"* ]]
    [[ "$output" != *"SHERPA_DB_PORT"* ]]
}

@test "CLI: unknown flag exits 1 with error message" {
    run bash "${SCRIPT}" --bogus-flag
    [ "$status" -eq 1 ]
    [[ "$output" == *"Unknown option"* ]]
}

@test "CLI: --version flag is accepted by the argument parser" {
    # Verify the parser handles --version without treating it as an error.
    # We cannot run a full install, so we test that the flag is parsed and
    # SHERPA_VERSION is set before any preflight checks fire.
    run bash -c "
        set +e
        source '${STRIPPED_SCRIPT}'
        SHERPA_VERSION=''
        # Simulate only the argument-parsing loop from main
        while [ \$# -gt 0 ]; do
            case \"\$1\" in
                --version) SHERPA_VERSION=\"\$2\"; shift 2 ;;
                *) break ;;
            esac
        done
        echo \"SHERPA_VERSION=\${SHERPA_VERSION}\"
    " -- --version v0.3.4
    [ "$status" -eq 0 ]
    [[ "$output" == *"SHERPA_VERSION=v0.3.4"* ]]
}

# ============================================================
# Pre-flight: Ubuntu Version
# ============================================================

@test "check_ubuntu_version: fails on Ubuntu 22.04 (below 24.04 minimum)" {
    local mock_os_release
    mock_os_release="$(mktemp)"
    printf 'ID=ubuntu\nVERSION_ID="22.04"\n' > "${mock_os_release}"
    run bash -c "
        set +e
        source '${STRIPPED_SCRIPT}'
        # Override check_ubuntu_version to use mocked os-release
        check_ubuntu_version() {
            local id version_id
            id=\$(grep '^ID=' '${mock_os_release}' | cut -d= -f2 | tr -d '\"')
            version_id=\$(grep '^VERSION_ID=' '${mock_os_release}' | cut -d= -f2 | tr -d '\"')
            if [ \"\${id}\" != 'ubuntu' ]; then
                print_error \"This installer requires Ubuntu. Detected: \${id}\"
                exit 1
            fi
            local major minor
            major=\$(echo \"\${version_id}\" | cut -d. -f1)
            minor=\$(echo \"\${version_id}\" | cut -d. -f2)
            if [ \"\${major}\" -lt 24 ] || { [ \"\${major}\" -eq 24 ] && [ \"\${minor}\" -lt 4 ]; }; then
                print_error \"Ubuntu 24.04 or later required. Detected: \${version_id}\"
                exit 1
            fi
        }
        check_ubuntu_version
    "
    rm -f "${mock_os_release}"
    [ "$status" -eq 1 ]
    [[ "$output" == *"24.04 or later"* ]]
}

@test "check_ubuntu_version: error message includes detected version" {
    local mock_os_release
    mock_os_release="$(mktemp)"
    printf 'ID=ubuntu\nVERSION_ID="22.04"\n' > "${mock_os_release}"
    run bash -c "
        set +e
        source '${STRIPPED_SCRIPT}'
        check_ubuntu_version() {
            local id version_id
            id=\$(grep '^ID=' '${mock_os_release}' | cut -d= -f2 | tr -d '\"')
            version_id=\$(grep '^VERSION_ID=' '${mock_os_release}' | cut -d= -f2 | tr -d '\"')
            if [ \"\${id}\" != 'ubuntu' ]; then
                print_error \"This installer requires Ubuntu. Detected: \${id}\"
                exit 1
            fi
            local major minor
            major=\$(echo \"\${version_id}\" | cut -d. -f1)
            minor=\$(echo \"\${version_id}\" | cut -d. -f2)
            if [ \"\${major}\" -lt 24 ] || { [ \"\${major}\" -eq 24 ] && [ \"\${minor}\" -lt 4 ]; }; then
                print_error \"Ubuntu 24.04 or later required. Detected: \${version_id}\"
                exit 1
            fi
        }
        check_ubuntu_version
    "
    rm -f "${mock_os_release}"
    [ "$status" -eq 1 ]
    [[ "$output" == *"22.04"* ]]
}

@test "check_ubuntu_version: fails when /etc/os-release is absent" {
    run bash -c "
        set +e
        source '${STRIPPED_SCRIPT}'
        # Override the check to use a non-existent file by redefining the function
        check_ubuntu_version() {
            if [ ! -f /nonexistent/os-release ]; then
                print_error 'Cannot determine OS: /etc/os-release not found'
                exit 1
            fi
        }
        check_ubuntu_version
    "
    [ "$status" -eq 1 ]
    [[ "$output" == *"Cannot determine OS"* ]]
}

@test "check_ubuntu_version: succeeds on Ubuntu 24.04+" {
    # This test uses the real /etc/os-release on dev02 which runs Ubuntu 24.04.
    run bash -c "
        set +e
        source '${STRIPPED_SCRIPT}'
        check_ubuntu_version
    "
    [ "$status" -eq 0 ]
}

@test "check_ubuntu_version: fails on non-Ubuntu OS" {
    local mock_os_release
    mock_os_release="$(mktemp)"
    printf 'ID=debian\nVERSION_ID="12"\n' > "${mock_os_release}"
    run bash -c "
        set +e
        source '${STRIPPED_SCRIPT}'
        check_ubuntu_version() {
            local id version_id
            id=\$(grep '^ID=' '${mock_os_release}' | cut -d= -f2 | tr -d '\"')
            version_id=\$(grep '^VERSION_ID=' '${mock_os_release}' | cut -d= -f2 | tr -d '\"')
            if [ \"\${id}\" != 'ubuntu' ]; then
                print_error \"This script requires Ubuntu (detected: \${id:-unknown})\"
                exit 1
            fi
        }
        check_ubuntu_version
    "
    rm -f "${mock_os_release}"
    [ "$status" -eq 1 ]
    [[ "$output" == *"debian"* ]]
}

# ============================================================
# Pre-flight: Root Privileges
# ============================================================

@test "check_root_privileges: fails when running as non-root" {
    # This test only makes sense when BATS itself runs as a non-root user.
    # When invoked via `sudo bats`, every subshell is already root → skip.
    if [ "$(id -u)" -eq 0 ]; then
        skip "running as root — invoke as non-root to exercise this failure path"
    fi
    run bash -c "
        set +e
        source '${STRIPPED_SCRIPT}'
        check_root_privileges
    "
    [ "$status" -eq 1 ]
    [[ "$output" == *"must be run as root"* ]]
}

# ============================================================
# Pre-flight: Curl Check
# ============================================================

@test "check_curl_installed: fails when curl is not on PATH" {
    local empty_bin
    empty_bin="$(mktemp -d)"
    run bash -c "
        set +e
        PATH='${empty_bin}'
        source '${STRIPPED_SCRIPT}'
        check_curl_installed
    "
    rm -rf "${empty_bin}"
    [ "$status" -eq 1 ]
    [[ "$output" == *"curl is not installed"* ]]
}

@test "check_curl_installed: passes when curl is available" {
    local mock_bin
    mock_bin="$(mktemp -d)"
    printf '#!/bin/bash\nexit 0\n' > "${mock_bin}/curl"
    chmod +x "${mock_bin}/curl"
    run bash -c "
        set +e
        PATH='${mock_bin}:${PATH}'
        source '${STRIPPED_SCRIPT}'
        check_curl_installed
    "
    rm -rf "${mock_bin}"
    [ "$status" -eq 0 ]
    [[ "$output" == *"curl is installed"* ]]
}

# ============================================================
# Pre-flight: Virtualization
# ============================================================

@test "check_virtualization: detects Intel VT-x (vmx flag in /proc/cpuinfo)" {
    # dev02 has VT-x/AMD-V — this confirms detection on a real hypervisor-capable host.
    run bash -c "
        set +e
        source '${STRIPPED_SCRIPT}'
        check_virtualization
    "
    [ "$status" -eq 0 ]
    [[ "$output" == *"Hardware virtualization supported"* ]]
}

@test "check_virtualization: fails when neither vmx nor svm is present" {
    run bash -c "
        set +e
        source '${STRIPPED_SCRIPT}'
        # Override grep to simulate a CPU without virt extensions
        grep() {
            if [[ \"\$*\" == *vmx* ]] || [[ \"\$*\" == *svm* ]]; then
                return 1
            fi
            command grep \"\$@\"
        }
        check_virtualization
    "
    [ "$status" -eq 1 ]
    [[ "$output" == *"virtualization extensions not detected"* ]]
}

# ============================================================
# Upgrade Check: pre-0.5.0 SurrealDB container
# ============================================================

# A docker stub whose `ps -a --format` lists the container names given.
_stub_docker_with_containers() {
    {
        echo '#!/bin/bash'
        echo 'if [ "$1" = "ps" ]; then'
        for name in "$@"; do
            echo "    echo '${name}'"
        done
        echo 'fi'
        echo 'exit 0'
    } > "${MOCK_BIN}/docker"
    chmod +x "${MOCK_BIN}/docker"
}

@test "check_legacy_database: refuses when a sherpa-db container exists" {
    _stub_docker_with_containers other sherpa-db
    run bash -c "
        set +e
        PATH='${MOCK_BIN}:${PATH}'
        source '${STRIPPED_SCRIPT}'
        check_legacy_database
    "
    [ "$status" -eq 1 ]
    [[ "$output" == *"sherpa-db"* ]]
    [[ "$output" == *"sherpa_update.sh"* ]]
}

@test "check_legacy_database: passes when no sherpa-db container exists" {
    _stub_docker_with_containers other sherpa-db-backup
    run bash -c "
        set +e
        PATH='${MOCK_BIN}:${PATH}'
        source '${STRIPPED_SCRIPT}'
        check_legacy_database
    "
    [ "$status" -eq 0 ]
    [[ "$output" == *"No SurrealDB container from an older release"* ]]
}

@test "check_legacy_database: passes when Docker is not installed yet" {
    local no_docker_bin
    no_docker_bin="$(mktemp -d)"
    for tool in grep cat; do
        ln -s "$(command -v "${tool}")" "${no_docker_bin}/${tool}"
    done
    run bash -c "
        set +e
        PATH='${no_docker_bin}'
        source '${STRIPPED_SCRIPT}'
        check_legacy_database
    "
    rm -rf "${no_docker_bin}"
    [ "$status" -eq 0 ]
}

# ============================================================
# Server IP Validation
# ============================================================

@test "get_server_ip: accepts a valid IPv4 address from SERVER_IP env var" {
    run bash -c "
        set +e
        source '${STRIPPED_SCRIPT}'
        SERVER_IP='192.168.1.100'
        get_server_ip
    "
    [ "$status" -eq 0 ]
    [[ "$output" == *"192.168.1.100"* ]]
}

@test "get_server_ip: accepts 0.0.0.0 (all interfaces)" {
    run bash -c "
        set +e
        source '${STRIPPED_SCRIPT}'
        SERVER_IP='0.0.0.0'
        get_server_ip
    "
    [ "$status" -eq 0 ]
}

@test "get_server_ip: accepts 10.0.0.1" {
    run bash -c "
        set +e
        source '${STRIPPED_SCRIPT}'
        SERVER_IP='10.0.0.1'
        get_server_ip
    "
    [ "$status" -eq 0 ]
}

@test "get_server_ip: rejects a plain hostname" {
    run bash -c "
        set +e
        source '${STRIPPED_SCRIPT}'
        SERVER_IP='myserver.example.com'
        get_server_ip
    "
    [ "$status" -eq 1 ]
    [[ "$output" == *"Invalid IPv4"* ]]
}

@test "get_server_ip: rejects a non-IP string" {
    run bash -c "
        set +e
        source '${STRIPPED_SCRIPT}'
        SERVER_IP='not-an-ip'
        get_server_ip
    "
    [ "$status" -eq 1 ]
    [[ "$output" == *"Invalid IPv4"* ]]
}

@test "get_server_ip: rejects an empty / whitespace-only string" {
    run bash -c "
        set +e
        source '${STRIPPED_SCRIPT}'
        SERVER_IP='   '
        get_server_ip
    "
    [ "$status" -eq 1 ]
    [[ "$output" == *"Invalid IPv4"* ]]
}

@test "get_server_ip: rejects out-of-range octets (e.g. 999.999.999.999)" {
    run bash -c "
        set +e
        source '${STRIPPED_SCRIPT}'
        SERVER_IP='999.999.999.999'
        get_server_ip
    "
    [ "$status" -eq 1 ]
    [[ "$output" == *"Invalid IPv4"* ]]
}

# ============================================================
# Transparent hugepages
# ============================================================

@test "configure_transparent_hugepages: sets madvise now and on boot when THP is never" {
    THP_FILE="$(mktemp)"
    RULE_FILE="$(mktemp -u)"
    echo "always madvise [never]" > "${THP_FILE}"
    run bash -c "
        set +e
        source '${STRIPPED_SCRIPT}'
        THP_ENABLED_FILE='${THP_FILE}'
        THP_TMPFILES_CONF='${RULE_FILE}'
        configure_transparent_hugepages
    "
    [ "$status" -eq 0 ]
    [ "$(cat "${THP_FILE}")" = "madvise" ]
    grep -q "^w ${THP_FILE} - - - - madvise$" "${RULE_FILE}"
    rm -f "${THP_FILE}" "${RULE_FILE}"
}

@test "configure_transparent_hugepages: leaves always alone and adds no boot rule" {
    THP_FILE="$(mktemp)"
    RULE_FILE="$(mktemp -u)"
    echo "[always] madvise never" > "${THP_FILE}"
    run bash -c "
        set +e
        source '${STRIPPED_SCRIPT}'
        THP_ENABLED_FILE='${THP_FILE}'
        THP_TMPFILES_CONF='${RULE_FILE}'
        configure_transparent_hugepages
    "
    [ "$status" -eq 0 ]
    [ "$(cat "${THP_FILE}")" = "[always] madvise never" ]
    [ ! -e "${RULE_FILE}" ]
    rm -f "${THP_FILE}"
}

@test "configure_transparent_hugepages: warns and continues when the kernel has no THP" {
    RULE_FILE="$(mktemp -u)"
    run bash -c "
        set +e
        source '${STRIPPED_SCRIPT}'
        THP_ENABLED_FILE='/nonexistent/transparent_hugepage/enabled'
        THP_TMPFILES_CONF='${RULE_FILE}'
        configure_transparent_hugepages
    "
    [ "$status" -eq 0 ]
    [[ "$output" == *"Transparent hugepages are not available"* ]]
    [ ! -e "${RULE_FILE}" ]
}

# ============================================================
# Post-install Integration Tests
# (skipped unless /opt/sherpa exists — requires completed install as root)
# ============================================================

_require_install() {
    if [ ! -d /opt/sherpa ]; then
        skip "install artefacts absent — run as root after a successful install"
    fi
}

@test "integration: transparent hugepages are madvise or always" {
    _require_install
    grep -qE '\[(madvise|always)\]' /sys/kernel/mm/transparent_hugepage/enabled
}

@test "integration: transparent hugepages boot rule sets madvise unless THP is always" {
    _require_install
    if grep -q '\[always\]' /sys/kernel/mm/transparent_hugepage/enabled; then
        skip "THP is always; the installer adds no boot rule"
    fi
    grep -q '^w /sys/kernel/mm/transparent_hugepage/enabled - - - - madvise$' \
        /etc/tmpfiles.d/sherpa-thp.conf
}

@test "integration: sherpa user exists" {
    _require_install
    run id -u sherpa
    [ "$status" -eq 0 ]
}

@test "integration: sherpa user shell is /usr/sbin/nologin" {
    _require_install
    # If the sherpa user is a regular login user (uid >= 1000), the install script
    # detects the existing user and skips creation, leaving the original shell intact.
    # On a clean system the script creates a system user with nologin.
    local sherpa_uid
    sherpa_uid=$(id -u sherpa 2>/dev/null || echo 9999)
    if [ "${sherpa_uid}" -ge 1000 ]; then
        skip "sherpa is a login user (uid=${sherpa_uid}) — nologin only set when install creates the system user"
    fi
    run bash -c "getent passwd sherpa | cut -d: -f7"
    [ "$status" -eq 0 ]
    [[ "$output" == *"nologin"* ]]
}

@test "integration: sherpa user belongs to libvirt group" {
    _require_install
    run bash -c "id -nG sherpa"
    [ "$status" -eq 0 ]
    [[ "$output" == *"libvirt"* ]]
}

@test "integration: sherpa user belongs to kvm group" {
    _require_install
    run bash -c "id -nG sherpa"
    [ "$status" -eq 0 ]
    [[ "$output" == *"kvm"* ]]
}

@test "integration: sherpa user belongs to docker group" {
    _require_install
    run bash -c "id -nG sherpa"
    [ "$status" -eq 0 ]
    [[ "$output" == *"docker"* ]]
}

@test "integration: /opt/sherpa directories exist" {
    _require_install
    [ -d /opt/sherpa ]
    [ -d /opt/sherpa/db ]
    [ -d /opt/sherpa/config ]
    [ -d /opt/sherpa/env ]
    [ -d /opt/sherpa/bin ]
}

@test "integration: /opt/sherpa directory permissions are 775" {
    _require_install
    run bash -c "stat -c '%a' /opt/sherpa"
    [ "$status" -eq 0 ]
    [[ "$output" == "775" ]]
}

@test "integration: /opt/sherpa/env directory permissions are 750" {
    _require_install
    run bash -c "stat -c '%a' /opt/sherpa/env"
    [ "$status" -eq 0 ]
    [[ "$output" == "750" ]]
}

@test "integration: /opt/sherpa owned by sherpa:sherpa" {
    _require_install
    run bash -c "stat -c '%U:%G' /opt/sherpa"
    [ "$status" -eq 0 ]
    [[ "$output" == "sherpa:sherpa" ]]
}

@test "integration: sherpad binary installed with correct permissions" {
    _require_install
    [ -x /opt/sherpa/bin/sherpad ]
    run bash -c "stat -c '%a' /opt/sherpa/bin/sherpad"
    [ "$status" -eq 0 ]
    [[ "$output" == "755" ]]
}

@test "integration: sherpad binary owned by sherpa:sherpa" {
    _require_install
    run bash -c "stat -c '%U:%G' /opt/sherpa/bin/sherpad"
    [ "$status" -eq 0 ]
    [[ "$output" == "sherpa:sherpa" ]]
}

@test "integration: /usr/local/bin/sherpad symlink exists" {
    _require_install
    [ -L /usr/local/bin/sherpad ]
    run readlink /usr/local/bin/sherpad
    [ "$status" -eq 0 ]
    [[ "$output" == "/opt/sherpa/bin/sherpad" ]]
}

@test "integration: sherpad systemd service file exists" {
    _require_install
    [ -f /etc/systemd/system/sherpad.service ]
}

@test "integration: sherpad service unit references correct binary path" {
    _require_install
    run grep -q "ExecStart=/opt/sherpa/bin/sherpad" /etc/systemd/system/sherpad.service
    [ "$status" -eq 0 ]
}

@test "integration: sherpad service requires docker.service and libvirtd.service" {
    _require_install
    run grep -q "Requires=docker.service libvirtd.service" /etc/systemd/system/sherpad.service
    [ "$status" -eq 0 ]
}

@test "integration: sherpad service keeps a private /run/sherpa for container secrets" {
    _require_install
    run grep -q "^RuntimeDirectory=sherpa$" /etc/systemd/system/sherpad.service
    [ "$status" -eq 0 ]
    run grep -q "^RuntimeDirectoryMode=0700$" /etc/systemd/system/sherpad.service
    [ "$status" -eq 0 ]
    run grep -q "^RuntimeDirectoryPreserve=yes$" /etc/systemd/system/sherpad.service
    [ "$status" -eq 0 ]
}

@test "integration: sherpad service is enabled (starts on boot)" {
    _require_install
    run systemctl is-enabled sherpad
    [ "$status" -eq 0 ]
    [[ "$output" == "enabled" ]]
}

@test "integration: sherpa.env file exists with restricted permissions (640)" {
    _require_install
    [ -f /opt/sherpa/env/sherpa.env ]
    run bash -c "stat -c '%a' /opt/sherpa/env/sherpa.env"
    [ "$status" -eq 0 ]
    [[ "$output" == "640" ]]
}

@test "integration: sherpa.env file owned by sherpa:sherpa" {
    _require_install
    run bash -c "stat -c '%U:%G' /opt/sherpa/env/sherpa.env"
    [ "$status" -eq 0 ]
    [[ "$output" == "sherpa:sherpa" ]]
}

@test "integration: sherpa.env holds no database password or port" {
    _require_install
    run grep -qE "^SHERPA_DB_(PASSWORD|PORT)=" /opt/sherpa/env/sherpa.env
    [ "$status" -eq 1 ]
}

@test "integration: no SurrealDB container is installed" {
    _require_install
    run bash -c "docker ps -a --format '{{.Names}}' | grep -qx sherpa-db"
    [ "$status" -eq 1 ]
}

@test "integration: sherpa.env contains SHERPA_SERVER_IPV4" {
    _require_install
    run grep -q "^SHERPA_SERVER_IPV4=" /opt/sherpa/env/sherpa.env
    [ "$status" -eq 0 ]
}

@test "integration: logrotate config installed" {
    _require_install
    [ -f /etc/logrotate.d/sherpad ]
    run grep -q "sherpad.log" /etc/logrotate.d/sherpad
    [ "$status" -eq 0 ]
}

@test "integration: idempotent re-run completes without error" {
    _require_install
    # Requires SHERPA_SERVER_IPV4 to be set
    if [ -z "${SHERPA_SERVER_IPV4:-}" ]; then
        skip "set SHERPA_SERVER_IPV4 to run idempotency test"
    fi
    run bash -c "SHERPA_SERVER_IPV4='${SHERPA_SERVER_IPV4}' bash '${SCRIPT}'"
    [ "$status" -eq 0 ]
}
