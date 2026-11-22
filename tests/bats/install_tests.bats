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

# Stubs for running a script's function as a user that is not root, recording in WORK_DIR:
# chown records its arguments and changes nothing; install records its arguments and installs
# without the owner and group; runuser runs its command as this user, with SHERPA_TEST_AS set to
# the account it was asked for.
_stub_root_commands() {
    printf '#!/bin/bash\necho "$@" >> "%s/chown.args"\nexit 0\n' "${WORK_DIR}" > "${MOCK_BIN}/chown"
    cat > "${MOCK_BIN}/install" << EOF
#!/bin/bash
echo "\$@" >> '${WORK_DIR}/install.args'
args=()
while [ \$# -gt 0 ]; do
    case "\$1" in
        -o|-g) shift 2 ;;
        *) args+=("\$1"); shift ;;
    esac
done
exec $(command -v install) "\${args[@]}"
EOF
    cat > "${MOCK_BIN}/runuser" << 'EOF'
#!/bin/bash
[ "$1" = "-u" ] || exit 2
export SHERPA_TEST_AS="$2"
shift 2
[ "$1" = "--" ] && shift
exec "$@"
EOF
    chmod +x "${MOCK_BIN}/chown" "${MOCK_BIN}/install" "${MOCK_BIN}/runuser"
}

# Run ensure_blank_disks of the given script, sourced without its main, on an images folder under
# WORK_DIR, with the commands root runs stubbed. $2 runs before it.
_run_ensure_blank_disks() {
    local stripped="${WORK_DIR}/script.sh"
    grep -v '^main "\$@"' "$1" > "${stripped}"
    _stub_root_commands
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

# ============================================================
# The server's folders and binaries, in both scripts
# ============================================================

# Run the commands $2 of the given script, sourced without its main, with the commands root
# runs stubbed and the base folder at WORK_DIR/opt/sherpa.
_run_server_folders() {
    local stripped="${WORK_DIR}/script.sh"
    grep -v '^main "\$@"' "$1" > "${stripped}"
    _stub_root_commands
    run bash -c "
        set +e
        PATH='${MOCK_BIN}:${PATH}'
        source '${stripped}'
        SHERPA_BASE_DIR='${WORK_DIR}/opt/sherpa'
        $2
    "
}

# The folders an older release's install left: the base folder and the binaries' folder 0775,
# as sherpa's, and the folders below them 0775 or 0755.
_older_release_folders() {
    local base="${WORK_DIR}/opt/sherpa"
    mkdir -p "${base}/bin" "${base}/libvirt/images" "${base}/config" "${base}/env" "${base}/logs" \
        "${base}/db" "${base}/labs" "${base}/.secret" "${base}/images"
    chmod 775 "${base}" "${base}/bin" "${base}/libvirt" "${base}/config" "${base}/env" "${base}/db" \
        "${base}/labs" "${base}/libvirt/images" "${base}/images"
    chmod 755 "${base}/logs"
    chmod 750 "${base}/.secret"
}

_server_file_modes_case_013() {
    WORK_DIR="$(mktemp -d)"
    local base="${WORK_DIR}/opt/sherpa" pool_group=sherpa
    getent group kvm >/dev/null 2>&1 && pool_group=kvm
    _older_release_folders
    _run_server_folders "$1" secure_server_folders
    echo "${output}"
    [ "$status" -eq 0 ]
    local dir
    for dir in "${base}" "${base}/bin" "${base}/libvirt"; do
        grep -qx -- "-d -o root -g root -m 755 ${dir}" "${WORK_DIR}/install.args"
        [ "$(stat -c '%a' "${dir}")" = "755" ]
    done
    for dir in config env logs db; do
        grep -qx -- "-d -o sherpa -g sherpa -m 750 ${base}/${dir}" "${WORK_DIR}/install.args"
        [ "$(stat -c '%a' "${base}/${dir}")" = "750" ]
    done
    grep -qx -- "-d -o sherpa -g sherpa -m 2750 ${base}/labs" "${WORK_DIR}/install.args"
    grep -qx -- "-d -o sherpa -g ${pool_group} -m 2750 ${base}/libvirt/images" "${WORK_DIR}/install.args"
    [ "$(stat -c '%a' "${base}/labs")" = "2750" ]
    [ "$(stat -c '%a' "${base}/libvirt/images")" = "2750" ]
    # Kept as they were: .secret and images. Made when missing: .certs, containers, bins, run.
    [ "$(stat -c '%a' "${base}/.secret")" = "750" ]
    [ "$(stat -c '%a' "${base}/images")" = "775" ]
    run grep -q -- "${base}/.secret$" "${WORK_DIR}/install.args"
    [ "$status" -ne 0 ]
    run grep -q -- "${base}/images$" "${WORK_DIR}/install.args"
    [ "$status" -ne 0 ]
    grep -qx -- "-d -o sherpa -g sherpa -m 700 ${base}/.certs" "${WORK_DIR}/install.args"
    [ "$(stat -c '%a' "${base}/.certs")" = "700" ]
    for dir in containers bins run; do
        grep -qx -- "-d -o sherpa -g sherpa -m 755 ${base}/${dir}" "${WORK_DIR}/install.args"
        [ "$(stat -c '%a' "${base}/${dir}")" = "755" ]
    done
    rm -rf "${WORK_DIR}"
}

_server_file_modes_case_016() {
    WORK_DIR="$(mktemp -d)"
    local base="${WORK_DIR}/opt/sherpa"
    _older_release_folders
    printf 'binary\n' > "${base}/bin/sherpad"
    printf 'disk\n' > "${base}/labs/disk.qcow2"
    mkdir "${base}/images/alpine_linux"
    printf 'image\n' > "${base}/images/alpine_linux/virtioa.qcow2"
    chmod 775 "${base}/bin/sherpad" "${base}/images/alpine_linux"
    chmod 660 "${base}/labs/disk.qcow2" "${base}/images/alpine_linux/virtioa.qcow2"
    local before
    before="$(cd "${base}" && stat -c '%n %a %u %g' bin/sherpad labs/disk.qcow2 images/alpine_linux images/alpine_linux/virtioa.qcow2)"
    _run_server_folders "$1" secure_server_folders
    [ "$status" -eq 0 ]
    [ "$(cd "${base}" && stat -c '%n %a %u %g' bin/sherpad labs/disk.qcow2 images/alpine_linux images/alpine_linux/virtioa.qcow2)" = "${before}" ]
    run grep -E -- "sherpad$|qcow2$|alpine_linux$" "${WORK_DIR}/install.args"
    [ "$status" -ne 0 ]
    [ ! -e "${WORK_DIR}/chown.args" ]
    rm -rf "${WORK_DIR}"
}

_server_file_modes_case_017() {
    WORK_DIR="$(mktemp -d)"
    local base="${WORK_DIR}/opt/sherpa"
    mkdir -p "${WORK_DIR}/elsewhere"
    chmod 777 "${WORK_DIR}/elsewhere"

    # The base folder is a link: nothing changes.
    mkdir -p "${WORK_DIR}/opt"
    ln -s "${WORK_DIR}/elsewhere" "${base}"
    _run_server_folders "$1" secure_server_folders
    [ "$status" -ne 0 ]
    [[ "$output" == *"${base} is a symlink"* ]]
    [ "$(stat -c '%a' "${WORK_DIR}/elsewhere")" = "777" ]
    [ ! -e "${WORK_DIR}/install.args" ]
    rm "${base}"

    # A folder above it is a link: nothing changes.
    mv "${WORK_DIR}/opt" "${WORK_DIR}/real-opt"
    ln -s "${WORK_DIR}/real-opt" "${WORK_DIR}/opt"
    _run_server_folders "$1" secure_server_folders
    [ "$status" -ne 0 ]
    [[ "$output" == *"${WORK_DIR}/opt is a symlink"* ]]
    [ ! -e "${WORK_DIR}/install.args" ]
    [ ! -e "${WORK_DIR}/real-opt/sherpa" ]
    rm "${WORK_DIR}/opt"
    mv "${WORK_DIR}/real-opt" "${WORK_DIR}/opt"

    # A folder sherpad writes is a link: the script stops, and the link's target keeps its mode.
    _older_release_folders
    rmdir "${base}/labs"
    ln -s "${WORK_DIR}/elsewhere" "${base}/labs"
    _run_server_folders "$1" secure_server_folders
    [ "$status" -ne 0 ]
    [[ "$output" == *"${base}/labs is a symlink"* ]]
    [ "$(stat -c '%a' "${WORK_DIR}/elsewhere")" = "777" ]
    run grep -q -- "labs$" "${WORK_DIR}/install.args"
    [ "$status" -ne 0 ]
    rm -rf "${WORK_DIR}"
}

_server_file_modes_case_018() {
    WORK_DIR="$(mktemp -d)"
    local base="${WORK_DIR}/opt/sherpa"
    mkdir -p "${base}/bin"
    printf 'not a binary\n' > "${WORK_DIR}/target"
    chmod 644 "${WORK_DIR}/target"
    ln -s "${WORK_DIR}/target" "${base}/bin/sherpad"
    printf 'new release\n' > "${WORK_DIR}/sherpad"
    _run_server_folders "$1" "install_binary '${WORK_DIR}/sherpad' sherpad"
    [ "$status" -eq 0 ]
    [ ! -L "${base}/bin/sherpad" ]
    [ "$(cat "${base}/bin/sherpad")" = "new release" ]
    [ "$(stat -c '%a' "${base}/bin/sherpad")" = "755" ]
    grep -qx -- "-o root -g root -m 755 ${WORK_DIR}/sherpad ${base}/bin/sherpad" "${WORK_DIR}/install.args"
    [ "$(cat "${WORK_DIR}/target")" = "not a binary" ]
    [ "$(stat -c '%a' "${WORK_DIR}/target")" = "644" ]
    rm -rf "${WORK_DIR}"
}

# Stubs of the commands that write a blank disk or its folder: each records the account it runs
# as, from the runuser stub, and its arguments, then does its work (the disk tools make an empty
# file of the size asked for).
# The stubs' own variables: they expand when a stub runs, not here.
# shellcheck disable=SC2016
_stub_blank_disk_writers() {
    local tool real
    for tool in mktemp chmod ln rm mkdir; do
        real="$(command -v "${tool}")"
        printf '#!/bin/bash\necho "${SHERPA_TEST_AS:-root} %s $*" >> "%s/writes"\nexec %s "$@"\n' \
            "${tool}" "${WORK_DIR}" "${real}" > "${MOCK_BIN}/${tool}"
    done
    printf '#!/bin/bash\necho "${SHERPA_TEST_AS:-root} qemu-img $*" >> "%s/writes"\n: > "$5"\n' \
        "${WORK_DIR}" > "${MOCK_BIN}/qemu-img"
    for tool in mkfs.fat mkfs.vfat mkfs.ext4; do
        printf '#!/bin/bash\necho "${SHERPA_TEST_AS:-root} %s $*" >> "%s/writes"\n' \
            "${tool}" "${WORK_DIR}" > "${MOCK_BIN}/${tool}"
    done
    chmod +x "${MOCK_BIN}"/*
}

_blank_disks_case_010() {
    WORK_DIR="$(mktemp -d)"
    _stub_blank_disk_writers
    _run_ensure_blank_disks "$1"
    rm -f "${MOCK_BIN}"/*
    echo "${output}"
    [ "$status" -eq 0 ]
    [ "$(grep -c "qemu-img create" "${WORK_DIR}/writes")" -eq 14 ]
    # Every write in the blank disk folder, and the folder's own, is sherpa's.
    run grep -v '^sherpa ' "${WORK_DIR}/writes"
    [ "$status" -ne 0 ]
    [ ! -e "${WORK_DIR}/chown.args" ]
    rm -rf "${WORK_DIR}"
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
# The server's folders and binaries, in both scripts
# ============================================================

@test "server_file_modes_013_the_scripts_make_the_base_folder_root_s_and_the_folders_sherpad_writes_sherpa_s" {
    _server_file_modes_case_013 "${SCRIPT}"
    _server_file_modes_case_013 "${UPDATE_SCRIPT}"
}

@test "server_file_modes_016_nothing_inside_those_folders_changes_owner_or_mode" {
    _server_file_modes_case_016 "${SCRIPT}"
    _server_file_modes_case_016 "${UPDATE_SCRIPT}"
}

@test "server_file_modes_017_a_symlink_at_or_above_one_of_those_folders_stops_the_script_and_its_target_is_unchanged" {
    _server_file_modes_case_017 "${SCRIPT}"
    _server_file_modes_case_017 "${UPDATE_SCRIPT}"
}

@test "server_file_modes_018_a_binary_is_installed_root_s_in_place_of_the_link_at_its_name" {
    _server_file_modes_case_018 "${SCRIPT}"
    _server_file_modes_case_018 "${UPDATE_SCRIPT}"
}

@test "blank_disks_010_the_blank_disks_and_their_folder_are_written_as_sherpa_never_as_root" {
    _blank_disks_case_010 "${SCRIPT}"
    _blank_disks_case_010 "${UPDATE_SCRIPT}"
}
