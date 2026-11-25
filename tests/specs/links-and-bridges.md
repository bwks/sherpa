| 016 | A container member from an older release, an interface in the container with no host end, is not repaired by any scan, and its lab is logged once at info level | unit |
# Links and bridges

What a lab's data plane does: point-to-point links between virtual machines, containers and unikernels, what happens to a link when the node at one end stops or is made again, link impairment, private bridges, where each interface of a node is and what an unused one looks like, the management network as far as it carries nodes' traffic, and what destroy and redeploy leave on the host. Whether a lab with links comes up at all is `lab_up_003` in [lab lifecycle](lab-lifecycle.md); how a node boots, stops and starts is in [nodes](nodes.md); which link, bridge and interface settings a manifest may hold, and the refusal of one that is wrong, are in the Manifests spec; who may change a lab is in [accounts and auth](accounts-and-auth.md); the address ranges a lab gets, and its nodes' addresses in them, are `lab_addressing` in lab lifecycle; the lab router's DHCP and DNS, the Tailscale gateway and how `sherpa inspect` and the web UI lay a lab out, its Links table included, have their own specs.

A link is one `[[links]]` entry: two ends, `src` and `dst`, each an interface of a node. Each end has an interface on the Sherpa server, its host end: `tpa<link>-<lab>` for `src` and `tpb<link>-<lab>` for `dst`.

Each behaviour lists its cases. A case is one test, whose name is its ID: `<behaviour>_<NNN>_<description>`, such as `link_carrier_002_a_stopped_vm_takes_carrier_from_its_container_peers_interface`. The level is the highest that can show the case: e2e through the `sherpa` CLI, integration against the real kernel, libvirt, Docker and the database, unit only for logic with too many branches to cover above.

## `p2p_link`: a link carries every frame between its two ends

**Statement:** a link carries every Ethernet frame from each end to the other, unchanged, whatever kinds of node it joins.
**Relied on by:** the user, the network operating systems and protocols run across a lab. **Priority:** P0.

| Case | What it shows | Level |
|---|---|---|
| 001 | LLDP, STP, pause, LACP, 802.1X, GARP-range, CDP, 802.1Q and QinQ frames arrive at the other end, each way, tags intact, on a VM to VM, a VM to container and a container to container link | e2e |
| 002 | Two ends joined by two links see each other as LLDP neighbours on both and bundle both with LACP, between two VMs, a VM and a container, and two containers. A VM's virtio link interface reports unknown speed and duplex, and a Linux 802.3ad bond sends no LACPDU from a member that is not full duplex, so the guest sets each member to full duplex before bonding it (`ethtool -s <port> speed 10000 duplex full`); Sherpa does not set it | e2e |
| 003 | Every host end, a VM's tap included, is MTU 9600, and unfragmented 8972-byte pings and 9000-byte frames pass each way | e2e |
| 004 | A VM is told its link's MTU, so a virtio interface in the guest comes up at 9600 (no test at this level yet) | e2e |
| 005 | A capture on a link's host end, `tpa<link>-<lab>` or `tpb<link>-<lab>`, sees every frame from both ends | e2e |
| 006 | A container's link interface is in the container under its name, up, promiscuous, with carrier and MTU 9600, also in an image with no `ip` command or no shell | e2e |
| 007 | Network OS, application and OS containers reach each other across links, directly and routed through a third node | e2e |
| 008 | A link with a unikernel end carries traffic (no test yet) | e2e |
| 009 | Links carry traffic across a restart of sherpad, with nothing attached again | e2e |
| 010 | sherpad keeps no open file descriptor for a link it attached | integration |
| 011 | Attaching a link that is attached leaves one redirect on each end | integration |
| 012 | A link whose host end does not exist is not attached, and the error carries the interface | integration |
| 013 | A container interface with the name of an interface on the server, such as `eth1`, is still made | integration |
| 014 | A container interface whose host end's name is held by an interface that is not a removed container's is refused, and that interface is left | integration |
| 015 | A container interface asked for a container that is gone fails, and leaves no host end behind | integration |
| 016 | A unikernel's link interface is the link's host end with the link's MTU | unit |
| 017 | During `up`, a container that exits after its link ends are made, whose end is gone when a peer's link is looked up, fails as a node with its exit code, its links stay unattached, and the rest of the lab is attached and `up` carries on | unit |
| 018 | During `up`, a container whose end goes between its lookup and its attach (`No such device`) fails as a node, its link stays unattached, and `up` carries on | unit |
| 019 | During `up`, a link end gone while its container still runs fails `up`, carrying the end and its node | unit |

## `link_reattach`: a link works again after one of its ends is made again

**Statement:** when a node's link ends are made again, by a resume, a redeploy or a start outside Sherpa, its links carry traffic again in both directions with their impairments, without the user doing anything.
**Relied on by:** the user. **Priority:** P0.

A VM or unikernel that boots again, a container started again and a redeployed node get new link ends. A reboot from inside the guest, a pause and an unpause keep them.

Cases 016, 018 and 019 show a repair on a real host. Cases 030 to 032 show what the scan decides given the same host state, on a fake host, where no root, Docker or libvirt is needed.

| Case | What it shows | Level |
|---|---|---|
| 001 | After redeploying a VM that ends two links, pings pass each way on every link | e2e |
| 002 | After redeploying a container that ends two links, pings pass each way on every link | e2e |
| 003 | After down and resume of the lab, pings, the L2 frames, capture and the LACP bundles work as after up | e2e |
| 004 | A reboot from inside a VM's guest leaves its links working | e2e |
| 005 | Redeployed containers with no shell or no `ip` command, and a redeployed router, reach every peer again | e2e |
| 006 | A cold-booted VM or unikernel has each of its links attached to its new ends, in both directions | unit |
| 007 | A container started again gets every data interface again, link ends, bridge members and unused interfaces, before its links are attached | unit |
| 008 | A container of a lab with no stored manifest gets as many data interfaces as its image has | unit |
| 009 | A container on no link and no bridge keeps its Docker networks when it is started again, and no interface is made for it | unit |
| 010 | A link whose peer is down waits for the peer, and the node's other links are attached | unit |
| 011 | When the peer comes back, the link is attached in both directions | unit |
| 012 | A link that fails does not stop the node's other links, and does not fail the node | unit |
| 013 | A container interface that cannot be made, a container whose network namespace cannot be found, and a node whose interfaces cannot be worked out each fail the node's links with the reason | unit |
| 014 | Each direction's stored impairment is set again on the host end it leaves through | unit |
| 015 | With real interfaces, a re-attach passes frames again both ways, and leaves a link whose peer is down waiting | integration |
| 016 | A container restarted with `docker restart` has its interfaces and links back after one scan | integration |
| 017 | A container stopped with `docker stop` is left alone with its links waiting, and its links are back one scan after `docker start` | integration |
| 018 | A VM destroyed and started with `virsh` has its links back after one scan | integration |
| 019 | The first scan after sherpad starts repairs what changed while it was down, and the next scan changes nothing | integration |
| 020 | A scan of a lab in which nothing changed makes and attaches nothing | unit |
| 021 | A scan makes only the interfaces of a restarted container that are missing, and leaves its working ones | unit |
| 022 | A node that is stopped or paused, by Sherpa or outside it, is not repaired, and its links wait for it | unit |
| 023 | A new, bare end whose peer is down is left until the peer runs | unit |
| 024 | A repair that fails does not stop the repair of other nodes, and is tried again at the next scan | unit |
| 025 | A running VM whose tap is missing, and an end that cannot be read, fail the link, and nothing is made in their place | unit |
| 026 | A link between a container stopped outside Sherpa and its peer works again after `sherpa resume` | e2e |
| 027 | A redeployed container's interface is made again while the removed container's old one, of the same name, is still on the host | integration |
| 028 | Only a filter running Sherpa's redirect counts as a link's redirect, so an end with another filter is still found bare | unit |
| 029 | A new interface has no redirect until a link is attached to it | integration |
| 030 | Given a VM started outside Sherpa, whose new tap has no redirect while its peer's still points at the old one, a scan attaches the link in both directions | unit |
| 031 | Given a container restarted outside Sherpa with none of its data interfaces left, a scan makes its link ends and unused interfaces again and attaches its links, and leaves the lab's other links as they were | unit |
| 032 | Given a VM and a container both changed with nothing remembered of the lab, one scan repairs both, and the next repairs nothing and writes nothing to the host | unit |
| 033 | An end that cannot be read is logged once at warn with a reference that its link's reason gives on every scan, and once at info when it can be read again; the scans between log nothing more | unit |

## `link_state`: each link reports whether it is attached

**Statement:** each link reports whether it is attached, waiting for a node that is down, or failed, and why.
**Relied on by:** the user, scripts reading `sherpa inspect`. **Priority:** P1.

| Case | What it shows | Level |
|---|---|---|
| 001 | A link with both ends is `attached` | unit |
| 002 | A link whose peer's end is missing is `peer down`, naming the peer | unit |
| 003 | A link whose own node's end is missing is `failed`, also when its peer is down | unit |
| 004 | An end that could not be made or read makes the link `failed` with the reason | unit |
| 005 | A link between two interfaces of one node is `attached` only with both | unit |
| 006 | `sherpa inspect` gives each link its index and the state the server last found or left it in, and none for a link not checked since sherpad started | unit |
| 007 | A link from a server that sends no state or no index still reads, with neither | unit |
| 008 | `sherpa resume` and `sherpa redeploy` list each link that is not attached, a waiting one as information and a failed one as a warning, and exit zero (no test at this level yet) | e2e |
| 009 | After `sherpa down --node` of one end, `sherpa inspect` shows the link `peer down: <node>` within a scan interval, and `attached` again after the resume (no test yet) | e2e |
| 010 | The nodes counted as running are those libvirt and Docker report running, so a paused node is down | unit |
| 011 | A lab's links are its own: another lab's are not listed with them | integration |

How `sherpa inspect` prints a link's state in its Links table is for the Inspect and web UI spec.

## `link_carrier`: a link end has carrier only while its link is attached

**Statement:** an interface on a link loses carrier when the node at the other end stops or the link fails, and gets it back when the link is attached again, as a cable's would.
**Relied on by:** the user, routing protocols in the lab that react to an interface going down. **Priority:** P1.

| Case | What it shows | Level |
|---|---|---|
| 001 | A container stopped outside Sherpa takes the link down inside its VM peer's guest within one scan | integration |
| 002 | A VM stopped outside Sherpa takes carrier from its container peer's interface within one scan | integration |
| 003 | A container unpaused by `sherpa resume` gives its VM peer carrier before the command returns | integration |
| 004 | An attached link has carrier at both ends; one waiting for a peer has none at the running node's end and leaves the peer's end alone; a failed one has none at either end | unit |
| 005 | A scan that finds a link in the state it had moves no carrier, so an end a user took down by hand stays down | unit |
| 006 | When a link's state changes, only the ends whose carrier should change are set | unit |
| 007 | A paused peer counts as down | unit |
| 008 | With both nodes down, a node that comes back before its peer has no carrier | unit |
| 009 | The first scan after sherpad starts sets the carrier that is wrong, and does not flap an end that is right | unit |
| 010 | A VM cold-booted while its peer is still down has its new end taken down | unit |
| 011 | An end that does not exist, or whose carrier cannot be set, does not stop the others | unit |
| 012 | A resume decides the links of the nodes it started only: a node started beside a paused peer has its link taken down at once, and other links are left | unit |
| 013 | A link with an end to repair is left to the scan, and nothing is decided when libvirt or Docker did not answer | unit |
| 014 | A VM's link is taken down and brought up in its live domain only, with everything else about the interface and every other interface as it was | unit |
| 015 | A VM's interface is read as up unless its link state says down, and a domain that is not well formed, or has no such interface, is refused | unit |
| 016 | A redeployed node's peers keep their carrier while it is replaced (no test yet) | e2e |
| 017 | With the scanner disabled, `sherpa down` leaves the peers' carrier, and `sherpa resume` still sets it (no test yet) | integration |
| 018 | Given a link that goes from attached to waiting for its container end, the VM peer's link is taken down through libvirt, and brought up again when the link is attached | unit |
| 019 | Given a link that goes from attached to waiting for its VM end, the container peer's host end is taken down, and it alone is brought up when the link is attached again | unit |
| 020 | Given a paused container whose VM peer's link a scan took down, a resume that unpauses it brings the VM's link up at once, and the next scan moves no carrier | unit |

Cases 001 to 003 show carrier move on a real host. Cases 018 to 020 show what is decided given the same host state, on a fake host.

## `link_impair`: an impairment applies to one direction of a link and stays

**Statement:** an impairment set on a direction of a link delays, drops, reorders or corrupts traffic in that direction only, stays until it is changed, and a change that fails leaves the one before.
**Relied on by:** the user. **Priority:** P1.

| Case | What it shows | Level |
|---|---|---|
| 001 | A manifest's `[links.impairment]` is on both directions after up, and the round-trip time shows it | e2e |
| 002 | `sherpa link impair --direction src-to-dst` delays traffic from `src` to `dst` only, and `dst-to-src` the other way only, on a VM to VM, a VM to container and a container to container link | e2e |
| 003 | An impairment with reorder and corruption on both directions is set, and the link still carries traffic | e2e |
| 004 | `--clear` removes the impairment, and the round-trip time is as without one | e2e |
| 005 | An impairment is as it was set after redeploying either end, after down and resume, and after a restart of sherpad | e2e |
| 006 | `sherpa link impair` finds the link by either of its ends among the lab's links, with no manifest, and refuses an end on no link | unit |
| 007 | Against a server that gives no link indexes the link is found in the folder's manifest, and with no manifest the command says the server is too old | unit |
| 008 | A request with no direction changes both, and a value left out is 0 | unit |
| 009 | A delay or jitter over 274877 ms, or a percentage outside 0 to 100, is refused naming the field, and one within is taken | unit |
| 010 | A number too large for its field in a request is refused, not wrapped to a small one | integration |
| 011 | A change the kernel refuses on the second end is undone on the first, and nothing is recorded | unit |
| 012 | A change that cannot be recorded is undone in the kernel | unit |
| 013 | A clear whose removal fails is an error and is not recorded; a clear of a direction with no impairment succeeds | unit |
| 014 | A changed impairment replaces the one before on a real interface, and a clear leaves no netem there | integration |
| 015 | Only the impairment Sherpa set, the root netem of the link's own host end, is removed by a clear | unit |
| 016 | Reorder and corruption reach netem with the percentage asked, alone and together, and a delay netem cannot hold is refused | unit |
| 017 | A change to a link whose node is down is refused saying its interface does not exist (no test yet) | integration |
| 018 | `sherpa link impair` with no value and no `--clear` is refused (no test yet) | unit |
| 019 | An impairment of jitter, reorder or corruption alone is set, not taken for a clear | unit |
| 020 | Recording a link's impairments leaves the rest of the link as it was, and a link that is gone is refused | integration |
| 021 | Given a change to one direction, the impairment is set on the host end that direction leaves through, and the other direction's is left as it was | unit |

## `private_bridge`: the members of a bridge share one segment

**Statement:** the interfaces named in a `[[bridges]]` entry are in one layer 2 broadcast domain, with nothing else in it.
**Relied on by:** the user. **Priority:** P1.

A container member, on a link or not, takes every data interface as a veth made in its namespace, and its member's host end, `cb<node>i<interface>-<lab>`, is a port of the bridge. A VM member joins through its tap.

| Case | What it shows | Level |
|---|---|---|
| 001 | A VM, containers on a link and a container on no link, all members of one bridge, each ping every other member from their member interface | e2e |
| 002 | The same after redeploying a member container on a link, one on no link, and the member VM | e2e |
| 003 | A node with a bridge member interface and a link has both working, each on its own interface | e2e |
| 004 | The members of one bridge are not reachable from another bridge of the lab, from the management network or from another lab (no test yet) | e2e |
| 005 | A bridge on the host is up with MTU 9600 | integration |
| 006 | A member container joins through an interface made in its namespace, and its host end is in the bridge | integration |
| 007 | Retired: a member container on no link no longer gets a Docker macvlan network on the bridge; see 013 | |
| 008 | A bridge is recorded with its members or not at all, and its members go with the lab | integration |
| 009 | An interface is recorded in one link or one bridge only | integration |
| 010 | Members keep reaching each other after down and resume, and after a member container is restarted outside Sherpa (no test yet) | e2e |
| 011 | A bridge whose name is taken is not made over the one there, and a member whose interface or bridge does not exist fails to join, each with an error | integration |
| 012 | A bridge whose members are only containers on no link carries traffic between them | integration |
| 013 | A container member on no link takes every data interface as a veth in its namespace, and gets no Docker data network; a VM member does not | unit |
| 014 | A container on a bridge and no link that is restarted outside Sherpa has its interfaces made again by one scan, the member's host end in the bridge; the next scan makes nothing, and a stopped one is left alone | unit |
| 015 | A container on a bridge and no link restarted with `docker restart` reaches the other members again after one scan | integration |

## `node_interfaces`: every interface of a node is where its name says

**Statement:** a node has its management interface first, then its model's reserved interfaces, then its data interfaces in the order of their names, and a data interface on no link and no bridge is there but not connected.
**Relied on by:** the user, the node's own configuration, which names interfaces. **Priority:** P0.

| Case | What it shows | Level |
|---|---|---|
| 001 | The interface a manifest names is that interface inside the node, for each model and kind: the name gives its place, and the place gives the name back | unit |
| 002 | A container's data interfaces are named inside it as its model needs: `eth<N>` by default, SR Linux's `e1-<N>`, `eth4` upwards on cJunosEvolved, `eth<N>` for port N of Cisco IOL | unit |
| 003 | A VM's interfaces are its management interface, its reserved interfaces, then each data interface on its link, on its bridge, or unused, at up and after a redeploy | unit |
| 004 | An unused interface of a container on a link is in the container under its name with no carrier | e2e |
| 005 | An unused interface of a VM is in the guest with its link down (no test at this level yet) | e2e |
| 006 | An unused container interface is down at both ends, except on a model that needs its interfaces up, where it is up in the container with no carrier | integration |
| 007 | An unused interface of a container on no link and no bridge gets a Docker network of its own, named and down | unit |
| 008 | A node's data interfaces number as many as its manifest or image says, after its reserved ones, and a count past the model's range is an error | unit |
| 009 | A VM's interfaces are named `eth0`, `eth1` in a guest that keeps kernel names | e2e |
| 010 | A VM's reserved interfaces are on a network of the node's own with their link up, and its unused ones on another with ports kept apart | integration |
| 011 | An interface name reaches the command that renames it in a container as an argument, never as script | unit |
| 012 | Only a model that waits for its interfaces at boot (cJunosEvolved) has its unused interfaces up in the container, and a linked interface is up on every model | unit |
| 013 | A container interface on a Docker network that its model renames is given that name in the container, with its address, and one its model does not rename keeps the name Docker gives it | unit |
| 014 | Only an active container interface that its model renames runs a command in the container, which sets it promiscuous and up; one that is down or not renamed runs none, so an image with no `ip` starts | unit |

## `veth_checksums`: a node that reads raw packets gets finished checksums

**Statement:** a container of a model that reads its interfaces through packet sockets (`cisco_iol`, `cisco_ioll2`) receives TCP and UDP packets with finished checksums on every interface, so TCP to it works.
**Relied on by:** the user, SSH and provisioners into the node. **Priority:** P2.

| Case | What it shows | Level |
|---|---|---|
| 001 | TCP across a link into a `cisco_iol` node works, at up and after a cold boot | e2e |
| 002 | Turning transmit checksum offload off on a host end turns checksum and segmentation offload off, and it reads as off | integration |
| 003 | A new interface reads as offload on until it is turned off | integration |
| 004 | Offload reads as on while any transmit checksum feature is active, and off with none | unit |
| 005 | A kernel answer that is an error, cut short, with no features, or for another request is an error, never read as offload off | unit |
| 006 | Turning offload off on an interface that does not exist fails | integration |
| 007 | The host end of a container's management interface is found from inside its namespace | integration |
| 008 | Each scan turns offload off on the management interface of a running node that needs it, only when it is on, so a container started outside Sherpa is put right | unit |
| 009 | A node whose management interface cannot be found does not stop the scan of the others | unit |
| 010 | SSH to the management address of a `cisco_iol` node restarted with `docker restart` works again within one scan (no test yet) | e2e |
| 011 | Ping and TCP into a `cisco_iol` node across a private bridge it has no link on work (no test yet) | e2e |
| 012 | A `cisco_iol` container on a bridge and no link gets its member and unused interfaces as veths, each host end with offload off, and another model's keep it on | unit |

## `management_network`: every node is reachable on its lab's management network

**Statement:** every node of a lab has a management address on a network of the lab's own, and reaches the server and the lab's other nodes there.
**Relied on by:** the user, SSH and provisioners, ZTP, the Tailscale gateway. **Priority:** P0.

| Case | What it shows | Level |
|---|---|---|
| 001 | A VM and a container of one lab reach each other and the server on their management addresses, and each reaches the internet through the server (no test yet) | e2e |
| 002 | The management bridge runs no spanning tree (no test at this level yet) | integration |
| 003 | A lab's management network is made in libvirt as an active NAT network, in IPv4 alone or with IPv6, and making a libvirt network that is already there succeeds | integration |
| 004 | A node's management interface passes traffic from the moment it starts, with no four-second wait (no test yet) | e2e |
| 005 | A management network Docker cannot make is reported, not passed over | integration |
| 006 | Making a lab's Docker management network when it is already there, with the same subnets and bridge, succeeds | integration |
| 007 | A unikernel's management interface is on its lab's management network, with the MAC address Sherpa gave it | unit |
| 008 | A Docker management network that is already there with the lab's subnets, gateways and bridge is reused | unit |
| 009 | A Docker management network of the lab's name with other subnets, gateways or another bridge refuses the up before anything of the lab is created, naming the network and each difference | integration |
| 010 | That network is left as it was: the refused up and its rollback neither remove it nor make it again | integration |
| 011 | A network that differs which Docker reports as already there when the up makes it fails the creation with an error that carries each difference, with the value expected and the value found, and nothing is removed | unit |

Which ranges a lab's management network takes, and which addresses its nodes get in them, are `lab_addressing` in [lab lifecycle](lab-lifecycle.md).

## `link_cleanup`: destroy and redeploy leave nothing of theirs on the host

**Statement:** destroy removes every host interface, bridge and network of the lab, and redeploy those of the node, and neither touches another lab's or another node's.
**Relied on by:** the user, the host, the next lab to take the same names. **Priority:** P0.

| Case | What it shows | Level |
|---|---|---|
| 001 | After destroying a lab of VM and container links, no host interface, Docker network or libvirt network of the lab is left | e2e |
| 002 | After destroying a lab with a private bridge and containers on no link, no bridge, Docker network or libvirt network of the lab is left | e2e |
| 003 | Only the lab's own host interfaces are deleted: one of another lab is left, also when its name contains this lab's ID | unit |
| 004 | A redeploy takes only its own node's Docker networks: those of a node whose name starts with this node's name are left | unit |
| 005 | Deleting an interface that is already gone succeeds, so a destroy can be run again | integration |
| 006 | A Docker network still in use is reported as not removed, and one already gone counts as removed | integration |
| 007 | A host interface or network that cannot be removed is reported by destroy, which exits non-zero (no test yet) | integration |
| 008 | After destroying a lab of containers only, no host interface or Docker network of the lab is left | e2e |
