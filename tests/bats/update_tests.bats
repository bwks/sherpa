#!/usr/bin/env bats
#
# Test suite for scripts/sherpa_update.sh
#
# Run (no root required):
#   bats tests/bats/update_tests.bats

SCRIPT="${BATS_TEST_DIRNAME}/../../scripts/sherpa_update.sh"

# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------

setup() {
    MOCK_BIN="$(mktemp -d)"
    WORK_DIR="$(mktemp -d)"

    # A version of the script without the trailing `main "$@"` call, so functions can be sourced.
    STRIPPED_SCRIPT="$(mktemp)"
    grep -v '^main "\$@"' "${SCRIPT}" > "${STRIPPED_SCRIPT}"
}

teardown() {
    rm -rf "${MOCK_BIN:-}" "${WORK_DIR:-}"
    rm -f "${STRIPPED_SCRIPT:-}"
}

# A docker stub whose `ps` lists the given container names.
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

# Run a function of the script with the docker stub first on PATH and the database files under
# WORK_DIR.
_run_function() {
    run bash -c "
        set +e
        PATH='${MOCK_BIN}:${PATH}'
        source '${STRIPPED_SCRIPT}'
        SHERPA_DB_FILE='${WORK_DIR}/sherpa.sqlite'
        LEGACY_DB_DATA='${WORK_DIR}/sherpa.db'
        $1
    "
}

# ============================================================
# CLI & Argument Parsing
# ============================================================

@test "CLI: --help documents local release archives" {
    run bash "${SCRIPT}" --help
    [ "$status" -eq 0 ]
    [[ "$output" == *"SHERPA_ARTIFACT_DIR"* ]]
}

@test "CLI: local release archives require --version" {
    SHERPA_ARTIFACT_DIR="${WORK_DIR}" run bash "${SCRIPT}"
    [ "$status" -ne 0 ]
    [[ "$output" == *"Local artifacts require --version"* ]]
}

@test "CLI: local release archives must include sherpad" {
    SHERPA_ARTIFACT_DIR="${WORK_DIR}" run bash "${SCRIPT}" --version v0.5.0
    [ "$status" -ne 0 ]
    [[ "$output" == *"Required sherpad artifact not found"* ]]
}

# ============================================================
# Upgrade Check: pre-0.5.0 database
# ============================================================

@test "check_legacy_database: refuses with a sherpa-db container and no database file" {
    _stub_docker_with_containers other sherpa-db
    _run_function check_legacy_database
    [ "$status" -eq 1 ]
    [[ "$output" == *"older than 0.5.0"* ]]
    [[ "$output" == *"v0.5.12/scripts/sherpa_update.sh"* ]]
    [[ "$output" == *"--version v0.5.12"* ]]
}

@test "check_legacy_database: refuses with SurrealDB data and no database file" {
    _stub_docker_with_containers other
    mkdir "${WORK_DIR}/sherpa.db"
    _run_function check_legacy_database
    [ "$status" -eq 1 ]
    [[ "$output" == *"${WORK_DIR}/sherpa.db"* ]]
}

@test "check_legacy_database: refuses without Docker when SurrealDB data is left" {
    local no_docker_bin
    no_docker_bin="$(mktemp -d)"
    for tool in grep cat; do
        ln -s "$(command -v "${tool}")" "${no_docker_bin}/${tool}"
    done
    mkdir "${WORK_DIR}/sherpa.db"
    run bash -c "
        set +e
        PATH='${no_docker_bin}'
        source '${STRIPPED_SCRIPT}'
        SHERPA_DB_FILE='${WORK_DIR}/sherpa.sqlite'
        LEGACY_DB_DATA='${WORK_DIR}/sherpa.db'
        check_legacy_database
    "
    rm -rf "${no_docker_bin}"
    [ "$status" -eq 1 ]
}

@test "check_legacy_database: passes once the data has moved to the database file" {
    _stub_docker_with_containers sherpa-db
    mkdir "${WORK_DIR}/sherpa.db"
    touch "${WORK_DIR}/sherpa.sqlite"
    _run_function check_legacy_database
    [ "$status" -eq 0 ]
}

@test "check_legacy_database: passes on a server with neither SurrealDB container nor data" {
    _stub_docker_with_containers other sherpa-db-backup
    _run_function check_legacy_database
    [ "$status" -eq 0 ]
}

@test "update: refuses a pre-0.5.0 server before changing anything" {
    _stub_docker_with_containers sherpa-db
    printf '#!/bin/bash\nexit 0\n' > "${MOCK_BIN}/curl"
    # The stub's own variables: they expand when the stub runs, not here.
    # shellcheck disable=SC2016
    printf '#!/bin/bash\necho "$0 $*" >> "%s/changed"\nexit 0\n' "${WORK_DIR}" > "${MOCK_BIN}/systemctl"
    cp "${MOCK_BIN}/systemctl" "${MOCK_BIN}/pkill"
    mkdir -p "${WORK_DIR}/bin"
    printf '#!/bin/bash\necho "sherpad 0.4.18"\n' > "${WORK_DIR}/bin/sherpad"
    chmod +x "${MOCK_BIN}"/* "${WORK_DIR}/bin/sherpad"
    sed -e "s|^SHERPA_BASE_DIR=.*|SHERPA_BASE_DIR='${WORK_DIR}'|" \
        -e 's|^    check_root_privileges$|    :|' "${SCRIPT}" > "${WORK_DIR}/update.sh"
    PATH="${MOCK_BIN}:${PATH}" run bash "${WORK_DIR}/update.sh" --version v0.6.0
    [ "$status" -eq 1 ]
    [[ "$output" == *"older than 0.5.0"* ]]
    [ ! -e "${WORK_DIR}/changed" ]
    [ "$(cat "${WORK_DIR}/bin/sherpad")" = "$(printf '#!/bin/bash\necho "sherpad 0.4.18"')" ]
}

# ============================================================
# Server SSH Key
# ============================================================

# A runuser stub that records its arguments, prints what the given sherpad check-database
# prints, and exits with the given status. systemctl and sleep succeed at once, and systemctl
# records its arguments.
_stub_check_database() {
    cat > "${MOCK_BIN}/runuser" << EOF
#!/bin/bash
printf '%s\n' "\$@" > '${WORK_DIR}/runuser.args'
echo '$2' >&2
exit $1
EOF
    cat > "${MOCK_BIN}/systemctl" << EOF
#!/bin/bash
echo "\$@" >> '${WORK_DIR}/systemctl.args'
exit 0
EOF
    printf '#!/bin/bash\nexit 0\n' > "${MOCK_BIN}/sleep"
    chmod +x "${MOCK_BIN}/runuser" "${MOCK_BIN}/systemctl" "${MOCK_BIN}/sleep"
}

@test "check_new_sherpad_can_use_database: refuses, names the labs and starts the running release again" {
    _stub_check_database 1 "lab-0001 (Old, owned by alice)"
    touch "${WORK_DIR}/sherpa.sqlite"
    mkdir -p "${WORK_DIR}/new"
    _run_function "SHERPA_VERSION=v0.6.0; check_new_sherpad_can_use_database '${WORK_DIR}/new/sherpad'"
    [ "$status" -eq 1 ]
    [[ "$output" == *"lab-0001 (Old, owned by alice)"* ]]
    [[ "$output" == *"nothing was installed"* ]]
    grep -qx "restart sherpad" "${WORK_DIR}/systemctl.args"
}

@test "check_new_sherpad_can_use_database: runs the new sherpad's check-database as sherpa" {
    _stub_check_database 0 "This release can use the database"
    touch "${WORK_DIR}/sherpa.sqlite"
    mkdir -p "${WORK_DIR}/new"
    _run_function "SHERPA_VERSION=v0.6.0; check_new_sherpad_can_use_database '${WORK_DIR}/new/sherpad'"
    [ "$status" -eq 0 ]
    [ "$(tr '\n' ' ' < "${WORK_DIR}/runuser.args")" = "-u sherpa -- ${WORK_DIR}/new/sherpad check-database " ]
    # The service is left stopped for the install.
    [ ! -e "${WORK_DIR}/systemctl.args" ]
}

@test "check_new_sherpad_can_use_database: skips releases before v0.6.0 and servers without a database" {
    _stub_check_database 1 "unrecognized subcommand"
    touch "${WORK_DIR}/sherpa.sqlite"
    _run_function "SHERPA_VERSION=v0.5.35; check_new_sherpad_can_use_database '${WORK_DIR}/sherpad'"
    [ "$status" -eq 0 ]
    rm "${WORK_DIR}/sherpa.sqlite"
    _run_function "SHERPA_VERSION=v0.6.0; check_new_sherpad_can_use_database '${WORK_DIR}/sherpad'"
    [ "$status" -eq 0 ]
    [ ! -e "${WORK_DIR}/runuser.args" ]
}

@test "update_binaries: a sherpad that refuses the database is not installed" {
    _stub_check_database 1 "lab-0001 (Old, owned by alice)"
    touch "${WORK_DIR}/sherpa.sqlite"
    mkdir -p "${WORK_DIR}/bin" "${WORK_DIR}/artifacts" "${WORK_DIR}/build"
    echo running > "${WORK_DIR}/bin/sherpad"
    echo new > "${WORK_DIR}/build/sherpad"
    tar czf "${WORK_DIR}/artifacts/sherpad-x86_64-unknown-linux-gnu.tar.gz" -C "${WORK_DIR}/build" sherpad
    _run_function "SHERPA_VERSION=v0.6.0; FORCE=true; SHERPA_ARTIFACT_DIR='${WORK_DIR}/artifacts'; update_binaries"
    [ "$status" -eq 1 ]
    [ "$(cat "${WORK_DIR}/bin/sherpad")" = "running" ]
    [ ! -e "${WORK_DIR}/bin/sherpad.previous" ]
}

@test "remove_server_ssh_key: deletes the server's key pair and its empty folder" {
    mkdir -p "${WORK_DIR}/ssh"
    touch "${WORK_DIR}/ssh/sherpa_ssh_key" "${WORK_DIR}/ssh/sherpa_ssh_key.pub"
    _run_function "SERVER_SSH_KEY_DIR='${WORK_DIR}/ssh'; SHERPA_VERSION=v0.6.0; remove_server_ssh_key"
    [ "$status" -eq 0 ]
    [ ! -e "${WORK_DIR}/ssh" ]
    [[ "$output" == *"Removed the server's shared SSH key"* ]]
}

@test "remove_server_ssh_key: says nothing when there is no key and no folder" {
    _run_function "SERVER_SSH_KEY_DIR='${WORK_DIR}/ssh'; SHERPA_VERSION=v0.6.1; remove_server_ssh_key"
    [ "$status" -eq 0 ]
    [[ "$output" != *"Removed"* ]]
}

@test "remove_server_ssh_key: says nothing when the folder holds no key and is kept" {
    mkdir -p "${WORK_DIR}/ssh"
    touch "${WORK_DIR}/ssh/other"
    _run_function "SERVER_SSH_KEY_DIR='${WORK_DIR}/ssh'; SHERPA_VERSION=v0.6.1; remove_server_ssh_key"
    [ "$status" -eq 0 ]
    [ -e "${WORK_DIR}/ssh/other" ]
    [[ "$output" != *"Removed"* ]]
}

@test "remove_server_ssh_key: reports an empty folder it removed" {
    mkdir -p "${WORK_DIR}/ssh"
    _run_function "SERVER_SSH_KEY_DIR='${WORK_DIR}/ssh'; SHERPA_VERSION=v0.6.1; remove_server_ssh_key"
    [ "$status" -eq 0 ]
    [ ! -e "${WORK_DIR}/ssh" ]
    [[ "$output" == *"Removed the server's shared SSH key"* ]]
}

@test "remove_server_ssh_key: keeps other files in the folder, and the key for releases before v0.6.0" {
    mkdir -p "${WORK_DIR}/ssh"
    touch "${WORK_DIR}/ssh/sherpa_ssh_key" "${WORK_DIR}/ssh/sherpa_ssh_key.pub" "${WORK_DIR}/ssh/other"
    _run_function "SERVER_SSH_KEY_DIR='${WORK_DIR}/ssh'; SHERPA_VERSION=v0.5.35; remove_server_ssh_key"
    [ "$status" -eq 0 ]
    [ -e "${WORK_DIR}/ssh/sherpa_ssh_key" ]
    _run_function "SERVER_SSH_KEY_DIR='${WORK_DIR}/ssh'; SHERPA_VERSION=v0.6.0; remove_server_ssh_key"
    [ "$status" -eq 0 ]
    [ ! -e "${WORK_DIR}/ssh/sherpa_ssh_key" ]
    [ ! -e "${WORK_DIR}/ssh/sherpa_ssh_key.pub" ]
    [ -e "${WORK_DIR}/ssh/other" ]
}

# ============================================================
# Config, env and log files: owner and mode
# ============================================================

# A chown stub that records its arguments, so the owner can be checked without root.
_stub_chown() {
    printf '#!/bin/bash\necho "$@" >> "%s/chown.args"\nexit 0\n' "${WORK_DIR}" > "${MOCK_BIN}/chown"
    chmod +x "${MOCK_BIN}/chown"
}

# An install made by an older release: sherpa.toml at the umask of 'sherpad init', the env files
# 0640, the log and its rotated copy world-readable, the folders 0775 and 0755.
_old_install_layout() {
    mkdir -p "${WORK_DIR}/config" "${WORK_DIR}/env" "${WORK_DIR}/logs"
    printf 'server_ipv4 = "0.0.0.0"\n[ztp_server]\npassword = "kept"\n' > "${WORK_DIR}/config/sherpa.toml"
    printf 'SHERPA_SERVER_IPV4=0.0.0.0\nOTEL_EXPORTER_OTLP_HEADERS=authorization=kept\n' > "${WORK_DIR}/env/sherpa.env"
    printf '# example\n' > "${WORK_DIR}/env/sherpa.env.example"
    printf 'log line\n' > "${WORK_DIR}/logs/sherpad.log"
    printf 'old log line\n' > "${WORK_DIR}/logs/sherpad.log.1"
    chmod 775 "${WORK_DIR}/config"
    chmod 664 "${WORK_DIR}/config/sherpa.toml"
    chmod 750 "${WORK_DIR}/env"
    chmod 640 "${WORK_DIR}/env/sherpa.env" "${WORK_DIR}/env/sherpa.env.example"
    chmod 755 "${WORK_DIR}/logs"
    chmod 644 "${WORK_DIR}/logs/sherpad.log" "${WORK_DIR}/logs/sherpad.log.1"
}

_run_secure_config_files() {
    _run_function "SHERPA_CONFIG_DIR='${WORK_DIR}/config'; SHERPA_ENV_DIR='${WORK_DIR}/env'; SHERPA_LOG_DIR='${WORK_DIR}/logs'; secure_config_files"
}

@test "secure_config_files: gives each file and folder sherpa's ownership and its mode" {
    _stub_chown
    _old_install_layout
    _run_secure_config_files
    [ "$status" -eq 0 ]
    [ "$(stat -c '%a' "${WORK_DIR}/config")" = "750" ]
    [ "$(stat -c '%a' "${WORK_DIR}/config/sherpa.toml")" = "600" ]
    [ "$(stat -c '%a' "${WORK_DIR}/env")" = "750" ]
    [ "$(stat -c '%a' "${WORK_DIR}/env/sherpa.env")" = "600" ]
    [ "$(stat -c '%a' "${WORK_DIR}/env/sherpa.env.example")" = "600" ]
    [ "$(stat -c '%a' "${WORK_DIR}/logs")" = "750" ]
    [ "$(stat -c '%a' "${WORK_DIR}/logs/sherpad.log")" = "640" ]
    [ "$(stat -c '%a' "${WORK_DIR}/logs/sherpad.log.1")" = "640" ]
    for path in config config/sherpa.toml env env/sherpa.env env/sherpa.env.example \
        logs logs/sherpad.log logs/sherpad.log.1; do
        grep -qx "sherpa:sherpa ${WORK_DIR}/${path}" "${WORK_DIR}/chown.args"
    done
}

@test "secure_config_files: leaves the contents unchanged" {
    _stub_chown
    _old_install_layout
    local before after
    before="$(cd "${WORK_DIR}" && sha256sum config/sherpa.toml env/sherpa.env env/sherpa.env.example logs/sherpad.log*)"
    _run_secure_config_files
    [ "$status" -eq 0 ]
    after="$(cd "${WORK_DIR}" && sha256sum config/sherpa.toml env/sherpa.env env/sherpa.env.example logs/sherpad.log*)"
    [ "${before}" = "${after}" ]
}

@test "secure_config_files: a missing file or folder is not an error" {
    _stub_chown
    mkdir -p "${WORK_DIR}/config"
    _run_secure_config_files
    [ "$status" -eq 0 ]
    [ "$(stat -c '%a' "${WORK_DIR}/config")" = "750" ]
    [ ! -e "${WORK_DIR}/env" ]
    [ ! -e "${WORK_DIR}/logs" ]
    [ "$(cat "${WORK_DIR}/chown.args")" = "sherpa:sherpa ${WORK_DIR}/config" ]
}

@test "secure_config_files: leaves a symlink and its target alone" {
    _stub_chown
    mkdir -p "${WORK_DIR}/config"
    touch "${WORK_DIR}/target"
    chmod 644 "${WORK_DIR}/target"
    ln -s "${WORK_DIR}/target" "${WORK_DIR}/config/sherpa.toml"
    _run_secure_config_files
    [ "$status" -eq 0 ]
    [[ "$output" == *"is a symlink"* ]]
    [ "$(stat -c '%a' "${WORK_DIR}/target")" = "644" ]
    run grep -q "sherpa.toml" "${WORK_DIR}/chown.args"
    [ "$status" -ne 0 ]
}

@test "update: restricts the config and env files when there is nothing to update, too" {
    grep -A6 'needs_update" = false' "${SCRIPT}" | grep -qx '        secure_config_files'
    grep -B2 -A2 '^    secure_database_storage$' "${SCRIPT}" | grep -qx '    secure_config_files'
}

# ============================================================
# Blank Disks
# ============================================================

# An apt-get stub that records its arguments.
_stub_apt_get() {
    printf '#!/bin/bash\necho "$@" >> "%s/apt-get.args"\nexit 0\n' "${WORK_DIR}" > "${MOCK_BIN}/apt-get"
    chmod +x "${MOCK_BIN}/apt-get"
}

@test "update: fills in missing blank disks before sherpad stops, and when there is nothing to update" {
    grep -A3 'needs_update" = false' "${SCRIPT}" | grep -qx '        ensure_blank_disks'
    grep -A3 'needs_update" = false' "${SCRIPT}" | grep -qx '        install_blank_disk_tools'
    sed -n '/^main()/,/^}/p' "${SCRIPT}" | grep -B4 '^    stop_sherpad$' | grep -qx '    ensure_blank_disks'
}

@test "install_blank_disk_tools: installs nothing when every tool is there" {
    _stub_apt_get
    _run_function "BLANK_DISK_TOOLS=(bash); install_blank_disk_tools"
    [ "$status" -eq 0 ]
    [ ! -e "${WORK_DIR}/apt-get.args" ]
}

@test "install_blank_disk_tools: installs the packages when a tool is missing" {
    _stub_apt_get
    _run_function "BLANK_DISK_TOOLS=(bash no-such-mkfs); install_blank_disk_tools"
    [ "$status" -eq 0 ]
    grep -qx "install -y -qq qemu-utils dosfstools e2fsprogs" "${WORK_DIR}/apt-get.args"
}
