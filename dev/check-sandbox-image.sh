#! /usr/bin/bash
#
# Refuse a sandbox base image the server and the scripts disagree on: the "sandbox image"
# section, with the Flatcar version and the SHA-512s of its image and kernel, must be the same
# in `scripts/sherpa_install.sh` and `scripts/sherpa_update.sh`, and its version must be
# `FLATCAR_SANDBOX_VERSION` in `crates/server/src/services/sandbox/base.rs`.
#
# Run by `.githooks/pre-commit`.

set -euo pipefail

root="$(git rev-parse --show-toplevel)"
install="${root}/scripts/sherpa_install.sh"
update="${root}/scripts/sherpa_update.sh"
rust="${root}/crates/server/src/services/sandbox/base.rs"

# Prints the sandbox image section of script $1.
section() {
    sed -n '/^# BEGIN sandbox image$/,/^# END sandbox image$/p' "$1"
}

# Prints the value of shell variable $1 in the sandbox image section of script $2.
script_value() {
    section "$2" | sed -n "s/^$1=\"\\(.*\\)\"$/\\1/p"
}

status=0
for variable in SANDBOX_FLATCAR_VERSION SANDBOX_IMAGE_SHA512 SANDBOX_KERNEL_SHA512; do
    if [ -z "$(script_value "${variable}" "${install}")" ]; then
        echo "${install} sets no ${variable} in its sandbox image section" >&2
        status=1
    fi
done
if ! diff <(section "${install}") <(section "${update}") >/dev/null; then
    echo "The sandbox image sections of ${install} and ${update} differ" >&2
    status=1
fi
script_version="$(script_value SANDBOX_FLATCAR_VERSION "${install}")"
rust_version="$(sed -n 's/^pub(crate) const FLATCAR_SANDBOX_VERSION: &str = "\(.*\)";$/\1/p' "${rust}")"
if [ "${script_version}" != "${rust_version}" ]; then
    echo "The scripts download Flatcar ${script_version}, and sherpad boots ${rust_version:-no version} (${rust})" >&2
    status=1
fi
exit "${status}"
