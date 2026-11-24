#!/usr/bin/env bats
#
# Test suite for scripts/sherpa_uninstall.sh. Each test is a case of
# tests/specs/install-and-update.md and is named by its ID.
#
# Run as a user that is not root:
#   bats tests/bats/uninstall_tests.bats
#
# What an uninstall removes and keeps on a host is checked by the VM run
# (tests/harness/test_install.sh).

SCRIPT="${BATS_TEST_DIRNAME}/../../scripts/sherpa_uninstall.sh"
LAB_ID="1a2b3c4d"
MODES=("--keep-data" "--remove-data" "--remove-all")

# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------

setup() {
    WORK_DIR="$(mktemp -d)"
    STUB_DIR="${WORK_DIR}/stub"
    MOCK_BIN="${WORK_DIR}/bin"
    STRIPPED_SCRIPT="${WORK_DIR}/uninstall.sh"
    mkdir -p "${STUB_DIR}" "${MOCK_BIN}"
    grep -v '^main "\$@"' "${SCRIPT}" > "${STRIPPED_SCRIPT}"
    _stub_runtimes
}

teardown() {
    rm -rf "${WORK_DIR:-}"
}

# Stub virsh and docker to answer from the files in STUB_DIR, and the commands that change the
# host to only record their arguments in STUB_DIR/calls.
_stub_runtimes() {
    # The stubs' own variables: they expand when a stub runs, not here.
    cat > "${MOCK_BIN}/virsh" << EOF
#!/bin/bash
dir='${STUB_DIR}'
EOF
    cat >> "${MOCK_BIN}/virsh" << 'EOF'
[ -e "${dir}/virsh-fails" ] && { echo "failed to connect to the hypervisor" >&2; exit 1; }
shift 2
case "$1 $2" in
    "list --name") cat "${dir}/domains-running" 2>/dev/null || true ;;
    "list --all") cat "${dir}/domains-all" 2>/dev/null || true ;;
    "net-list --all") cat "${dir}/networks" 2>/dev/null || true ;;
    "pool-info "*) [ -e "${dir}/pool" ] ;;
esac
EOF
    cat > "${MOCK_BIN}/docker" << EOF
#!/bin/bash
dir='${STUB_DIR}'
EOF
    cat >> "${MOCK_BIN}/docker" << 'EOF'
[ -e "${dir}/docker-fails" ] && { echo "Cannot connect to the Docker daemon" >&2; exit 1; }
case "$1 $2" in
    "ps --format") cat "${dir}/containers-running" 2>/dev/null || true ;;
    "ps -a") cat "${dir}/containers-all" 2>/dev/null || true ;;
    "network ls") cat "${dir}/docker-networks" 2>/dev/null || true ;;
    "volume ls") cat "${dir}/volumes" 2>/dev/null || true ;;
    *) echo "docker $*" >> "${dir}/calls" ;;
esac
EOF
    local command
    for command in systemctl userdel groupdel; do
        cat > "${MOCK_BIN}/${command}" << EOF
#!/bin/bash
echo "${command} \$*" >> '${STUB_DIR}/calls'
exit 1
EOF
    done
    chmod +x "${MOCK_BIN}"/*
}

# Make an installation under WORK_DIR with one lab folder.
_make_installation() {
    mkdir -p "${WORK_DIR}/opt/sherpa/"{bin,db,config,env,.secret,.certs,labs/${LAB_ID}} \
        "${WORK_DIR}/run/sherpa"
    touch "${WORK_DIR}/opt/sherpa/bin/sherpad" "${WORK_DIR}/opt/sherpa/db/sherpa.sqlite" \
        "${WORK_DIR}/opt/sherpa/.secret/db.key" "${WORK_DIR}/opt/sherpa/config/sherpa.toml"
}

# Every path under WORK_DIR's installation, with its size.
_installation_tree() {
    find "${WORK_DIR}/opt" "${WORK_DIR}/run" -printf '%p %s\n' | sort
}

# The script's settings pointed at WORK_DIR, for a bash -c that sourced it.
_paths() {
    echo "
        SHERPA_BASE_DIR='${WORK_DIR}/opt/sherpa'
        SHERPA_DB_DIR='${WORK_DIR}/opt/sherpa/db'
        SHERPA_LABS_DIR='${WORK_DIR}/opt/sherpa/labs'
        SHERPA_RUN_DIR='${WORK_DIR}/run/sherpa'
        SHERPA_UPDATE_BACKUP_DIR='${WORK_DIR}/var/backups/sherpa'
    "
}

# Run main with the arguments given, as root would get past the root check, with the stubs, the
# paths under WORK_DIR and the answer $ANSWER on stdin.
_run_main() {
    run bash -c "
        PATH='${MOCK_BIN}:${PATH}'
        source '${STRIPPED_SCRIPT}'
        $(_paths)
        check_root_privileges() { :; }
        main $*
    " <<< "${ANSWER-yes}"
}

# Run each mode, with and without --force, and check each is refused, names the lab and its
# node $1, and changes nothing.
_check_refused_in_every_mode() {
    local node="$1" mode force before
    _make_installation
    before="$(_installation_tree)"
    for mode in "${MODES[@]}"; do
        for force in "" "--force"; do
            _run_main "${mode}" ${force}
            [ "$status" -eq 1 ] || { echo "${mode} ${force}: exit ${status}"; return 1; }
            [[ "$output" == *"Labs are running. Nothing was changed."* ]]
            [[ "$output" == *"Lab ${LAB_ID}:"* ]]
            [[ "$output" == *"${node}"* ]]
            [[ "$output" == *"sherpa destroy"* ]]
            [[ "$output" != *"Continue?"* ]]
            [ "$(_installation_tree)" = "${before}" ]
            [ ! -e "${STUB_DIR}/calls" ]
        done
    done
}

# ---------------------------------------------------------------------------
# Tests
# ---------------------------------------------------------------------------

@test "uninstall_script_009_without_force_any_answer_but_yes_removes_nothing_and_exits_0" {
    local before answer
    _make_installation
    before="$(_installation_tree)"
    for answer in no y ""; do
        ANSWER="${answer}" _run_main --remove-all
        [ "$status" -eq 0 ]
        [[ "$output" == *"Uninstall cancelled by user"* ]]
        [ "$(_installation_tree)" = "${before}" ]
        [ ! -e "${STUB_DIR}/calls" ]
    done
}

@test "uninstall_script_010_help_prints_the_usage_an_unknown_option_is_an_error_and_not_root_is_refused" {
    # Past the root check the script removes the installation: never run it as root here.
    if [ "$(id -u)" -eq 0 ]; then
        skip "running as root — invoke as non-root"
    fi
    run bash "${SCRIPT}" --help
    [ "$status" -eq 0 ]
    [[ "$output" == *"Usage:"* ]]
    [[ "$output" == *"--remove-all"* ]]

    run bash "${SCRIPT}" --bogus
    [ "$status" -eq 1 ]
    [[ "$output" == *"Unknown option: --bogus"* ]]

    run bash "${SCRIPT}" --force
    [ "$status" -eq 1 ]
    [[ "$output" == *"must be run as root"* ]]
}

@test "uninstall_script_011_a_sherpa_db_container_on_the_host_is_stopped_and_removed" {
    printf 'sherpa-db\n' > "${STUB_DIR}/containers-all"
    cat > "${MOCK_BIN}/docker" << EOF
#!/bin/bash
case "\$1 \$2" in
    "ps -a") echo sherpa-db-backup; echo sherpa-db ;;
    "ps --format") echo sherpa-db ;;
    *) echo "docker \$*" >> '${STUB_DIR}/calls' ;;
esac
EOF
    run bash -c "
        PATH='${MOCK_BIN}:${PATH}'
        source '${STRIPPED_SCRIPT}'
        stop_container
        remove_container
    "
    [ "$status" -eq 0 ]
    [ "$(cat "${STUB_DIR}/calls")" = "docker stop sherpa-db
docker rm sherpa-db" ]
}

@test "uninstall_script_012_a_link_at_or_above_the_database_or_binaries_folder_removes_nothing_through_it" {
    local work stripped
    work="$(mktemp -d)"
    stripped="${work}/uninstall.sh"
    grep -v '^main "\$@"' "${SCRIPT}" > "${stripped}"
    mkdir -p "${work}/elsewhere" "${work}/opt/sherpa"
    touch "${work}/elsewhere/sherpa.sqlite" "${work}/elsewhere/sherpad"
    ln -s "${work}/elsewhere" "${work}/opt/sherpa/db"
    ln -s "${work}/elsewhere" "${work}/opt/sherpa/bin"
    run bash -c "
        source '${stripped}'
        set +e
        SHERPA_BASE_DIR='${work}/opt/sherpa'
        SHERPA_DB_DIR='${work}/opt/sherpa/db'
        remove_database_data
        echo \"database: \$?\"
        remove_sherpad_binary
    "
    [[ "$output" == *"${work}/opt/sherpa/db is a symlink"* ]]
    [[ "$output" == *"database: 1"* ]]
    [[ "$output" == *"${work}/opt/sherpa/bin is a symlink"* ]]
    [ -e "${work}/elsewhere/sherpa.sqlite" ]
    [ -e "${work}/elsewhere/sherpad" ]
    rm -rf "${work}"
}

@test "uninstall_script_013_a_running_lab_vm_is_refused_in_every_mode_and_with_force_changing_nothing" {
    printf 'r1-%s\nsomeone-elses-vm\n' "${LAB_ID}" > "${STUB_DIR}/domains-running"
    _check_refused_in_every_mode "VM r1 (r1-${LAB_ID})"
}

@test "uninstall_script_014_a_running_lab_container_is_refused_in_every_mode_and_with_force_changing_nothing" {
    printf 'ctr01-%s\t\t\nweb-1\t\t\n' "${LAB_ID}" > "${STUB_DIR}/containers-running"
    _check_refused_in_every_mode "container ctr01 (ctr01-${LAB_ID})"
}

@test "uninstall_script_015_a_running_tailscale_gateway_is_refused_in_every_mode_and_with_force_changing_nothing" {
    printf 'sherpa-tailnet-%s\ttailnet-gateway\t%s\n' "${LAB_ID}" "${LAB_ID}" \
        > "${STUB_DIR}/containers-running"
    _check_refused_in_every_mode "Tailscale gateway sherpa-tailnet-${LAB_ID}"
}

@test "uninstall_script_017_remove_all_lists_the_stopped_labs_leftovers_before_the_prompt_and_in_the_summary" {
    _make_installation
    printf 'r1-%s\nsomeone-elses-vm\n' "${LAB_ID}" > "${STUB_DIR}/domains-all"
    printf 'sherpa-management-%s\ndefault\n' "${LAB_ID}" > "${STUB_DIR}/networks"
    printf 'ctr01-%s\nweb-1\n' "${LAB_ID}" > "${STUB_DIR}/containers-all"
    printf 'sherpa-tailnet-state-%s\n' "${LAB_ID}" > "${STUB_DIR}/volumes"
    touch "${STUB_DIR}/pool"
    run bash -c "
        PATH='${MOCK_BIN}:${PATH}'
        source '${STRIPPED_SCRIPT}'
        $(_paths)
        REMOVE_ALL=true
        KEEP_DATA=false
        LAB_LEFTOVERS=\"\$(lab_leftovers)\"
        print_plan
        echo '--- summary ---'
        print_completion_message
    "
    [ "$status" -eq 0 ]
    local part
    for part in "${output%%--- summary ---*}" "${output##*--- summary ---}"; do
        [[ "$part" == *"libvirt domain r1-${LAB_ID} (lab ${LAB_ID})"* ]]
        [[ "$part" == *"libvirt network sherpa-management-${LAB_ID} (lab ${LAB_ID})"* ]]
        [[ "$part" == *"Docker container ctr01-${LAB_ID} (lab ${LAB_ID})"* ]]
        [[ "$part" == *"Docker volume sherpa-tailnet-state-${LAB_ID} (lab ${LAB_ID})"* ]]
        [[ "$part" == *"libvirt storage pool sherpa-pool"* ]]
        [[ "$part" == *"destroy the labs before --remove-all"* ]]
        [[ "$part" != *"someone-elses-vm"* ]]
        [[ "$part" != *"web-1"* ]]
        [[ "$part" != *"network default"* ]]
    done
}

# Print the plan and the closing summary of the mode set by $1, separated by a marker line.
_plan_and_summary() {
    run bash -c "
        PATH='${MOCK_BIN}:${PATH}'
        source '${STRIPPED_SCRIPT}'
        $(_paths)
        $1
        print_plan
        echo '--- summary ---'
        print_completion_message
    "
    [ "$status" -eq 0 ]
}

@test "uninstall_script_018_keep_data_states_before_the_prompt_and_in_the_summary_everything_it_keeps" {
    mkdir -p "${WORK_DIR}/var/backups/sherpa"
    _plan_and_summary "KEEP_DATA=true"
    local base="${WORK_DIR}/opt/sherpa" part
    for part in "${output%%--- summary ---*}" "${output##*--- summary ---}"; do
        [[ "$part" == *"The database and its backups: ${base}/db"* ]]
        [[ "$part" == *"${base}/.secret"* ]]
        [[ "$part" == *"${base}/.certs"* ]]
        [[ "$part" == *"${base}/config/sherpa.toml"* ]]
        [[ "$part" == *"${base}/env/sherpa.env"* ]]
        [[ "$part" == *"The lab folders: ${base}/labs"* ]]
        [[ "$part" == *"${WORK_DIR}/run/sherpa"* ]]
        [[ "$part" == *"${WORK_DIR}/var/backups/sherpa"* ]]
        [[ "$part" == *"The sherpa account and group"* ]]
    done
}

@test "uninstall_script_019_remove_data_states_before_the_prompt_and_in_the_summary_everything_it_keeps" {
    mkdir -p "${WORK_DIR}/var/backups/sherpa"
    _plan_and_summary "KEEP_DATA=false"
    local base="${WORK_DIR}/opt/sherpa" part
    for part in "${output%%--- summary ---*}" "${output##*--- summary ---}"; do
        [[ "$part" == *"The database and its backups in ${base}/db"* ]]
        [[ "$part" != *"The database and its backups: ${base}/db"* ]]
        [[ "$part" == *"${base}/.secret"* ]]
        [[ "$part" == *"${base}/.certs"* ]]
        [[ "$part" == *"${base}/config/sherpa.toml"* ]]
        [[ "$part" == *"${base}/env/sherpa.env"* ]]
        [[ "$part" == *"The lab folders: ${base}/labs"* ]]
        [[ "$part" == *"${WORK_DIR}/run/sherpa"* ]]
        [[ "$part" == *"${WORK_DIR}/var/backups/sherpa"* ]]
        [[ "$part" == *"The sherpa account and group"* ]]
    done
}

@test "uninstall_script_020_remove_all_states_before_the_prompt_and_in_the_summary_everything_it_keeps" {
    mkdir -p "${WORK_DIR}/var/backups/sherpa"
    _plan_and_summary "REMOVE_ALL=true; KEEP_DATA=false"
    local base="${WORK_DIR}/opt/sherpa" part removed kept
    for part in "${output%%--- summary ---*}" "${output##*--- summary ---}"; do
        removed="${part%%[Kk]ept:*}"
        kept="${part#*[Kk]ept:}"
        [[ "$removed" == *"${WORK_DIR}/var/backups/sherpa"* ]]
        [[ "$kept" != *"${WORK_DIR}/var/backups/sherpa"* ]]
        [[ "$kept" == *"The packages the install script installed"* ]]
        [[ "$kept" != *"${base}/.secret"* ]]
        [[ "$kept" != *"The lab folders"* ]]
        [[ "$kept" != *"The sherpa account and group"* ]]
    done
}

@test "uninstall_script_022_labs_on_a_host_whose_docker_or_libvirt_does_not_answer_are_refused" {
    local before stub
    _make_installation
    before="$(_installation_tree)"
    for stub in docker-fails virsh-fails; do
        touch "${STUB_DIR}/${stub}"
        _run_main --remove-all --force
        [ "$status" -eq 1 ]
        [[ "$output" == *"cannot tell whether a lab is running"* ]]
        [[ "$output" == *"Cannot list the running"* ]]
        [ "$(_installation_tree)" = "${before}" ]
        [ ! -e "${STUB_DIR}/calls" ]
        rm "${STUB_DIR}/${stub}"
    done
}

@test "uninstall_script_023_remove_all_removes_the_update_backups_and_nothing_through_a_link_above_them" {
    local backups="${WORK_DIR}/var/backups/sherpa"
    mkdir -p "${backups}/update-20261101T090807Z" "${WORK_DIR}/elsewhere/sherpa"
    touch "${backups}/update-20261101T090807Z/sherpa.sqlite" "${WORK_DIR}/elsewhere/sherpa/db.key"
    run bash -c "
        source '${STRIPPED_SCRIPT}'
        $(_paths)
        remove_update_backups
    "
    [ "$status" -eq 0 ]
    [ ! -e "${backups}" ]

    rmdir "${WORK_DIR}/var/backups"
    ln -s "${WORK_DIR}/elsewhere" "${WORK_DIR}/var/backups"
    run bash -c "
        source '${STRIPPED_SCRIPT}'
        $(_paths)
        remove_update_backups
    "
    [ "$status" -eq 1 ]
    [[ "$output" == *"${WORK_DIR}/var/backups is a symlink"* ]]
    [ -e "${WORK_DIR}/elsewhere/sherpa/db.key" ]
}
