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
