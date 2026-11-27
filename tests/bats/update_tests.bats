#!/usr/bin/env bats
#
# Test suite for scripts/sherpa_update.sh. Each test is a case of tests/specs/install-and-update.md
# and is named by its ID.
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

@test "update_script_016_local_archives_need_version_and_the_sherpad_archive" {
    SHERPA_ARTIFACT_DIR="${WORK_DIR}" run bash "${SCRIPT}"
    [ "$status" -ne 0 ]
    [[ "$output" == *"Local artifacts require --version"* ]]
    SHERPA_ARTIFACT_DIR="${WORK_DIR}" run bash "${SCRIPT}" --version v0.5.0
    [ "$status" -ne 0 ]
    [[ "$output" == *"Required sherpad artifact not found"* ]]
}

@test "update_script_017_help_prints_the_usage_and_an_unknown_option_is_an_error" {
    run bash "${SCRIPT}" --help
    [ "$status" -eq 0 ]
    [[ "$output" == *"Usage:"* ]]
    [[ "$output" == *"SHERPA_ARTIFACT_DIR"* ]]
    run bash "${SCRIPT}" --bogus
    [ "$status" -eq 1 ]
    [[ "$output" == *"Unknown option: --bogus"* ]]
}

@test "update_script_028_version_with_no_value_says_so_and_shows_the_usage" {
    run bash "${SCRIPT}" --version
    [ "$status" -eq 1 ]
    [[ "$output" == *"--version requires a version"* ]]
    [[ "$output" == *"Usage:"* ]]
    run bash "${SCRIPT}" --version --force
    [ "$status" -eq 1 ]
    [[ "$output" == *"--version requires a version"* ]]
    run bash "${SCRIPT}" --version ""
    [ "$status" -eq 1 ]
    [[ "$output" == *"--version requires a version"* ]]
}

# ============================================================
# Upgrade Check: pre-0.5.0 database
# ============================================================

@test "update_script_004_a_server_with_only_surrealdb_data_is_refused_with_the_release_to_update_to" {
    _stub_docker_with_containers other sherpa-db
    _run_function check_legacy_database
    [ "$status" -eq 1 ]
    [[ "$output" == *"older than 0.5.0"* ]]
    [[ "$output" == *"v0.5.12/scripts/sherpa_update.sh"* ]]
    [[ "$output" == *"--version v0.5.12"* ]]

    # The data folder the container kept, with no container.
    _stub_docker_with_containers other
    mkdir "${WORK_DIR}/sherpa.db"
    _run_function check_legacy_database
    [ "$status" -eq 1 ]
    [[ "$output" == *"${WORK_DIR}/sherpa.db"* ]]
    [[ "$output" == *"--version v0.5.12"* ]]

    # The data folder, with no Docker.
    local no_docker_bin
    no_docker_bin="$(mktemp -d)"
    for tool in grep cat; do
        ln -s "$(command -v "${tool}")" "${no_docker_bin}/${tool}"
    done
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
    [[ "$output" == *"--version v0.5.12"* ]]
}

@test "update_script_005_a_server_with_a_database_file_or_no_surrealdb_passes" {
    _stub_docker_with_containers sherpa-db
    mkdir "${WORK_DIR}/sherpa.db"
    touch "${WORK_DIR}/sherpa.sqlite"
    _run_function check_legacy_database
    [ "$status" -eq 0 ]

    rm -rf "${WORK_DIR}/sherpa.db" "${WORK_DIR}/sherpa.sqlite"
    _stub_docker_with_containers other sherpa-db-backup
    _run_function check_legacy_database
    [ "$status" -eq 0 ]
}

@test "update_script_006_refusing_such_a_server_stops_nothing_and_keeps_the_binary" {
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
# The new release's database check and the server SSH key
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

@test "update_script_007_a_release_that_refuses_the_database_is_not_installed_and_the_old_one_restarts" {
    _stub_check_database 1 "lab-0001 (Old, owned by alice)"
    touch "${WORK_DIR}/sherpa.sqlite"
    mkdir -p "${WORK_DIR}/new"
    _run_function "SHERPA_VERSION=v0.6.0; check_new_sherpad_can_use_database '${WORK_DIR}/new/sherpad'"
    [ "$status" -eq 1 ]
    [[ "$output" == *"lab-0001 (Old, owned by alice)"* ]]
    [[ "$output" == *"nothing was installed"* ]]
    grep -qx "restart sherpad" "${WORK_DIR}/systemctl.args"
}

@test "update_script_009_the_new_sherpads_check_runs_as_sherpa_and_is_skipped_without_a_database" {
    _stub_check_database 0 "This release can use the database"
    touch "${WORK_DIR}/sherpa.sqlite"
    mkdir -p "${WORK_DIR}/new"
    _run_function "SHERPA_VERSION=v0.6.0; check_new_sherpad_can_use_database '${WORK_DIR}/new/sherpad'"
    [ "$status" -eq 0 ]
    [ "$(tr '\n' ' ' < "${WORK_DIR}/runuser.args")" = "-u sherpa -- ${WORK_DIR}/new/sherpad check-database " ]
    # The service is left stopped for the install.
    [ ! -e "${WORK_DIR}/systemctl.args" ]

    rm "${WORK_DIR}/sherpa.sqlite" "${WORK_DIR}/runuser.args"
    _stub_check_database 1 "unrecognized subcommand"
    _run_function "SHERPA_VERSION=v0.6.0; check_new_sherpad_can_use_database '${WORK_DIR}/new/sherpad'"
    [ "$status" -eq 0 ]
    [ ! -e "${WORK_DIR}/runuser.args" ]
}

@test "update_script_010_a_release_before_0_6_0_skips_the_check_and_keeps_the_servers_key" {
    _stub_check_database 1 "unrecognized subcommand"
    touch "${WORK_DIR}/sherpa.sqlite"
    _run_function "SHERPA_VERSION=v0.5.35; check_new_sherpad_can_use_database '${WORK_DIR}/sherpad'"
    [ "$status" -eq 0 ]
    [ ! -e "${WORK_DIR}/runuser.args" ]

    mkdir -p "${WORK_DIR}/ssh"
    touch "${WORK_DIR}/ssh/sherpa_ssh_key" "${WORK_DIR}/ssh/sherpa_ssh_key.pub"
    _run_function "SERVER_SSH_KEY_DIR='${WORK_DIR}/ssh'; SHERPA_VERSION=v0.5.35; remove_server_ssh_key"
    [ "$status" -eq 0 ]
    [ -e "${WORK_DIR}/ssh/sherpa_ssh_key" ]
    [ -e "${WORK_DIR}/ssh/sherpa_ssh_key.pub" ]
}

@test "update_script_008_after_that_refusal_the_installed_sherpad_is_the_one_that_was_there" {
    _stub_check_database 1 "lab-0001 (Old, owned by alice)"
    touch "${WORK_DIR}/sherpa.sqlite"
    mkdir -p "${WORK_DIR}/bin" "${WORK_DIR}/artifacts" "${WORK_DIR}/build"
    echo running > "${WORK_DIR}/bin/sherpad"
    echo new > "${WORK_DIR}/build/sherpad"
    tar czf "${WORK_DIR}/artifacts/sherpad-x86_64-unknown-linux-gnu.tar.gz" -C "${WORK_DIR}/build" sherpad
    _run_function "SHERPA_BASE_DIR='${WORK_DIR}'; SHERPA_BIN_DIR='${WORK_DIR}/bin'; SHERPA_VERSION=v0.6.0; FORCE=true; SHERPA_ARTIFACT_DIR='${WORK_DIR}/artifacts'; download_binaries; install_binaries"
    [ "$status" -eq 1 ]
    [ "$(cat "${WORK_DIR}/bin/sherpad")" = "running" ]
    [ ! -e "${WORK_DIR}/bin/sherpad.previous" ]
}

@test "update_script_011_the_servers_key_pair_is_deleted_and_other_files_are_kept" {
    # The key pair, and its folder once that leaves it empty.
    mkdir -p "${WORK_DIR}/ssh"
    touch "${WORK_DIR}/ssh/sherpa_ssh_key" "${WORK_DIR}/ssh/sherpa_ssh_key.pub"
    _run_function "SERVER_SSH_KEY_DIR='${WORK_DIR}/ssh'; SHERPA_VERSION=v0.6.0; remove_server_ssh_key"
    [ "$status" -eq 0 ]
    [ ! -e "${WORK_DIR}/ssh" ]
    [[ "$output" == *"Removed the server's shared SSH key"* ]]

    # The key pair beside another file: the file and the folder stay.
    mkdir -p "${WORK_DIR}/ssh"
    touch "${WORK_DIR}/ssh/sherpa_ssh_key" "${WORK_DIR}/ssh/sherpa_ssh_key.pub" "${WORK_DIR}/ssh/other"
    _run_function "SERVER_SSH_KEY_DIR='${WORK_DIR}/ssh'; SHERPA_VERSION=v0.6.0; remove_server_ssh_key"
    [ "$status" -eq 0 ]
    [ ! -e "${WORK_DIR}/ssh/sherpa_ssh_key" ]
    [ ! -e "${WORK_DIR}/ssh/sherpa_ssh_key.pub" ]
    [ -e "${WORK_DIR}/ssh/other" ]

    # A folder with no key is kept, and nothing is said.
    _run_function "SERVER_SSH_KEY_DIR='${WORK_DIR}/ssh'; SHERPA_VERSION=v0.6.1; remove_server_ssh_key"
    [ "$status" -eq 0 ]
    [ -e "${WORK_DIR}/ssh/other" ]
    [[ "$output" != *"Removed"* ]]

    # No key and no folder: nothing is said.
    rm -rf "${WORK_DIR}/ssh"
    _run_function "SERVER_SSH_KEY_DIR='${WORK_DIR}/ssh'; SHERPA_VERSION=v0.6.1; remove_server_ssh_key"
    [ "$status" -eq 0 ]
    [[ "$output" != *"Removed"* ]]

    # An empty folder is removed, and that is reported.
    mkdir -p "${WORK_DIR}/ssh"
    _run_function "SERVER_SSH_KEY_DIR='${WORK_DIR}/ssh'; SHERPA_VERSION=v0.6.1; remove_server_ssh_key"
    [ "$status" -eq 0 ]
    [ ! -e "${WORK_DIR}/ssh" ]
    [[ "$output" == *"Removed the server's shared SSH key"* ]]
}

# ============================================================
# Config, env and log files: owner and mode
# ============================================================

# A chown stub that records its arguments, so the owner can be checked without root, and a
# runuser stub that runs its command as this user and records the account it was asked for.
_stub_chown() {
    printf '#!/bin/bash\necho "$@" >> "%s/chown.args"\nexit 0\n' "${WORK_DIR}" > "${MOCK_BIN}/chown"
    # The stub's own variables: they expand when the stub runs, not here.
    # shellcheck disable=SC2016
    printf '#!/bin/bash\necho "$2 ${*:4}" >> "%s/runuser.args"\nshift 3\nexec "$@"\n' "${WORK_DIR}" > "${MOCK_BIN}/runuser"
    chmod +x "${MOCK_BIN}/chown" "${MOCK_BIN}/runuser"
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

@test "server_file_modes_010_the_update_gives_the_config_env_and_log_files_their_modes_and_owner" {
    _stub_chown
    _old_install_layout
    local before after
    before="$(cd "${WORK_DIR}" && sha256sum config/sherpa.toml env/sherpa.env env/sherpa.env.example logs/sherpad.log*)"
    _run_secure_config_files
    [ "$status" -eq 0 ]
    [ "$(stat -c '%a' "${WORK_DIR}/config/sherpa.toml")" = "600" ]
    [ "$(stat -c '%a' "${WORK_DIR}/env/sherpa.env")" = "600" ]
    [ "$(stat -c '%a' "${WORK_DIR}/env/sherpa.env.example")" = "600" ]
    [ "$(stat -c '%a' "${WORK_DIR}/logs/sherpad.log")" = "640" ]
    [ "$(stat -c '%a' "${WORK_DIR}/logs/sherpad.log.1")" = "640" ]
    for path in config/sherpa.toml env/sherpa.env env/sherpa.env.example \
        logs/sherpad.log logs/sherpad.log.1; do
        grep -qx -- "-h sherpa:sherpa ${WORK_DIR}/${path}" "${WORK_DIR}/chown.args"
        # The mode is set as sherpa, once the file is sherpa's.
        grep -qx "sherpa chmod [0-9]* ${WORK_DIR}/${path}" "${WORK_DIR}/runuser.args"
    done
    after="$(cd "${WORK_DIR}" && sha256sum config/sherpa.toml env/sherpa.env env/sherpa.env.example logs/sherpad.log*)"
    [ "${before}" = "${after}" ]
}

@test "server_file_modes_011_the_update_skips_a_missing_file_and_creates_none" {
    _stub_chown
    mkdir -p "${WORK_DIR}/config" "${WORK_DIR}/env" "${WORK_DIR}/logs"
    _run_secure_config_files
    [ "$status" -eq 0 ]
    [ -z "$(find "${WORK_DIR}/config" "${WORK_DIR}/env" "${WORK_DIR}/logs" -mindepth 1)" ]
    [ ! -e "${WORK_DIR}/chown.args" ]
}

@test "server_file_modes_012_the_update_leaves_a_symlink_and_its_target_and_says_so" {
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

    # A link above it: the folder of the file.
    rm "${WORK_DIR}/config/sherpa.toml"
    mv "${WORK_DIR}/config" "${WORK_DIR}/real-config"
    touch "${WORK_DIR}/real-config/sherpa.toml"
    chmod 644 "${WORK_DIR}/real-config/sherpa.toml"
    ln -s "${WORK_DIR}/real-config" "${WORK_DIR}/config"
    _run_secure_config_files
    [ "$status" -eq 0 ]
    [[ "$output" == *"${WORK_DIR}/config is a symlink"* ]]
    [ "$(stat -c '%a' "${WORK_DIR}/real-config/sherpa.toml")" = "644" ]
    run grep -q "sherpa.toml" "${WORK_DIR}/chown.args"
    [ "$status" -ne 0 ]
}

# ============================================================
# The installed binaries are run only when they are root's alone
# ============================================================

@test "update_script_019_an_installed_binary_is_run_for_its_version_only_when_only_root_can_have_written_it" {
    # A binary of this account's, as sherpa's is on a server an older release installed.
    mkdir -p "${WORK_DIR}/bin"
    printf '#!/bin/bash\ntouch "%s/ran"\necho "sherpad 0.7.1"\n' "${WORK_DIR}" > "${WORK_DIR}/bin/sherpad"
    chmod 755 "${WORK_DIR}/bin/sherpad"
    _run_function "SHERPA_BIN_DIR='${WORK_DIR}/bin'; SHERPA_VERSION=v0.7.1; check_binary_needs_update sherpad"
    [ "$status" -eq 0 ]
    [[ "$output" == *"is not run, and will be replaced"* ]]
    [ ! -e "${WORK_DIR}/ran" ]

    # A link to a binary of root's.
    rm "${WORK_DIR}/bin/sherpad"
    ln -s "$(type -P true)" "${WORK_DIR}/bin/sherpad"
    _run_function "trusted_binary_version '${WORK_DIR}/bin/sherpad'"
    [ "$status" -eq 0 ]
    [ -z "$output" ]

    # A binary of root's that only root can write is run.
    _run_function "trusted_binary_version '$(type -P true)'"
    [ "$status" -eq 0 ]
    [ -n "$output" ]
}

@test "update_script_020_a_link_at_or_above_the_servers_ssh_key_folder_removes_nothing" {
    mkdir -p "${WORK_DIR}/elsewhere"
    touch "${WORK_DIR}/elsewhere/sherpa_ssh_key" "${WORK_DIR}/elsewhere/sherpa_ssh_key.pub"
    ln -s "${WORK_DIR}/elsewhere" "${WORK_DIR}/ssh"
    _run_function "SERVER_SSH_KEY_DIR='${WORK_DIR}/ssh'; SHERPA_VERSION=v0.7.1; remove_server_ssh_key"
    [ "$status" -eq 0 ]
    [[ "$output" == *"${WORK_DIR}/ssh is a symlink"* ]]
    [ -e "${WORK_DIR}/elsewhere/sherpa_ssh_key" ]
    [ -e "${WORK_DIR}/elsewhere/sherpa_ssh_key.pub" ]
    [ -L "${WORK_DIR}/ssh" ]
}

# ============================================================
# Blank Disks
# ============================================================

# An apt-get stub that records its arguments.
_stub_apt_get() {
    printf '#!/bin/bash\necho "$@" >> "%s/apt-get.args"\nexit 0\n' "${WORK_DIR}" > "${MOCK_BIN}/apt-get"
    chmod +x "${MOCK_BIN}/apt-get"
}

@test "blank_disks_007_the_update_installs_the_tools_packages_only_when_a_tool_is_missing" {
    _stub_apt_get
    _run_function "BLANK_DISK_TOOLS=(bash); install_blank_disk_tools"
    [ "$status" -eq 0 ]
    [ ! -e "${WORK_DIR}/apt-get.args" ]

    _run_function "BLANK_DISK_TOOLS=(bash no-such-mkfs); install_blank_disk_tools"
    [ "$status" -eq 0 ]
    grep -qx "install -y -qq qemu-utils dosfstools e2fsprogs" "${WORK_DIR}/apt-get.args"
}

@test "sandbox_054_the_update_installs_bzip2_only_when_bunzip2_is_missing" {
    _stub_apt_get
    _run_function "SANDBOX_TOOLS=(bash); install_sandbox_tools"
    [ "$status" -eq 0 ]
    [ ! -e "${WORK_DIR}/apt-get.args" ]

    _run_function "SANDBOX_TOOLS=(bash no-such-bunzip2); install_sandbox_tools"
    [ "$status" -eq 0 ]
    grep -qx "install -y -qq bzip2" "${WORK_DIR}/apt-get.args"
}

# ============================================================
# Download first, back up, and go back on a failure
# ============================================================

# Stubs for an update run as a user that is not root, recording in WORK_DIR: systemctl records
# its arguments and, for `is-active`, fails while the installed sherpad is the new release (a
# file with "# new" in it); runuser runs its command as this user and records the account;
# install installs without the owner and group; chown, pgrep, pkill and sleep do nothing.
_stub_update_commands() {
    cat > "${MOCK_BIN}/systemctl" << EOF
#!/bin/bash
echo "\$@" >> '${WORK_DIR}/systemctl.args'
if [ "\$1" = "is-active" ] && grep -q '# new' '${WORK_DIR}/bin/sherpad' 2>/dev/null; then
    exit 3
fi
exit 0
EOF
    cat > "${MOCK_BIN}/runuser" << EOF
#!/bin/bash
echo "\$2 \${*:4}" >> '${WORK_DIR}/runuser.args'
shift 3
exec "\$@"
EOF
    cat > "${MOCK_BIN}/install" << EOF
#!/bin/bash
echo "\$*" >> '${WORK_DIR}/install.args'
args=()
while [ \$# -gt 0 ]; do
    case "\$1" in
        -o|-g) shift 2 ;;
        *) args+=("\$1"); shift ;;
    esac
done
exec $(command -v install) "\${args[@]}"
EOF
    printf '#!/bin/bash\nexit 1\n' > "${MOCK_BIN}/pgrep"
    for tool in chown pkill sleep; do
        printf '#!/bin/bash\nexit 0\n' > "${MOCK_BIN}/${tool}"
    done
    chmod +x "${MOCK_BIN}"/*
}

# A server with a database, its WAL file and its key under WORK_DIR, the installed release's
# sherpad in WORK_DIR/bin and the new release's in WORK_DIR/new.
_server_with_database() {
    mkdir -p "${WORK_DIR}/bin" "${WORK_DIR}/new"
    printf '#!/bin/bash\n# old\nexit 0\n' > "${WORK_DIR}/bin/sherpad"
    printf '#!/bin/bash\n# new\nexit 0\n' > "${WORK_DIR}/new/sherpad"
    chmod 755 "${WORK_DIR}/bin/sherpad" "${WORK_DIR}/new/sherpad"
    printf 'records before the update\n' > "${WORK_DIR}/sherpa.sqlite"
    printf 'wal before the update\n' > "${WORK_DIR}/sherpa.sqlite-wal"
    printf 'thirty-two bytes of key material\n' > "${WORK_DIR}/db.key"
    chmod 600 "${WORK_DIR}/sherpa.sqlite" "${WORK_DIR}/sherpa.sqlite-wal" "${WORK_DIR}/db.key"
}

# Run the commands $1 of the script under `set -e` with finish_update on exit, as main runs the
# update, with the server of _server_with_database.
_run_update_steps() {
    run bash -c "
        PATH='${MOCK_BIN}:${PATH}'
        source '${STRIPPED_SCRIPT}'
        set -e
        SHERPA_BASE_DIR='${WORK_DIR}'
        SHERPA_BIN_DIR='${WORK_DIR}/bin'
        SHERPA_DB_FILE='${WORK_DIR}/sherpa.sqlite'
        SHERPA_DB_KEY_FILE='${WORK_DIR}/db.key'
        SHERPA_UPDATE_BACKUP_ROOT='${WORK_DIR}/backups'
        LEGACY_DB_DATA='${WORK_DIR}/sherpa.db'
        SHERPA_VERSION=v0.7.1
        DOWNLOAD_DIR='${WORK_DIR}/new'
        DOWNLOADED_BINARIES=(sherpad)
        trap finish_update EXIT
        $1
    "
}

@test "update_script_018_a_sherpad_archive_that_cannot_be_fetched_stops_the_update_with_sherpad_running" {
    _stub_update_commands
    printf '#!/bin/bash\nexit 22\n' > "${MOCK_BIN}/curl"
    printf '#!/bin/bash\nexit 0\n' > "${MOCK_BIN}/docker"
    for tool in qemu-img mkfs.fat mkfs.vfat mkfs.ext4; do
        printf '#!/bin/bash\nexit 0\n' > "${MOCK_BIN}/${tool}"
    done
    chmod +x "${MOCK_BIN}"/*
    mkdir -p "${WORK_DIR}/bin"
    printf '#!/bin/bash\necho "sherpad 0.7.0"\n' > "${WORK_DIR}/bin/sherpad"
    chmod +x "${WORK_DIR}/bin/sherpad"
    sed -e "s|^SHERPA_BASE_DIR=.*|SHERPA_BASE_DIR='${WORK_DIR}'|" \
        -e 's|^    check_root_privileges$|    :|' "${SCRIPT}" > "${WORK_DIR}/update.sh"
    PATH="${MOCK_BIN}:${PATH}" run bash "${WORK_DIR}/update.sh" --version v0.7.1
    [ "$status" -ne 0 ]
    [[ "$output" == *"Failed to download required binary: sherpad-x86_64-unknown-linux-gnu.tar.gz"* ]]
    [[ "$output" == *"v0.7.1/sherpad-x86_64-unknown-linux-gnu.tar.gz"* ]]
    # sherpad was never stopped, nor restarted, and its binary is the one that was there.
    run grep -E "^(stop|restart) sherpad" "${WORK_DIR}/systemctl.args"
    [ "$status" -ne 0 ]
    [ "$(cat "${WORK_DIR}/bin/sherpad")" = "$(printf '#!/bin/bash\necho "sherpad 0.7.0"')" ]
}

@test "update_script_021_the_database_its_wal_and_its_key_are_backed_up_to_a_new_root_only_folder_and_named" {
    _stub_update_commands
    _server_with_database
    _run_update_steps "backup_database"
    [ "$status" -eq 0 ]
    local backup
    backup="$(find "${WORK_DIR}/backups" -mindepth 1 -maxdepth 1 -type d -name 'update-*')"
    [ -n "${backup}" ]
    [[ "$output" == *"Backup of the database and its key: ${backup}"* ]]
    [ "$(stat -c '%a' "${backup}")" = "700" ]
    for file in sherpa.sqlite sherpa.sqlite-wal db.key; do
        cmp "${WORK_DIR}/${file}" "${backup}/${file}"
        [ "$(stat -c '%a' "${backup}/${file}")" = "600" ]
        # Read as sherpa.
        grep -qx "sherpa cat ${WORK_DIR}/${file}" "${WORK_DIR}/runuser.args"
    done
    [ ! -e "${backup}/sherpa.sqlite-shm" ]
}

@test "update_script_022_a_failure_after_sherpad_stops_and_before_the_install_starts_the_installed_release_again" {
    _stub_update_commands
    _server_with_database
    # The backup folder cannot be made: a file has its name.
    touch "${WORK_DIR}/backups"
    _run_update_steps "SHERPAD_STOPPED=true; backup_database; install_binaries"
    [ "$status" -ne 0 ]
    grep -qx "restart sherpad" "${WORK_DIR}/systemctl.args"
    [[ "$output" == *"running again"* ]]
    grep -q '# old' "${WORK_DIR}/bin/sherpad"
    [ ! -e "${WORK_DIR}/bin/sherpad.previous" ]
    [ "$(cat "${WORK_DIR}/sherpa.sqlite")" = "records before the update" ]
}

@test "update_script_023_a_new_release_that_does_not_start_is_undone_and_the_backup_restored" {
    _stub_update_commands
    _server_with_database
    # The new release migrates the database and leaves a WAL file of its own, then does not start.
    _run_update_steps "SHERPAD_STOPPED=true; backup_database; install_binaries
        printf 'migrated records\n' > '${WORK_DIR}/sherpa.sqlite'
        printf 'new wal\n' > '${WORK_DIR}/sherpa.sqlite-wal'
        printf 'new shm\n' > '${WORK_DIR}/sherpa.sqlite-shm'
        start_sherpad"
    echo "${output}"
    [ "$status" -ne 0 ]
    [[ "$output" == *"Restored the database and its key from ${WORK_DIR}/backups/update-"* ]]
    [[ "$output" == *"running again"* ]]
    # The installed release is back and started, after the new one was started once.
    grep -q '# old' "${WORK_DIR}/bin/sherpad"
    [ ! -e "${WORK_DIR}/bin/sherpad.previous" ]
    [ "$(grep -cx "restart sherpad" "${WORK_DIR}/systemctl.args")" -eq 2 ]
    # The database is the one before the update, and nothing the new release left beside it.
    [ "$(cat "${WORK_DIR}/sherpa.sqlite")" = "records before the update" ]
    [ "$(cat "${WORK_DIR}/sherpa.sqlite-wal")" = "wal before the update" ]
    [ ! -e "${WORK_DIR}/sherpa.sqlite-shm" ]
    [ "$(cat "${WORK_DIR}/db.key")" = "thirty-two bytes of key material" ]
    [ "$(stat -c '%a' "${WORK_DIR}/sherpa.sqlite")" = "600" ]
}

@test "update_script_024_sherpad_counts_as_started_only_when_systemd_does_not_restart_it_during_the_check" {
    _stub_update_commands
    # NRestarts goes up by one each time it is read: systemd keeps starting sherpad again.
    cat > "${MOCK_BIN}/systemctl" << EOF
#!/bin/bash
if [ "\$1" = "show" ]; then
    count=\$(cat '${WORK_DIR}/restarts' 2>/dev/null || echo 0)
    echo \$((count + 1)) > '${WORK_DIR}/restarts'
    echo "\${count}"
fi
exit 0
EOF
    chmod +x "${MOCK_BIN}/systemctl"
    _run_function "set +e; SHERPAD_STOPPED=true; start_sherpad; echo \"started \$? stopped=\${SHERPAD_STOPPED}\""
    [[ "$output" == *"sherpad service failed to start"* ]]
    [[ "$output" == *"started 1 stopped=true"* ]]

    # The same count on every read: started.
    # The stub's own variables: they expand when the stub runs, not here.
    # shellcheck disable=SC2016
    printf '#!/bin/bash\n[ "$1" = "show" ] && echo 0\nexit 0\n' > "${MOCK_BIN}/systemctl"
    _run_function "set +e; SHERPAD_STOPPED=true; start_sherpad; echo \"started \$? stopped=\${SHERPAD_STOPPED}\""
    [[ "$output" == *"started 0 stopped=false"* ]]
}

@test "update_script_026_binaries_are_installed_roots_and_a_failed_start_gets_back_only_those_moved_aside" {
    _stub_update_commands
    _server_with_database
    printf '#!/bin/bash\n# new cli\nexit 0\n' > "${WORK_DIR}/new/sherpa"
    # No CLI is installed, but a sherpa.previous is: the update never moved it aside.
    printf 'planted\n' > "${WORK_DIR}/bin/sherpa.previous"
    local inode
    inode="$(stat -c '%i' "${WORK_DIR}/bin/sherpad")"
    _run_update_steps "SHERPAD_STOPPED=true; DOWNLOADED_BINARIES=(sherpad sherpa); backup_database; install_binaries; false"
    [ "$status" -ne 0 ]
    grep -qx -- "-o root -g root -m 755 ${WORK_DIR}/new/sherpad ${WORK_DIR}/bin/sherpad" "${WORK_DIR}/install.args"
    grep -qx -- "-o root -g root -m 755 ${WORK_DIR}/new/sherpa ${WORK_DIR}/bin/sherpa" "${WORK_DIR}/install.args"
    [[ "$output" == *"running again"* ]]
    # The installed sherpad is back: the same file, so the same owner and mode.
    [ "$(stat -c '%i' "${WORK_DIR}/bin/sherpad")" = "${inode}" ]
    [ ! -e "${WORK_DIR}/bin/sherpad.previous" ]
    # The CLI the update did not move aside is not replaced by the sherpa.previous it found.
    grep -q '# new cli' "${WORK_DIR}/bin/sherpa"
}

@test "update_script_027_a_link_at_or_above_the_backup_folder_stops_the_update_before_the_install" {
    _stub_update_commands
    _server_with_database
    mkdir "${WORK_DIR}/elsewhere"
    ln -s "${WORK_DIR}/elsewhere" "${WORK_DIR}/backups"
    _run_update_steps "SHERPAD_STOPPED=true; backup_database; install_binaries"
    [ "$status" -ne 0 ]
    [[ "$output" == *"${WORK_DIR}/backups is a symlink"* ]]
    [[ "$output" == *"running again"* ]]
    [ -z "$(ls -A "${WORK_DIR}/elsewhere")" ]
    grep -q '# old' "${WORK_DIR}/bin/sherpad"
    [ ! -e "${WORK_DIR}/bin/sherpad.previous" ]
}

# ============================================================
# Router image
# ============================================================

@test "update_script_029_the_router_image_is_pulled_by_the_digest_the_install_pins_and_a_failed_pull_stops_the_update" {
    printf '#!/bin/bash\necho "$@" >> "%s/docker.args"\nexit 0\n' "${WORK_DIR}" > "${MOCK_BIN}/docker"
    chmod +x "${MOCK_BIN}/docker"
    _run_function pull_sherpa_router_image
    [ "$status" -eq 0 ]
    local install_digest install_pin
    install_digest="$(sed -n 's/^SHERPA_ROUTER_DIGEST="\(sha256:[0-9a-f]\{64\}\)"$/\1/p' "${BATS_TEST_DIRNAME}/../../scripts/sherpa_install.sh")"
    install_pin="ghcr.io/bwks/sherpa-router@${install_digest}"
    grep -qx "image pull ${install_pin}" "${WORK_DIR}/docker.args"
    grep -qx "image tag ${install_pin} ghcr.io/bwks/sherpa-router:latest" "${WORK_DIR}/docker.args"

    rm "${WORK_DIR}/docker.args"
    printf '#!/bin/bash\necho "$@" >> "%s/docker.args"\nexit 1\n' "${WORK_DIR}" > "${MOCK_BIN}/docker"
    _run_function pull_sherpa_router_image
    [ "$status" -eq 1 ]
    run grep -q "image tag" "${WORK_DIR}/docker.args"
    [ "$status" -ne 0 ]
}
