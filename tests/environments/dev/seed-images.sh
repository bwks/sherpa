#!/usr/bin/env bash
# Seed a dev server with the host's VM images. Run it on the host, after `sherpa up` and the
# provisioning of the lab have finished.
#
#   tests/environments/dev/seed-images.sh <lab dir> <node>
#
# For each model and version in images.toml next to this script it asks the guest's Sherpa server
# whether the image is there, and skips it if so. Otherwise it copies
# /opt/sherpa/images/<model>/<version>/virtioa.qcow2 from the host into a staging directory in
# the guest with rsync, imports it with the guest's `sherpa server image import`, and deletes the
# staged copy. An interrupted copy resumes on the next run. The guest's CLI is already logged in
# by provisioning, so no password or token is needed or passed.
#
# Everything goes over the lab's own .sherpa/ssh/config, alias `<node>` or `<node>.<lab id>`,
# through one shared SSH connection. An image that is missing on the host or fails to import is
# reported and the rest go on; the exit status is 1 if any failed.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
IMAGES_FILE="${SCRIPT_DIR}/images.toml"
HOST_IMAGES="/opt/sherpa/images"
DISK_FILE="virtioa.qcow2"
GUEST_SHERPA="/opt/sherpa/bin/sherpa"
# Readable by the Sherpa server whichever user it runs as; it copies the disk from here.
GUEST_STAGING="/var/tmp/sherpa-seed"

if [ "$#" -ne 2 ]; then
    echo "Usage: $0 <lab dir> <node>" >&2
    exit 2
fi
LAB_DIR="$1"
NODE="$2"

if ! [[ "${NODE}" =~ ^[a-z0-9-]{1,63}$ ]]; then
    echo "Node '${NODE}' is not a node name: lowercase letters, digits and hyphens" >&2
    exit 2
fi
SSH_CONFIG="${LAB_DIR}/.sherpa/ssh/config"
LAB_INFO="${LAB_DIR}/.sherpa/lab-info.toml"
for file in "${SSH_CONFIG}" "${LAB_INFO}" "${IMAGES_FILE}"; do
    if [ ! -f "${file}" ]; then
        echo "${file} not found" >&2
        exit 1
    fi
done

# The first top-level `key = "value"` string in a TOML file.
toml_string() {
    sed -n "s/^$1[[:space:]]*=[[:space:]]*\"\([^\"]*\)\".*/\1/p" "$2" | head -n 1
}

# images.toml as `<model> <version>` lines. Only the layout images.toml uses is accepted:
# comments, blank lines, `[[images]]` and `model`/`version` strings; anything else is an error.
read_images() {
    awk '
        function flush() {
            if (started) {
                if (model == "" || version == "") {
                    printf "images.toml line %d: an [[images]] entry needs model and version\n", start > "/dev/stderr"
                    bad = 1
                } else {
                    print model, version
                }
            }
        }
        /^[[:space:]]*(#.*)?$/ { next }
        /^\[\[images\]\][[:space:]]*$/ { flush(); started = 1; start = NR; model = ""; version = ""; next }
        started && /^model[[:space:]]*=[[:space:]]*"[^"]*"[[:space:]]*$/ { split($0, part, "\""); model = part[2]; next }
        started && /^version[[:space:]]*=[[:space:]]*"[^"]*"[[:space:]]*$/ { split($0, part, "\""); version = part[2]; next }
        { printf "images.toml line %d: not understood: %s\n", NR, $0 > "/dev/stderr"; bad = 1 }
        END { flush(); exit bad }
    ' "${IMAGES_FILE}"
}

IMAGES="$(read_images)"
# Model and version go into commands run in the guest: hold them to the server's rules.
while read -r model version; do
    if ! [[ "${model}" =~ ^[a-z0-9_]+$ ]] || ! [[ "${version}" =~ ^[A-Za-z0-9][A-Za-z0-9._+-]{0,63}$ ]]; then
        echo "images.toml: '${model}' '${version}' is not a valid model and version" >&2
        exit 1
    fi
done <<< "${IMAGES}"

LAB_ID="$(toml_string id "${LAB_INFO}")"
ALIAS=""
HOSTS="$(awk 'tolower($1) == "host" { for (i = 2; i <= NF; i++) print $i }' "${SSH_CONFIG}")"
for candidate in "${NODE}" "${NODE}.${LAB_ID}"; do
    if grep -qxF -- "${candidate}" <<< "${HOSTS}"; then
        ALIAS="${candidate}"
        break
    fi
done
if [ -z "${ALIAS}" ]; then
    echo "No host '${NODE}' or '${NODE}.${LAB_ID}' in ${SSH_CONFIG}" >&2
    exit 1
fi

# The lab's SSH config names its key and known hosts by absolute path; ssh runs from the lab
# directory so the config's path is the one `sherpa` documents. One master connection serves
# every ssh and rsync below; its socket directory is private and removed on exit, with the master.
cd "${LAB_DIR}"
SOCKET_DIR="$(mktemp -d)"
SSH=(ssh -F .sherpa/ssh/config -o ControlMaster=auto -o "ControlPath=${SOCKET_DIR}/%C" -o ControlPersist=60)
cleanup() {
    "${SSH[@]}" -O exit "${ALIAS}" 2> /dev/null || true
    rm -rf "${SOCKET_DIR}"
}
trap cleanup EXIT

guest() {
    "${SSH[@]}" -n "${ALIAS}" "$@"
}

if ! guest "${GUEST_SHERPA} server image list" > /dev/null; then
    echo "The Sherpa server in ${ALIAS} did not answer \`sherpa server image list\`; is the lab provisioned?" >&2
    exit 1
fi
guest "install -d -m 755 ${GUEST_STAGING}"

imported=0
skipped=0
failed=()
while read -r model version; do
    name="${model} ${version}"
    if guest "${GUEST_SHERPA} server image show ${model} --version ${version}" > /dev/null 2>&1; then
        echo "==> ${name}: already imported, skipping"
        skipped=$((skipped + 1))
        continue
    fi
    src="${HOST_IMAGES}/${model}/${version}/${DISK_FILE}"
    if [ ! -r "${src}" ]; then
        echo "==> ${name}: ${src} not found or not readable on the host" >&2
        failed+=("${name}")
        continue
    fi
    staged="${GUEST_STAGING}/${model}-${version}.qcow2"
    echo "==> ${name}: copying $(du -h "${src}" | cut -f1)"
    if ! rsync --partial --chmod=F644 -e "${SSH[*]}" "${src}" "${ALIAS}:${staged}"; then
        echo "==> ${name}: copy failed; the next run resumes it" >&2
        failed+=("${name}")
        continue
    fi
    echo "==> ${name}: importing"
    if ! guest "${GUEST_SHERPA} server image import --model ${model} --version ${version} --src ${staged}"; then
        echo "==> ${name}: import failed; the staged copy is kept at ${staged}" >&2
        failed+=("${name}")
        continue
    fi
    guest "rm -f ${staged}"
    imported=$((imported + 1))
done <<< "${IMAGES}"

echo "Imported ${imported}, skipped ${skipped}, failed ${#failed[@]}"
if [ "${#failed[@]}" -gt 0 ]; then
    printf '  failed: %s\n' "${failed[@]}" >&2
    exit 1
fi
