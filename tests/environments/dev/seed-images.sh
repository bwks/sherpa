#!/usr/bin/env bash
# Seed a dev server with the host's VM images. Run it on the host, after `sherpa up` and the
# provisioning of the lab have finished, once for each node:
#
#   tests/environments/dev/seed-images.sh <lab dir> <node>
#   tests/environments/dev/seed-images.sh ~/labs/sherpa-dev dev01
#
# For each model and version in images.toml next to this script it asks the guest's Sherpa server
# whether the image is there, and skips it if so. Otherwise it copies the entry's `source`, a disk
# under /opt/sherpa/images on the host, into a staging directory in the guest with rsync, imports it with the guest's `sherpa server image import`, and deletes the
# staged copy. An interrupted copy resumes on the next run. The guest's CLI is already logged in
# by provisioning, so no password or token is needed or passed.
#
# Then, for each of images.toml's extra files, such as the vEOS Aboot ISO, it compares the SHA-256
# of its `source` on the host and its `path` in the guest, both under /opt/sherpa/images, and skips
# the file if they match. Otherwise it copies the file through the same staging directory, checks
# its SHA-256 there, and installs it at its path in the guest, owned by the guest's `sherpa`
# account.
#
# The host path is each entry's own, not derived from the model or the guest path, so a host on a
# release with other model names (before 0.7.0, `arista_veos` for `arista_eos`) seeds a newer
# guest unchanged.
#
# Everything goes over the lab's own .sherpa/ssh/config, alias `<node>` or `<node>.<lab id>`,
# through one shared SSH connection. An image or file that is missing on the host or fails to
# import or copy is reported and the rest go on; the exit status is 1 if any failed.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
IMAGES_FILE="${SCRIPT_DIR}/images.toml"
HOST_IMAGES="/opt/sherpa/images"
GUEST_IMAGES="/opt/sherpa/images"
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

# images.toml as `image <model> <version> <source>` and `file <path> <source>` lines. Only the
# layout images.toml uses is accepted: comments, blank lines, `[[images]]` with `model`, `version`
# and `source` strings and `[[extra_files]]` with `path` and `source` strings; anything else is an
# error.
read_entries() {
    awk '
        function flush() {
            if (section == "images") {
                if (model == "" || version == "" || source == "") {
                    printf "images.toml line %d: an [[images]] entry needs model, version and source\n", start > "/dev/stderr"
                    bad = 1
                } else {
                    print "image", model, version, source
                }
            } else if (section == "extra_files") {
                if (path == "" || source == "") {
                    printf "images.toml line %d: an [[extra_files]] entry needs path and source\n", start > "/dev/stderr"
                    bad = 1
                } else {
                    print "file", path, source
                }
            }
        }
        function value() { split($0, part, "\""); return part[2] }
        /^[[:space:]]*(#.*)?$/ { next }
        /^\[\[images\]\][[:space:]]*$/ { flush(); section = "images"; start = NR; model = ""; version = ""; source = ""; next }
        /^\[\[extra_files\]\][[:space:]]*$/ { flush(); section = "extra_files"; start = NR; path = ""; source = ""; next }
        section == "images" && /^model[[:space:]]*=[[:space:]]*"[^"]*"[[:space:]]*$/ { model = value(); next }
        section == "images" && /^version[[:space:]]*=[[:space:]]*"[^"]*"[[:space:]]*$/ { version = value(); next }
        section == "extra_files" && /^path[[:space:]]*=[[:space:]]*"[^"]*"[[:space:]]*$/ { path = value(); next }
        section != "" && /^source[[:space:]]*=[[:space:]]*"[^"]*"[[:space:]]*$/ { source = value(); next }
        { printf "images.toml line %d: not understood: %s\n", NR, $0 > "/dev/stderr"; bad = 1 }
        END { flush(); exit bad }
    ' "${IMAGES_FILE}"
}

ENTRIES="$(read_entries)"
# Whether $1 is a plain relative path under the images folder, with no `.` or `..`.
relative_path() {
    [[ "$1" =~ ^[A-Za-z0-9._+-]+(/[A-Za-z0-9._+-]+)*$ ]] && ! [[ "/$1/" =~ /\.\.?/ ]]
}

# Models, versions and paths go into commands run in the guest: hold models and versions to the
# server's rules, and paths to plain relative names under the images folder.
while read -r kind first second third rest; do
    case "${kind}" in
        image)
            if ! [[ "${first}" =~ ^[a-z0-9_]+$ ]] || ! [[ "${second}" =~ ^[A-Za-z0-9][A-Za-z0-9._+-]{0,63}$ ]]; then
                echo "images.toml: '${first}' '${second}' is not a valid model and version" >&2
                exit 1
            fi
            if [ -n "${rest}" ] || ! relative_path "${third}"; then
                echo "images.toml: source '${third}${rest:+ ${rest}}' of ${first} ${second} is not a relative path under ${HOST_IMAGES}" >&2
                exit 1
            fi
            ;;
        file)
            for path in "${first}" "${second}"; do
                if [ -n "${third}" ] || ! relative_path "${path}"; then
                    echo "images.toml: '${first} ${second}${third:+ ${third}}' is not a path and a source under ${HOST_IMAGES}" >&2
                    exit 1
                fi
            done
            ;;
    esac
done <<< "${ENTRIES}"
IMAGES="$(awk '$1 == "image" { print $2, $3, $4 }' <<< "${ENTRIES}")"
EXTRA_FILES="$(awk '$1 == "file" { print $2, $3 }' <<< "${ENTRIES}")"

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
copied=0
skipped=0
failed=()
while read -r model version source; do
    [ -n "${model}" ] || continue
    name="${model} ${version}"
    if guest "${GUEST_SHERPA} server image show ${model} --version ${version}" > /dev/null 2>&1; then
        echo "==> ${name}: already imported, skipping"
        skipped=$((skipped + 1))
        continue
    fi
    src="${HOST_IMAGES}/${source}"
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

# The SHA-256 of a file in the guest, or nothing if it is not there. The images folder belongs to
# the guest's `sherpa` account, so the guest's login reads and writes in it with sudo.
guest_sha256() {
    guest "sudo sh -c 'test -f $1 && sha256sum $1'" | cut -d ' ' -f 1
}

while read -r path source; do
    [ -n "${path}" ] || continue
    src="${HOST_IMAGES}/${source}"
    dest="${GUEST_IMAGES}/${path}"
    if [ ! -r "${src}" ]; then
        echo "==> ${path}: ${src} not found or not readable on the host" >&2
        failed+=("${path}")
        continue
    fi
    sum="$(sha256sum "${src}" | cut -d ' ' -f 1)"
    if [ "$(guest_sha256 "${dest}")" = "${sum}" ]; then
        echo "==> ${path}: already there with the same checksum, skipping"
        skipped=$((skipped + 1))
        continue
    fi
    staged="${GUEST_STAGING}/$(basename "${path}")"
    echo "==> ${path}: copying $(du -h "${src}" | cut -f1)"
    if ! rsync --partial --chmod=F644 -e "${SSH[*]}" "${src}" "${ALIAS}:${staged}"; then
        echo "==> ${path}: copy failed; the next run resumes it" >&2
        failed+=("${path}")
        continue
    fi
    if [ "$(guest_sha256 "${staged}")" != "${sum}" ]; then
        echo "==> ${path}: the copy's checksum does not match the host's; deleted, the next run copies it again" >&2
        guest "rm -f ${staged}"
        failed+=("${path}")
        continue
    fi
    echo "==> ${path}: installing at ${dest}"
    if ! guest "sudo -u sherpa mkdir -p $(dirname "${dest}") && sudo install -o sherpa -g sherpa -m 644 ${staged} ${dest}"; then
        echo "==> ${path}: install failed; the staged copy is kept at ${staged}" >&2
        failed+=("${path}")
        continue
    fi
    guest "rm -f ${staged}"
    copied=$((copied + 1))
done <<< "${EXTRA_FILES}"

echo "Imported ${imported}, copied ${copied}, skipped ${skipped}, failed ${#failed[@]}"
if [ "${#failed[@]}" -gt 0 ]; then
    printf '  failed: %s\n' "${failed[@]}" >&2
    exit 1
fi
