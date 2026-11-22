#!/usr/bin/env bats
#
# Test suite for scripts/sherpa_install.sh. Each test is a case of tests/specs/install-and-update.md
# and is named by its ID.
#
# Prerequisites:
#   sudo apt-get install bats
#
# Run (no root required):
#   bats tests/bats/install_tests.bats
#
# What an install leaves on a host is checked by the VM run (tests/harness/test_install.sh).

SCRIPT="${BATS_TEST_DIRNAME}/../../scripts/sherpa_install.sh"
UPDATE_SCRIPT="${BATS_TEST_DIRNAME}/../../scripts/sherpa_update.sh"

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
    rm -rf "${MOCK_BIN:-}" "${WORK_DIR:-}"
    rm -f  "${STRIPPED_SCRIPT:-}"
}

# The blank disk templates

# Each blank disk as "name bytes type label FAT-bits": what the ZTP code in crates/server
# expects. "-" is none.
_EXPECTED_BLANK_DISKS="base.img 67108864 - - -
fat16.img 67108864 vfat - 16
fat32.img 67108864 vfat - 32
junos.img 67108864 vfat vmm-data 16
iosv.img 67092480 vfat - 16
ise.img 10485760 ext4 ISE-ZTP -
ext4-100mb.img 104857600 ext4 data-disk -
ext4-200mb.img 209715200 ext4 data-disk -
ext4-500mb.img 524288000 ext4 data-disk -
ext4-1g.img 1073741824 ext4 data-disk -
ext4-2g.img 2147483648 ext4 data-disk -
ext4-3g.img 3221225472 ext4 data-disk -
ext4-4g.img 4294967296 ext4 data-disk -
ext4-5g.img 5368709120 ext4 data-disk -"

# Check every blank disk in the given folder has its size, file system and label, and print the
# first that does not.
_check_blank_disks() {
    local dir="$1" name bytes type label bits disk
    while read -r name bytes type label bits; do
        disk="${dir}/${name}"
        [ -f "${disk}" ] || { echo "${name}: missing"; return 1; }
        [ "$(stat -c '%s' "${disk}")" = "${bytes}" ] || { echo "${name}: size"; return 1; }
        if [ "${type}" = "-" ]; then
            [ -z "$(blkid -p -o value -s TYPE "${disk}")" ] || { echo "${name}: has a file system"; return 1; }
            continue
        fi
        [ "$(blkid -p -o value -s TYPE "${disk}")" = "${type}" ] || { echo "${name}: type"; return 1; }
        if [ "${label}" = "-" ]; then
            [ -z "$(blkid -p -o value -s LABEL "${disk}")" ] || { echo "${name}: has a label"; return 1; }
        else
            [ "$(blkid -p -o value -s LABEL "${disk}")" = "${label}" ] || { echo "${name}: label"; return 1; }
        fi
        if [ "${bits}" != "-" ]; then
            [[ "$(file -b "${disk}")" == *"FAT (${bits} bit)"* ]] || { echo "${name}: FAT bits"; return 1; }
        fi
    done <<< "${_EXPECTED_BLANK_DISKS}"
}

_require_blank_disk_tools() {
    local tool
    for tool in qemu-img mkfs.fat mkfs.vfat mkfs.ext4 blkid file; do
        command -v "${tool}" >/dev/null 2>&1 || skip "${tool} is not installed"
    done
}

# Run ensure_blank_disks of the given script, sourced without its main, on an images folder under
# WORK_DIR, with chown stubbed. $2 runs before it.
_run_ensure_blank_disks() {
    local stripped="${WORK_DIR}/script.sh"
    grep -v '^main "\$@"' "$1" > "${stripped}"
    printf '#!/bin/bash\necho "$@" >> "%s/chown.args"\nexit 0\n' "${WORK_DIR}" > "${MOCK_BIN}/chown"
    chmod +x "${MOCK_BIN}/chown"
    run bash -c "
        set +e
        PATH='${MOCK_BIN}:${PATH}'
        source '${stripped}'
        SHERPA_IMAGES_DIR='${WORK_DIR}/images'
        SHERPA_BLANK_DISK_DIR='${WORK_DIR}/images/blank_disk'
        $2
        ensure_blank_disks
    "
}

# The cases of blank_disks 002 to 006 for the blank disk section of the given script. Each starts
# from an empty WORK_DIR and removes it at the end.

_blank_disks_case_002() {
    WORK_DIR="$(mktemp -d)"
    _run_ensure_blank_disks "$1"
    [ "$status" -eq 0 ]
    run _check_blank_disks "${WORK_DIR}/images/blank_disk"
    echo "${output}"
    [ "$status" -eq 0 ]
    [ "$(stat -c '%a' "${WORK_DIR}/images/blank_disk")" = "775" ]
    [ "$(stat -c '%a' "${WORK_DIR}/images/blank_disk/fat32.img")" = "644" ]
    grep -qx "sherpa:sherpa ${WORK_DIR}/images/blank_disk" "${WORK_DIR}/chown.args"
    # No temporary file is left behind.
    [ "$(find "${WORK_DIR}/images/blank_disk" -name '.*' -type f | wc -l)" -eq 0 ]
    rm -rf "${WORK_DIR}"
}

_blank_disks_case_003() {
    WORK_DIR="$(mktemp -d)"
    mkdir -p "${WORK_DIR}/images/blank_disk"
    chmod 750 "${WORK_DIR}/images/blank_disk"
    printf 'a node disk was copied from this\n' > "${WORK_DIR}/images/blank_disk/fat32.img"
    chmod 600 "${WORK_DIR}/images/blank_disk/fat32.img"
    ln -s "${WORK_DIR}/nowhere" "${WORK_DIR}/images/blank_disk/junos.img"
    local before
    before="$(sha256sum "${WORK_DIR}/images/blank_disk/fat32.img")"
    _run_ensure_blank_disks "$1"
    [ "$status" -eq 0 ]
    [ "$(sha256sum "${WORK_DIR}/images/blank_disk/fat32.img")" = "${before}" ]
    [ "$(stat -c '%a' "${WORK_DIR}/images/blank_disk/fat32.img")" = "600" ]
    [ "$(stat -c '%a' "${WORK_DIR}/images/blank_disk")" = "750" ]
    [ -L "${WORK_DIR}/images/blank_disk/junos.img" ]
    [ ! -e "${WORK_DIR}/nowhere" ]
    run grep -q "blank_disk$" "${WORK_DIR}/chown.args"
    [ "$status" -ne 0 ]
    [ -f "${WORK_DIR}/images/blank_disk/ise.img" ]
    [ "$(blkid -p -o value -s LABEL "${WORK_DIR}/images/blank_disk/ise.img")" = "ISE-ZTP" ]
    rm -rf "${WORK_DIR}"
}

_blank_disks_case_004() {
    WORK_DIR="$(mktemp -d)"
    _run_ensure_blank_disks "$1"
    [ "$status" -eq 0 ]
    # Inode, size and modification time: hashing 16 GB of sparse disks takes minutes.
    local before
    before="$(stat -c '%n %i %s %Y' "${WORK_DIR}"/images/blank_disk/*.img)"
    _run_ensure_blank_disks "$1"
    [ "$status" -eq 0 ]
    [[ "$output" != *"Created blank disk"* ]]
    [ "$(stat -c '%n %i %s %Y' "${WORK_DIR}"/images/blank_disk/*.img)" = "${before}" ]
    rm -rf "${WORK_DIR}"
}

_blank_disks_case_005() {
    WORK_DIR="$(mktemp -d)"
    _run_ensure_blank_disks "$1" "BLANK_DISK_TOOLS=(qemu-img no-such-mkfs)"
    [ "$status" -eq 1 ]
    [[ "$output" == *"no-such-mkfs"* ]]
    [[ "$output" == *"qemu-utils dosfstools e2fsprogs"* ]]
    [ ! -e "${WORK_DIR}/images" ]
    rm -rf "${WORK_DIR}"
}

_blank_disks_case_006() {
    WORK_DIR="$(mktemp -d)"
    printf '#!/bin/bash\nexit 1\n' > "${MOCK_BIN}/mkfs.ext4"
    chmod +x "${MOCK_BIN}/mkfs.ext4"
    _run_ensure_blank_disks "$1"
    rm -f "${MOCK_BIN}/mkfs.ext4"
    [ "$status" -eq 1 ]
    [[ "$output" == *"Could not create blank disk ise.img"* ]]
    [ ! -e "${WORK_DIR}/images/blank_disk/ise.img" ]
    [ "$(find "${WORK_DIR}/images/blank_disk" -name '.*' -type f | wc -l)" -eq 0 ]
    [ -f "${WORK_DIR}/images/blank_disk/iosv.img" ]
    rm -rf "${WORK_DIR}"
}

# Stubs for installing Docker on a host without it: PATH is MOCK_BIN and the few tools the
# function uses, so there is no docker. curl records its arguments and writes WORK_DIR/key for
# the key; apt-get records its arguments; dpkg prints amd64.
_stub_docker_install() {
    TOOLS="$(mktemp -d "${WORK_DIR}/tools.XXXXXX")"
    local tool
    for tool in bash cat mktemp rm sha256sum awk dirname chmod; do
        ln -s "$(command -v "${tool}")" "${TOOLS}/${tool}"
    done
    cat > "${MOCK_BIN}/curl" << STUB
#!/bin/bash
echo "\$@" >> '${WORK_DIR}/curl.args'
while [ \$# -gt 1 ]; do
    [ "\$1" = "-o" ] && cp '${WORK_DIR}/key' "\$2"
    shift
done
STUB
    printf '#!/bin/bash\necho "$@" >> "%s/apt-get.args"\n' "${WORK_DIR}" > "${MOCK_BIN}/apt-get"
    printf '#!/bin/bash\necho amd64\n' > "${MOCK_BIN}/dpkg"
    printf '#!/bin/bash\nexit 0\n' > "${MOCK_BIN}/systemctl"
    printf '#!/bin/bash\nexec %s "$@"\n' "$(command -v install)" > "${MOCK_BIN}/install"
    printf '#!/bin/bash\nexec %s "$@"\n' "$(command -v cp)" > "${MOCK_BIN}/cp"
    chmod +x "${MOCK_BIN}"/*
    printf 'ID=ubuntu\nVERSION_ID="26.04"\nVERSION_CODENAME=resolute\nUBUNTU_CODENAME=resolute\n' > "${WORK_DIR}/os-release"
    printf -- '-----BEGIN PGP PUBLIC KEY BLOCK-----\nkey\n' > "${WORK_DIR}/key"
}

# Run install_docker with the stubs, the apt files under WORK_DIR and the key pinned to $1.
_run_install_docker() {
    run bash -c "
        set +e
        PATH='${MOCK_BIN}:${TOOLS}'
        source '${STRIPPED_SCRIPT}'
        DOCKER_APT_KEY_SHA256='$1'
        DOCKER_APT_KEYRING='${WORK_DIR}/keyrings/docker.asc'
        DOCKER_APT_SOURCES='${WORK_DIR}/docker.sources'
        OS_RELEASE_FILE='${WORK_DIR}/os-release'
        install_docker
    "
}

# ============================================================
# install_preflight
# ============================================================

@test "install_preflight_001_not_root_is_refused_with_the_sudo_command" {
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
    [[ "$output" == *"Please run: sudo"* ]]
}

@test "install_preflight_003_a_cpu_without_vt_x_or_amd_v_is_refused" {
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

@test "install_preflight_004_without_curl_it_stops_and_says_how_to_install_it" {
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
    [[ "$output" == *"sudo apt-get install curl"* ]]
}

# A docker stub whose `ps -a --format` lists the container names given.
_stub_docker_with_containers() {
    {
        echo '#!/bin/bash'
        # The stub's own variables: they expand when the stub runs, not here.
        # shellcheck disable=SC2016
        echo 'if [ "$1" = "ps" ]; then'
        for name in "$@"; do
            echo "    echo '${name}'"
        done
        echo 'fi'
        echo 'exit 0'
    } > "${MOCK_BIN}/docker"
    chmod +x "${MOCK_BIN}/docker"
}

@test "install_preflight_005_a_sherpa_db_container_is_refused_with_the_release_to_update_to" {
    _stub_docker_with_containers other sherpa-db
    run bash -c "
        set +e
        PATH='${MOCK_BIN}:${PATH}'
        source '${STRIPPED_SCRIPT}'
        check_legacy_database
    "
    [ "$status" -eq 1 ]
    [[ "$output" == *"sherpa-db"* ]]
    [[ "$output" == *"v0.5.12/scripts/sherpa_update.sh"* ]]
    [[ "$output" == *"--version v0.5.12"* ]]

    # A container whose name only contains sherpa-db passes.
    _stub_docker_with_containers other sherpa-db-backup
    run bash -c "
        set +e
        PATH='${MOCK_BIN}:${PATH}'
        source '${STRIPPED_SCRIPT}'
        check_legacy_database
    "
    [ "$status" -eq 0 ]
    [[ "$output" == *"No SurrealDB container from an older release"* ]]

    # A host with no Docker passes.
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

@test "install_preflight_006_a_listen_address_that_is_not_ipv4_is_refused_and_one_given_is_taken" {
    local address
    for address in 192.168.1.100 0.0.0.0 10.0.0.1; do
        # stdin is closed: a prompt would read nothing and take the default.
        run bash -c "
            set +e
            source '${STRIPPED_SCRIPT}'
            SERVER_IP='${address}'
            get_server_ip
            echo \"chosen=\${SERVER_IP}\"
        " < /dev/null
        [ "$status" -eq 0 ]
        [[ "$output" != *"Server listen IP address"* ]]
        [[ "$output" == *"chosen=${address}"* ]]
    done
    for address in myserver.example.com not-an-ip '   ' 999.999.999.999; do
        run bash -c "
            set +e
            source '${STRIPPED_SCRIPT}'
            SERVER_IP='${address}'
            get_server_ip
        " < /dev/null
        [ "$status" -eq 1 ]
        [[ "$output" == *"Invalid IPv4"* ]]
    done
}

@test "install_preflight_007_help_prints_the_usage_with_the_options_and_the_environment_variables" {
    run bash "${SCRIPT}" --help
    [ "$status" -eq 0 ]
    [[ "$output" == *"Usage:"* ]]
    [[ "$output" == *"--version"* ]]
    [[ "$output" == *"sherpa_update.sh"* ]]
    [[ "$output" == *"SHERPA_SERVER_IPV4"* ]]
    [[ "$output" == *"SHERPA_SERVER_WS_PORT"* ]]
    [[ "$output" == *"SHERPA_SERVER_HTTP_PORT"* ]]
}

@test "install_preflight_008_an_unknown_option_and_version_with_no_value_are_refused" {
    run bash "${SCRIPT}" --bogus-flag
    [ "$status" -eq 1 ]
    [[ "$output" == *"Unknown option"* ]]
    run bash "${SCRIPT}" --version
    [ "$status" -ne 0 ]
    [[ "$output" == *"--version requires a version"* ]]
}

# ============================================================
# install_host
# ============================================================

@test "install_host_007_transparent_hugepages_at_never_become_madvise_always_is_left_and_none_is_skipped" {
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
    rm -f "${RULE_FILE}"

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

@test "install_host_011_the_closing_message_gives_the_init_command_with_the_address_and_ports" {
    run bash -c "
        set +e
        source '${STRIPPED_SCRIPT}'
        SERVER_IP=192.0.2.10
        WS_PORT=4030
        HTTP_PORT=4031
        print_success_message
    "
    [ "$status" -eq 0 ]
    [[ "$output" == *"sudo -u sherpa -H sherpad init --server-ipv4 192.0.2.10 --ws-port 4030 --http-port 4031"* ]]
}

# ============================================================
# blank_disks
# ============================================================

@test "blank_disks_002_each_template_is_made_with_its_size_file_system_and_label" {
    _require_blank_disk_tools
    _blank_disks_case_002 "${SCRIPT}"
}

@test "blank_disks_003_templates_folders_and_links_that_are_there_are_kept" {
    _require_blank_disk_tools
    _blank_disks_case_003 "${SCRIPT}"
}

@test "blank_disks_004_a_second_run_makes_nothing_and_touches_no_file" {
    _require_blank_disk_tools
    _blank_disks_case_004 "${SCRIPT}"
}

@test "blank_disks_005_with_a_tool_missing_nothing_is_made_and_the_tools_and_packages_are_named" {
    _blank_disks_case_005 "${SCRIPT}"
}

@test "blank_disks_006_a_template_that_fails_to_build_leaves_no_file_and_stops_the_script" {
    _require_blank_disk_tools
    _blank_disks_case_006 "${SCRIPT}"
}

# Cases 002 to 006 above run the install script's section; this runs the update script's.
@test "blank_disks_009_cases_002_to_006_hold_for_the_update_scripts_section_too" {
    _require_blank_disk_tools
    _blank_disks_case_002 "${UPDATE_SCRIPT}"
    _blank_disks_case_003 "${UPDATE_SCRIPT}"
    _blank_disks_case_004 "${UPDATE_SCRIPT}"
    _blank_disks_case_005 "${UPDATE_SCRIPT}"
    _blank_disks_case_006 "${UPDATE_SCRIPT}"
}

# ============================================================
# Verified downloads
# ============================================================

@test "install_host_016_docker_comes_from_dockers_apt_repository_signed_by_its_key_checked_against_its_pin" {
    WORK_DIR="$(mktemp -d)"
    _stub_docker_install
    _run_install_docker "$(sha256sum "${WORK_DIR}/key" | awk '{ print $1 }')"
    echo "${output}"
    [ "$status" -eq 0 ]
    cmp "${WORK_DIR}/key" "${WORK_DIR}/keyrings/docker.asc"
    [ "$(stat -c '%a' "${WORK_DIR}/keyrings/docker.asc")" = "644" ]
    grep -qx "URIs: https://download.docker.com/linux/ubuntu" "${WORK_DIR}/docker.sources"
    grep -qx "Suites: resolute" "${WORK_DIR}/docker.sources"
    grep -qx "Architectures: amd64" "${WORK_DIR}/docker.sources"
    grep -qx "Signed-By: ${WORK_DIR}/keyrings/docker.asc" "${WORK_DIR}/docker.sources"
    grep -q "^install -y docker-ce docker-ce-cli containerd.io" "${WORK_DIR}/apt-get.args"
    # The key is the only download, and no script is fetched to run.
    [ "$(wc -l < "${WORK_DIR}/curl.args")" -eq 1 ]
    grep -q "https://download.docker.com/linux/ubuntu/gpg$" "${WORK_DIR}/curl.args"
}

@test "install_host_017_a_docker_key_that_is_not_the_pinned_one_stops_the_install_before_apt_trusts_it" {
    WORK_DIR="$(mktemp -d)"
    _stub_docker_install
    _run_install_docker "$(printf '0%.0s' {1..64})"
    [ "$status" -ne 0 ]
    [[ "$output" == *"is not the one this script trusts"* ]]
    [ ! -e "${WORK_DIR}/keyrings/docker.asc" ]
    [ ! -e "${WORK_DIR}/docker.sources" ]
    [ ! -e "${WORK_DIR}/apt-get.args" ]
}

@test "install_host_018_the_router_image_is_pulled_by_its_digest_and_tagged_with_the_name_sherpad_runs" {
    WORK_DIR="$(mktemp -d)"
    printf '#!/bin/bash\necho "$@" >> "%s/docker.args"\nexit 0\n' "${WORK_DIR}" > "${MOCK_BIN}/docker"
    chmod +x "${MOCK_BIN}/docker"
    run bash -c "
        set +e
        PATH='${MOCK_BIN}:${PATH}'
        source '${STRIPPED_SCRIPT}'
        pull_sherpa_router_image
    "
    [ "$status" -eq 0 ]
    local pinned
    pinned="$(sed -n 's|^image pull \(ghcr.io/bwks/sherpa-router@sha256:[0-9a-f]\{64\}\)$|\1|p' "${WORK_DIR}/docker.args")"
    [ -n "${pinned}" ]
    grep -qx "image tag ${pinned} ghcr.io/bwks/sherpa-router:latest" "${WORK_DIR}/docker.args"

    # A pull that fails stops the install, and nothing is tagged.
    rm "${WORK_DIR}/docker.args"
    printf '#!/bin/bash\necho "$@" >> "%s/docker.args"\nexit 1\n' "${WORK_DIR}" > "${MOCK_BIN}/docker"
    run bash -c "
        set +e
        PATH='${MOCK_BIN}:${PATH}'
        source '${STRIPPED_SCRIPT}'
        pull_sherpa_router_image
    "
    [ "$status" -eq 1 ]
    run grep -q "image tag" "${WORK_DIR}/docker.args"
    [ "$status" -ne 0 ]
}
