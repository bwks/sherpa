#!/bin/bash
# Loads the node image's archive, the disk with serial sherpa-image, into this VM's Docker, whose
# data is the Docker store being made, reports to sherpad that it loaded or why it did not, and
# powers the VM off once it has.
set -eEu

ARCHIVE=/dev/disk/by-id/virtio-sherpa-image

fail() {
  /opt/sherpa/report "failed: $1" || true
  exit 1
}
trap 'fail "line $LINENO: $BASH_COMMAND"' ERR

output=$(docker load --input "$ARCHIVE" 2>&1) || fail "cannot load the image: $output"
/opt/sherpa/report loaded
systemctl --no-block poweroff
