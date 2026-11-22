# P2p Link Architecture

Every `[[links]]` link is a P2p link: an eBPF TC program on each end's host interface redirects
every frame to the other end, so the link passes all L2 protocols, STP, LLDP, LACP and pause
frames included. This document owns how links are plumbed, re-attached, repaired and given
carrier; the behaviour is in [`tests/specs/links-and-bridges.md`](../../tests/specs/links-and-bridges.md).

## How It Works

Each end has a host interface: a tap for a VM or unikernel, a veth for a container. On each,
`network::attach_p2p_redirect` attaches one shared program to ingress, with the peer's ifindex
in its map, and it redirects every frame to the peer's egress. Nothing sits between the ends. A
VM's tap is pre-made by libvirt (`<interface type='ethernet'>`) with an MTU of 9600: a redirect
does not check the target's MTU, but the guest must be told it.

The program is attached as a legacy netlink TC filter, never through TCX: a TCX link lasts only
while `sherpad` holds its descriptor, so a restart would drop every redirect, and the detach a
re-attach does first cannot see a TCX program. The filter holds the program and its map in the
kernel, with no descriptor in `sherpad`, until the interface is deleted.

### Re-attaching after an end is made again

A cold boot gives a VM or unikernel new taps, a container started from exited gets a new
namespace with none of its veths, and a redeploy makes the node's ends again. Each new end has a
new ifindex, so the peer's redirect is stale. Every such path goes through one function,
`p2p_links::reattach_node_links`, working from the database rows: it makes a container's data
interfaces again, then for each link points both redirects at each other and syncs both
impairments, or leaves the link `PeerDown` for the peer's own re-attach, or `Failed` with the
reason; none fails the node. Unpausing makes no end, so resume and redeploy then decide the
links of the nodes they started as the scanner would (`p2p_links::follow_started_links`).

### Link Impairment (TC netem)

The redirect on the `clsact` ingress hook coexists with a root `netem` qdisc on egress. A frame
from A to B leaves through B's host end, so netem there impairs A to B. Each direction is stored
and applied on its own; resume and redeploy set both from the database (`network::sync_netem`),
and a direction is never recorded clear while the kernel still has its qdisc.

## Container-to-Container

A container on at least one link gets every data interface as a veth, and Docker handles only
its management interface: mixing Docker networks with injected interfaces breaks Docker's `ethN`
naming. A container on no link takes its bridge members and disabled interfaces as Docker macvlan
networks. `network::create_veth_in_netns` makes each veth straight into the container's namespace
with its final name, so nothing runs inside the container and images with no shell or `ip` work.

A disabled interface's veth is down at both ends, so it has no carrier even when the NOS brings
it up; a VM's sits on the node's isolated network, whose bridge is set down.

### Transmit checksum offload

A veth leaves a TCP or UDP checksum for the "hardware" to fill in, and a program reading the
interface through a packet socket, as Cisco IOL does, gets the unfinished packet. For a model
whose `ModelKind::needs_host_checksums` is true, transmit checksum offload goes off on the host
end of every veth facing the container: `KernelHost::create_veth` for data veths, and
`node_ops::finish_management_checksums` for the management veth, which Docker makes again at
every start. An IOL node on no link has macvlan interfaces, with no host end to change. IOL's MTU
is 1500, so a Linux peer must use 1500 on that link, or its larger TCP MSS stalls TCP.

### Teardown

Destroy deletes every host interface with a lab prefix ending in exactly `-<lab_id>`
(`destroy::is_lab_interface`), with its container end and filter; libvirt removes the rest.

## Crash Resilience

When a node stops, its ends go with their filters and its peers' frames are dropped; a guest
reboot, a pause or a `sherpad` restart changes no redirect. A node started outside Sherpa is
repaired by the scanner within one scan.

### Repair by the scanner

A redirect is tied to an ifindex twice: the filter on the end's own interface, which dies with
it, and the peer's ifindex in the other end's map, which nothing updates. The scanner finds
broken ends of running nodes from the kernel alone: an end that does not exist (a restarted
container's veths), or one with no redirect filter whose peer exists (libvirt's new, bare tap).
`repair_node` then runs `reattach_node_links`, making only the missing interfaces. The map is
never read back, so a filter that survived with a stale peer ifindex cannot be seen. Each scan of
a `Ready` lab runs, under the lab hold, the management checksum offload, this repair, then carrier.

An end that cannot be looked up, or whose filter cannot be read, fails its link. Which ends are
unreadable, and what failed, is kept in memory (`AppState::unreadable_ends`), as the Vault seal
state is: the end is logged at `warn` with a reference when it becomes unreadable or what failed
changes, and at `info` when it can be read again. Each scan in between gives the link the same
reason and reference, and logs nothing.

### Carrier propagation

The carrier of a link's ends follows its state, as a cable's would (`p2p_links::carrier`). A
container end is set through its host veth, which the NOS cannot bring back up; a VM end through
libvirt's live link state (`libvirt::set_interface_link`), the only signal its guest sees.

Carrier is set only when a link's state changes, so an end a user took down by hand stays down.
Two cases set it anyway: nothing recorded for the link, as after a `sherpad` start, and an end
made again, which starts with carrier. Each end is read first and set only if it differs, so
neither flaps. With `scanner.enabled = false` only resume and redeploy set carrier.
