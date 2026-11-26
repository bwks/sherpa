#!/bin/bash
# Sends the status line "$*" to sherpad on the VM's status channel, newlines flattened, and logs
# it. Gives up after REPORT_SECONDS when sherpad is not listening, as when the VM boots again
# after down and resume.
set -eu

CHANNEL=/dev/virtio-ports/org.sherpa.status
REPORT_SECONDS=60

line=$(printf '%s' "$*" | tr '\r\n' '  ')
echo "$line" >&2
timeout "$REPORT_SECONDS" bash -c 'printf "%s\n" "$1" > "$2"' _ "$line" "$CHANNEL"
