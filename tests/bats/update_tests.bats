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

# A docker stub for the migration: sherpa-db exists and runs, and publishes SurrealDB on the given
# host port, or on none when it is empty.
_stub_docker_for_migration() {
    cat > "${MOCK_BIN}/docker" << EOF
#!/bin/bash
case "\$1" in
    ps) echo sherpa-db ;;
    inspect) echo '$1' ;;
esac
exit 0
EOF
    chmod +x "${MOCK_BIN}/docker"
}

# A systemd-run stub that records its arguments and standard input, then exits with the given
# status. With a non-zero status it first leaves a database file behind, as a failed import would.
_stub_systemd_run() {
    cat > "${MOCK_BIN}/systemd-run" << EOF
#!/bin/bash
printf '%s\n' "\$@" > '${WORK_DIR}/systemd-run.args'
readlink /proc/self/fd/0 > '${WORK_DIR}/systemd-run.stdin'
[ $1 -eq 0 ] || touch '${WORK_DIR}/sherpa.sqlite'
exit $1
EOF
    chmod +x "${MOCK_BIN}/systemd-run"
    printf '#!/bin/bash\nexit 0\n' > "${MOCK_BIN}/curl"
    printf '#!/bin/bash\nexit 0\n' > "${MOCK_BIN}/systemctl"
    chmod +x "${MOCK_BIN}/curl" "${MOCK_BIN}/systemctl"
}

# Run a function of the script with the docker stub first on PATH and the database file under
# WORK_DIR.
_run_function() {
    run bash -c "
        set +e
        PATH='${MOCK_BIN}:${PATH}'
        source '${STRIPPED_SCRIPT}'
        SHERPA_DB_FILE='${WORK_DIR}/sherpa.sqlite'
        SHERPA_ENV_FILE='${WORK_DIR}/sherpa.env'
        SHERPA_BIN_DIR='${WORK_DIR}/bin'
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
# Database Migration
# ============================================================

@test "needs_database_migration: true with a sherpa-db container and no database file" {
    _stub_docker_with_containers other sherpa-db
    _run_function needs_database_migration
    [ "$status" -eq 0 ]
}

@test "needs_database_migration: false once the database file exists" {
    _stub_docker_with_containers sherpa-db
    touch "${WORK_DIR}/sherpa.sqlite"
    _run_function needs_database_migration
    [ "$status" -ne 0 ]
}

@test "needs_database_migration: false without a sherpa-db container" {
    _stub_docker_with_containers other sherpa-db-backup
    _run_function needs_database_migration
    [ "$status" -ne 0 ]
}

@test "version_newer_than: compares release versions, not text" {
    _run_function 'version_newer_than v0.5.13 v0.5.12 && version_newer_than v0.10.0 v0.5.12 && echo newer'
    [ "$output" = "newer" ]
    _run_function 'version_newer_than v0.5.12 v0.5.12 || version_newer_than 0.5.9 v0.5.12 || echo not-newer'
    [ "$output" = "not-newer" ]
}

@test "check_database_can_move: refuses a release that cannot move the SurrealDB data" {
    _stub_docker_with_containers sherpa-db
    _run_function 'SHERPA_VERSION=v0.5.13; check_database_can_move'
    [ "$status" -ne 0 ]
    [[ "$output" == *"--version v0.5.12"* ]]
}

@test "check_database_can_move: allows a release that still moves the data" {
    _stub_docker_with_containers sherpa-db
    _run_function 'SHERPA_VERSION=v0.5.12; check_database_can_move'
    [ "$status" -eq 0 ]
}

@test "check_database_can_move: allows any release once the data has moved" {
    _stub_docker_with_containers sherpa-db
    touch "${WORK_DIR}/sherpa.sqlite"
    _run_function 'SHERPA_VERSION=v0.5.13; check_database_can_move'
    [ "$status" -eq 0 ]
}

@test "legacy_db_port: reads the port sherpa-db publishes from Docker" {
    _stub_docker_for_migration 8123
    _run_function legacy_db_port
    [ "$status" -eq 0 ]
    [ "$output" = "8123" ]
}

@test "legacy_db_port: defaults to 8000 when Docker reports no published port" {
    _stub_docker_for_migration ""
    _run_function legacy_db_port
    [ "$status" -eq 0 ]
    [ "$output" = "8000" ]
}

@test "migrate_database: runs migrate-db through systemd with the service's env file" {
    _stub_docker_for_migration 8123
    _stub_systemd_run 0
    _run_function migrate_database
    [ "$status" -eq 0 ]
    grep -qx -- "--uid=sherpa" "${WORK_DIR}/systemd-run.args"
    grep -qx -- "--property=EnvironmentFile=-${WORK_DIR}/sherpa.env" "${WORK_DIR}/systemd-run.args"
    grep -qx -- "--wait" "${WORK_DIR}/systemd-run.args"
    grep -qx -- "--pipe" "${WORK_DIR}/systemd-run.args"
    [ "$(tail -n 4 "${WORK_DIR}/systemd-run.args" | tr '\n' ' ')" = "${WORK_DIR}/bin/sherpad migrate-db --db-port 8123 " ]
    # The password comes only from the env file, never from the update script's environment.
    ! grep -q -- "--db-pass" "${WORK_DIR}/systemd-run.args"
    # A script piped into bash must not be read by migrate-db.
    [ "$(cat "${WORK_DIR}/systemd-run.stdin")" = "/dev/null" ]
}

@test "migrate_database: a failed move removes the new database and restores the previous sherpad" {
    _stub_docker_for_migration 8000
    _stub_systemd_run 1
    mkdir -p "${WORK_DIR}/bin"
    echo new > "${WORK_DIR}/bin/sherpad"
    echo previous > "${WORK_DIR}/bin/sherpad.previous"
    chmod +x "${WORK_DIR}/bin/sherpad" "${WORK_DIR}/bin/sherpad.previous"
    _run_function migrate_database
    [ "$status" -ne 0 ]
    [[ "$output" == *"Database move failed"* ]]
    [ ! -e "${WORK_DIR}/sherpa.sqlite" ]
    [ "$(cat "${WORK_DIR}/bin/sherpad")" = "previous" ]
}

@test "remove_failed_database: removes the database file and its WAL and SHM files" {
    touch "${WORK_DIR}/sherpa.sqlite" "${WORK_DIR}/sherpa.sqlite-wal" "${WORK_DIR}/sherpa.sqlite-shm"
    touch "${WORK_DIR}/sherpa.db"
    _run_function remove_failed_database
    [ "$status" -eq 0 ]
    [ ! -e "${WORK_DIR}/sherpa.sqlite" ]
    [ ! -e "${WORK_DIR}/sherpa.sqlite-wal" ]
    [ ! -e "${WORK_DIR}/sherpa.sqlite-shm" ]
    # The old SurrealDB data is never touched.
    [ -e "${WORK_DIR}/sherpa.db" ]
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
