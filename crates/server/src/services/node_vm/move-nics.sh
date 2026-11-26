#!/bin/bash
# Moves this node VM's data NICs into the node container's network namespace, named as the
# network OS expects, each time the container starts, and gives the container each data
# interface that is on no link as a veth with the interface's fixed MAC, whose peer stays down
# in the VM, so it has no carrier.
# Then runs the model's own start step. A NIC returns to the VM when the container's namespace
# goes, and a veth goes with it, so both are made again at the next start.
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
  index=0
  while read -r mac want; do
    index=$((index + 1))
    nsenter -t "$pid" -n ip link show dev "$want" >/dev/null 2>&1 && continue
    ip link del dev "spp$index" 2>/dev/null
    ip link add "sph$index" address "$mac" mtu 9600 type veth peer name "spp$index" mtu 9600 ||
      continue
    if ip link set dev "sph$index" netns "$pid"; then
      nsenter -t "$pid" -n ip link set dev "sph$index" name "$want"
      nsenter -t "$pid" -n ip link set dev "$want" up
    fi
  done < /etc/sherpa/placeholders
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
