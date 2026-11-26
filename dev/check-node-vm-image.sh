#! /usr/bin/bash
#
# Refuse a node VM base image the server and the scripts disagree on: the "node VM image"
# section, with the Flatcar version and the SHA-512s of its image and kernel, must be the same
# in `scripts/sherpa_install.sh` and `scripts/sherpa_update.sh`, and its version must be
# `FLATCAR_NODE_VM_VERSION` in `crates/server/src/services/node_vm/base.rs`.
#
# Run by `.githooks/pre-commit`.

set -euo pipefail

root="$(git rev-parse --show-toplevel)"
install="${root}/scripts/sherpa_install.sh"
update="${root}/scripts/sherpa_update.sh"
rust="${root}/crates/server/src/services/node_vm/base.rs"

# Prints the node VM image section of script $1.
section() {
    sed -n '/^# BEGIN node VM image$/,/^# END node VM image$/p' "$1"
}

# Prints the value of shell variable $1 in the node VM image section of script $2.
script_value() {
    section "$2" | sed -n "s/^$1=\"\\(.*\\)\"$/\\1/p"
}

status=0
for variable in NODE_VM_FLATCAR_VERSION NODE_VM_IMAGE_SHA512 NODE_VM_KERNEL_SHA512; do
    if [ -z "$(script_value "${variable}" "${install}")" ]; then
        echo "${install} sets no ${variable} in its node VM image section" >&2
        status=1
    fi
done
if ! diff <(section "${install}") <(section "${update}") >/dev/null; then
    echo "The node VM image sections of ${install} and ${update} differ" >&2
    status=1
fi
script_version="$(script_value NODE_VM_FLATCAR_VERSION "${install}")"
rust_version="$(sed -n 's/^pub(crate) const FLATCAR_NODE_VM_VERSION: &str = "\(.*\)";$/\1/p' "${rust}")"
if [ "${script_version}" != "${rust_version}" ]; then
    echo "The scripts download Flatcar ${script_version}, and sherpad boots ${rust_version:-no version} (${rust})" >&2
    status=1
fi
exit "${status}"
