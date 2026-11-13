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
