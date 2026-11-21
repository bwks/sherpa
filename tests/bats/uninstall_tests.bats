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
