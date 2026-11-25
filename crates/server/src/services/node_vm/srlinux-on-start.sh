#!/bin/bash
# SR Linux moves its management interface into its own namespace, where it sets the address
# from its config; Docker's copy of the address in the container's namespace is flushed, so the
# address answers once. Docker sets it again at every start of the container.
set -u

docker exec "$1" sh -c 'for i in $(seq 1 120); do
  ip netns list 2>/dev/null | grep -q srbase-mgmt && break
  sleep 1
done
ip addr flush dev mgmt0'
