#!/bin/bash
# Moves this node VM's data NICs into the node container's network namespace, named as the
# network OS expects, each time the container starts, then runs the model's own start step. A
# NIC returns to the VM when the container's namespace goes, so it is moved again at the next
# start.
set -u

name=$(cat /etc/sherpa/container)

# Move the NICs into the running container; fails when the container does not run.
move_nics() {
  local pid index=0 mac want dev
  pid=$(docker inspect -f '{{.State.Pid}}' "$name" 2>/dev/null) || return 1
  [ "${pid:-0}" -gt 0 ] || return 1
  while read -r mac want; do
    index=$((index + 1))
    for dev in /sys/class/net/*; do
      [ "$(cat "$dev/address")" = "$mac" ] || continue
      dev=${dev##*/}
      # A temporary name first, so a NIC never takes a name another NIC still has.
      ip link set dev "$dev" down
      ip link set dev "$dev" name "smv$index"
      if ip link set dev "smv$index" netns "$pid"; then
        nsenter -t "$pid" -n ip link set dev "smv$index" name "$want"
        nsenter -t "$pid" -n ip link set dev "$want" up
      fi
    done
  done < /etc/sherpa/interfaces
}

# The model's own step after each start, such as SR Linux's management address flush.
on_start() {
  if [ -x /opt/sherpa/on-start ]; then
    /opt/sherpa/on-start "$name" &
  fi
}

# A container Docker started with the VM runs before this script listens for starts.
if move_nics; then
  on_start
fi
docker events --filter "container=$name" --filter event=start --format '{{.Actor.ID}}' |
  while read -r _; do
    if move_nics; then
      on_start
    fi
  done
