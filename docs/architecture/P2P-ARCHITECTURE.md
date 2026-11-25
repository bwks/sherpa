# P2p Link Architecture

Every `[[links]]` link is a P2p link: an eBPF TC program on each end's host interface redirects
every frame to the other end, so the link passes all L2 protocols, STP, LLDP, LACP and pause
frames included. This document owns how links are plumbed, re-attached, repaired and given
carrier; the behaviour is in [`tests/specs/links-and-bridges.md`](../../tests/specs/links-and-bridges.md).

## How It Works

Each end has a host interface: a tap for a VM or unikernel, a veth for a container, and a tap
Cloud Hypervisor makes for a container that runs in a microVM ([`MICROVM.md`](MICROVM.md)). On each,
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

A container on at least one link or private bridge gets every data interface as a veth, and
Docker handles only its management interface: mixing Docker networks with injected interfaces
breaks Docker's `ethN` naming. A bridge member's host end, `cb<node>i<interface>-<lab>`, is a port
of the bridge's Linux bridge, so a bridge has a port, and carrier, for each running container
member, and every member gets its frames whatever MAC address its NOS uses. A macvlan would give
neither: a bridge of only macvlans has no port and no carrier, and a macvlan delivers unicast
only to its own MAC address. A container on no link and no bridge takes its disabled interfaces
as Docker macvlan networks on its isolated bridge. `network::create_veth_in_netns` makes each veth straight into the container's namespace
with its final name, so nothing runs inside the container and images with no shell or `ip` work.

During `up`, a container's veths are made once it has started, and each link is attached as soon
as both its ends exist (`up::links::attach_ready_p2p_links`). A container that exits takes its
namespace, and with it its veths' host ends, so an end made earlier can be gone when a later
container's start attaches its link: at the lookup, or as `No such device` at the attach.
Either way the end is looked up again, by state, never by the error's text. A gone end whose
container no longer runs fails that container as a node, with its exit code and log tail, and
its links stay unattached while the rest of the lab carries on. A gone end whose container still
runs fails `up`, naming the end and its node.

A disabled interface's veth is down at both ends, so it has no carrier even when the NOS brings
it up; a VM's sits on the node's isolated network, whose bridge is set down.

### Transmit checksum offload

A veth leaves a TCP or UDP checksum for the "hardware" to fill in, and a program reading the
interface through a packet socket, as Cisco IOL does, gets the unfinished packet. For a model
whose `ModelKind::needs_host_checksums` is true, transmit checksum offload goes off on the host
end of every veth facing the container: `p2p_links::make_veth` for data veths, link ends and
bridge members alike, and `node_ops::finish_management_checksums` for the management veth, which
Docker makes again at every start. IOL's MTU
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
container's veths, its bridge members' host ends included, so a container on a bridge and no link
is repaired too), or one with no redirect filter whose peer exists (libvirt's new, bare tap).
A bridge member from an older release is a macvlan: the container has the interface and its host
end does not exist. The scanner leaves it alone, logs its lab once at info level, and the node
gets a veth when it is redeployed.
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
