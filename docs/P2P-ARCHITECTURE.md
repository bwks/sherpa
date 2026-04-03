# P2p Link Architecture

P2p (point-to-point) links provide protocol-transparent connections between nodes using eBPF TC classifier programs. Unlike bridge-based links (`PeerBridge`), P2p links pass all L2 protocols including STP, LLDP, LACP, and any other protocol that would normally be filtered by a Linux bridge's `group_fwd_mask`.

## How It Works

Each link endpoint has a host-side network interface (tap for VMs, veth for containers). An eBPF TC classifier program attached to each interface's ingress redirects all packets to the peer interface's egress. No bridges or intermediate devices are involved.

### eBPF Redirect Program

A single TC classifier BPF program is shared across all P2p links. For each interface:

1. A `clsact` qdisc is added to the interface
2. The BPF program is attached to the ingress hook
3. A BPF HashMap stores the peer's ifindex (key=0, value=peer_ifindex)
4. On every ingress packet, the program calls `bpf_redirect(peer_ifindex, 0)` to send the packet to the peer's egress

The BPF objects are intentionally leaked via `std::mem::forget()` so the TC filters persist in the kernel after the setup function returns. They are cleaned up when the interfaces are deleted during lab destroy.

### Link Impairment (TC netem)

Each interface supports two coexisting TC layers:

- **clsact qdisc** — ingress hook for the BPF redirect program
- **root netem qdisc** — egress hook for delay/loss/jitter simulation

Packet flow with impairment:

```
Node_A sends packet
  -> iface_a ingress (BPF: redirect to iface_b egress)
  -> iface_b egress (netem: adds configured delay/loss/jitter)
  -> delivered to Node_B
```

netem on iface_a's egress controls B->A impairment. netem on iface_b's egress controls A->B impairment. This gives per-direction impairment control.

## VM-to-VM

```
VM_A <-- tap_a --[ingress: BPF redirect -> tap_b]--> tap_b --[ingress: BPF redirect -> tap_a]--> VM_B
```

### Setup

1. The libvirt domain template uses `<interface type='ethernet'>` with a `<target dev='tap_name'/>` directive. This tells libvirt/QEMU to use a pre-named tap device instead of connecting to a bridge.
2. Libvirt creates the tap device when the VM starts.
3. After VM creation (Phase 11b), the eBPF redirect program is attached to each tap's ingress, wiring tap_a -> tap_b and tap_b -> tap_a.

### Teardown

Deleting the tap devices (via libvirt domain undefine/destroy) automatically detaches the BPF programs.

### Naming Convention

- Tap devices: `tpa{link_index}-{lab_id}` (side A), `tpb{link_index}-{lab_id}` (side B)

## Container-to-Container

```
Container_A <-- eth1 (netns) | veth_a_out --[BPF redirect -> veth_b_out]--> veth_b_out | eth1 (netns) --> Container_B
```

### Setup

1. A veth pair is created in the host namespace for each container endpoint:
   - Host side (e.g. `tpa0-{lab_id}`) — stays in host namespace, gets the eBPF redirect program
   - Container side (e.g. `cva0-{lab_id}`) — moved into the container's network namespace
2. Disabled (unlinked) interfaces also get veth pairs, but without eBPF. The host side is set DOWN to remove carrier, so the container sees the interface as "not connected".
3. After the container starts (Phase 13), the container's PID is obtained via Docker inspect.
4. Each container-side veth is moved into the container's network namespace using `ip link set {veth} netns {pid}`.
5. Inside the container, the veth is renamed to the correct interface name (e.g. `eth1`, `e1-1`) and brought UP with promiscuous mode enabled.
6. The eBPF redirect program is attached to each host-side veth's ingress.

### Docker Networking Bypass

P2p containers bypass Docker's network stack entirely for data interfaces. Docker only handles the management interface. All data interfaces (both linked and disabled) are created as veth pairs and moved into the container's netns manually. This avoids Docker's ethN naming conflicts that occur when mixing Docker networks with manual netns interface injection.

### Disabled Interface Carrier Removal

For disabled container interfaces, the host-side veth is set DOWN after the container-side is moved into the netns. Since a veth pair shares carrier state, setting the host side DOWN causes the container side to lose carrier, making the interface appear as "not connected" to the container NOS.

### Teardown

Host-side veths are cleaned up during lab destroy by matching the `tpa*`, `tpb*`, `cv*`, `cd*`, and `ce*` prefixes via fuzzy interface search.

### Naming Convention

- Host-side veths (linked): `tpa{link_index}-{lab_id}` / `tpb{link_index}-{lab_id}` (same as VM taps)
- Container-side veths (linked): `cva{link_index}-{lab_id}` / `cvb{link_index}-{lab_id}`
- Host-side veths (disabled): `cd{node_index}i{interface_index}-{lab_id}`
- Container-side veths (disabled): `ce{node_index}i{interface_index}-{lab_id}`

Note: Disabled veth names include the node index to avoid collisions when multiple containers share the same disabled interface index. The compact format stays within Linux's 15-character interface name limit.

## VM-to-Container

```
VM_A <-- tap_a --[ingress: BPF redirect -> veth_out]--> veth_out --[ingress: BPF redirect -> tap_a]--> | eth1 (netns) --> Container_B
```

### Setup

This is a hybrid of the VM-VM and Container-Container flows:

1. The VM side uses a tap device created by libvirt (`<interface type='ethernet'>`).
2. The container side uses a veth pair — host side stays in host namespace, container side moves into the container's netns.
3. The eBPF redirect program wires the VM's tap to the container's host-side veth and vice versa.

The VM's tap and container's host-side veth are both regular kernel network interfaces, so the eBPF redirect works identically regardless of whether the peer is a tap or veth.

### Teardown

Same as above — deleting the interfaces removes the BPF programs.

## Disabled VM Interfaces

VM disabled interfaces use libvirt's isolated network (`<interface type='network'>` with `<link state='down'/>`). After VM creation, the isolated network bridge is set DOWN to remove carrier from all taps connected to it. This ensures disabled VM interfaces show as "not connected" to the VM NOS.

## Packet Capture

Since each endpoint has a standard kernel network interface on the host:

- `tcpdump -i tap_a` captures all traffic to/from VM_A
- `tcpdump -i veth_out` captures all traffic to/from a container endpoint
- All L2 protocols are visible — no bridge filtering

## Crash Resilience

If one side of a P2p link crashes (VM or container destroyed):

- The surviving side's eBPF program continues redirecting packets to the now-stale ifindex
- The kernel silently drops these packets (TC_ACT_SHOT) — no errors or panics
- When the crashed node's interface is deleted, the BPF program on the surviving side becomes a no-op
- Full cleanup happens during lab destroy
