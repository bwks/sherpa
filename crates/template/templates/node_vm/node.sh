#!/bin/bash
# Starts the node's container, and reports to sherpad that it started or why it did not. The
# first time, it creates the container from the arguments in /etc/sherpa/create, separated by
# NUL bytes, and copies the files of each writable volume into it, each line of
# /etc/sherpa/writable a file or folder on the config drive and its path in the container.
# Later boots start the container it created, with what it wrote.
set -eEuo pipefail

fail() {
  /opt/sherpa/report "failed: $1" || true
  exit 1
}
trap 'fail "line $LINENO: $BASH_COMMAND"' ERR

name=$(cat /etc/sherpa/container)
if ! docker container inspect "$name" > /dev/null 2>&1; then
  mapfile -d '' args < /etc/sherpa/create
  output=$(docker create "${args[@]}" 2>&1) || fail "cannot create the container: $output"
  # The files only, made writable: the folders they go into keep their owner and mode, and
  # Docker makes any that are missing.
  stage=$(mktemp -d)
  while read -r source destination; do
    mkdir -p "$stage$(dirname "$destination")"
    cp -R "$source" "$stage$destination"
  done < /etc/sherpa/writable
  chmod -R u+w "$stage"
  output=$( (cd "$stage" && find . -type f -print0 |
    tar --null --no-recursion --owner=0 --group=0 -cf - -T - |
    docker cp - "$name:/") 2>&1) || fail "cannot copy the writable volumes into the container: $output"
  rm -rf "$stage"
fi
output=$(docker start "$name" 2>&1) || fail "cannot start the container: $output"
/opt/sherpa/report started || true
