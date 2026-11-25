#!/bin/bash
# Opens the node's network OS CLI on the VM's serial console when Enter is pressed, once its
# container runs, so the CLI starts with someone at the console. systemd starts it again when
# the CLI exits.
set -u

name=$(cat /etc/sherpa/container)
echo
echo "$(hostname) console: press Enter to open the network OS CLI"
read -r _
until [ "$(docker inspect -f '{{.State.Running}}' "$name" 2>/dev/null)" = true ]; do
  echo "Waiting for the node's container to start..."
  sleep 2
done
# A serial line has no size, and full-screen CLIs draw nothing in a 0x0 terminal.
if [ "$(stty size)" = "0 0" ]; then
  stty rows 24 cols 80
fi
exec docker exec -it "$name" "$@"
