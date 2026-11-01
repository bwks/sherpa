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

# Run a function of the script with the docker stub first on PATH and the database file under
# WORK_DIR.
_run_function() {
    run bash -c "
        set +e
        PATH='${MOCK_BIN}:${PATH}'
        source '${STRIPPED_SCRIPT}'
        SHERPA_DB_FILE='${WORK_DIR}/sherpa.sqlite'
        SHERPA_ENV_FILE='${WORK_DIR}/sherpa.env'
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

@test "legacy_db_port: reads SHERPA_DB_PORT from the environment file" {
    printf 'SHERPA_SERVER_WS_PORT=3030\nSHERPA_DB_PORT=8123\n' > "${WORK_DIR}/sherpa.env"
    _run_function legacy_db_port
    [ "$status" -eq 0 ]
    [ "$output" = "8123" ]
}

@test "legacy_db_port: defaults to 8000 without the setting" {
    printf 'SHERPA_SERVER_WS_PORT=3030\n' > "${WORK_DIR}/sherpa.env"
    _run_function legacy_db_port
    [ "$status" -eq 0 ]
    [ "$output" = "8000" ]
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
