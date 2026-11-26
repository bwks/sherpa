#!/bin/bash
# Names this node VM's NICs as the network OS expects, each found by its MAC in
# /etc/sherpa/interfaces, gives each data interface on no link a veth with the interface's
# fixed MAC, whose peer stays down in a namespace of its own, so the interface has no carrier,
# and gives the management NIC the node's addresses and default routes in
# /etc/sherpa/management, as Docker gives a container its network's.
# Runs once per boot, before Docker starts the node's container: the container runs in the VM's
# network namespace, and its network OS owns the interfaces from then on.
set -u

# How long a NIC has to appear after boot.
WAIT_SECONDS=60
UNWIRED_NETNS=sherpa-unwired

# The name of the NIC with MAC $1, once the kernel has it.
nic_of() {
  local dev waited=0
  while [ "$waited" -lt "$WAIT_SECONDS" ]; do
    for dev in /sys/class/net/*; do
      if [ "$(cat "$dev/address")" = "$1" ]; then
        echo "${dev##*/}"
        return 0
      fi
    done
    sleep 1
    waited=$((waited + 1))
  done
  return 1
}

# A temporary name first, so a NIC never takes a name another NIC still has.
index=0
while read -r mac want; do
  index=$((index + 1))
  if ! dev=$(nic_of "$mac"); then
    echo "No NIC with MAC $mac for $want" >&2
    continue
  fi
  ip link set dev "$dev" down
  ip addr flush dev "$dev"
  ip link set dev "$dev" name "sln$index"
done < /etc/sherpa/interfaces
index=0
while read -r mac want; do
  index=$((index + 1))
  ip link set dev "sln$index" name "$want" && ip link set dev "$want" up
done < /etc/sherpa/interfaces

while read -r dev address gateway; do
  case "$address" in
    *:*)
      ip -6 addr replace "$address" dev "$dev" nodad &&
        ip -6 route replace default via "$gateway" dev "$dev"
      ;;
    *)
      ip addr replace "$address" dev "$dev" &&
        ip route replace default via "$gateway" dev "$dev"
      ;;
  esac
done < /etc/sherpa/management

ip netns add "$UNWIRED_NETNS" 2>/dev/null
index=0
while read -r mac want; do
  index=$((index + 1))
  ip link add "$want" address "$mac" mtu 9600 type veth \
    peer name "unw$index" mtu 9600 netns "$UNWIRED_NETNS" &&
    ip link set dev "$want" up
done < /etc/sherpa/placeholders
