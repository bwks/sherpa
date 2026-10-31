# P2p links: can eBPF redirect be the only method?

> Status: decided for #391. eBPF meets the requirements on every pair, so every link is now a
> P2p link: the `p2p` setting is rejected, bridge links are no longer created or handled (a lab
> that still has them must be destroyed and created again), and the unused UDP and veth leftovers
> are gone. Gaps 4 and 5 below are fixed. Gaps 1–3 are tracked in #394:
> changes made outside Sherpa are not handled yet. `tests/labs/p2p-links/` covers the rest.

Issue #391 sets a decision rule: if eBPF redirect meets every requirement on VM ⟷ VM,
container ⟷ container and VM ⟷ container links, it becomes the only P2p link method and the
per-link bridge goes. This records what a probe lab showed for each requirement.

## Lab used

Server v0.4.12 plus the #388 fix, on the bare-metal dev host (kernel 6.8). Every link is
`p2p = true`.

| Link | Pair | Ends |
|---|---|---|
| 0 | VM ⟷ VM | u1 `enp4s0` ⟷ u2 `enp4s0` (`ubuntu_linux` 24.04) |
| 1 | VM ⟷ container | u1 `enp5s0` ⟷ c1 `eth1` (`generic_container` alpine 3.22) |
| 2 | container ⟷ container | c1 `eth2` ⟷ c2 `eth1` |
| 3, 4 | VM ⟷ VM | veos1 Et1–2 ⟷ veos2 Et1–2 (`arista_veos` 4.35.3F) |
| 5, 6 | VM ⟷ container | veos1 Et3–4 ⟷ ceos1 Et1–2 (`arista_ceos` 4.35.1F) |
| 7, 8 | container ⟷ container | ceos1 Et3–4 ⟷ ceos2 Et1–2 |
| 9 | container ⟷ container | ceos2 Et3 ⟷ srl1 ethernet-1/1 (`nokia_srlinux` 25.10.1) |
| 10 | VM ⟷ container | veos2 Et3 ⟷ srl1 ethernet-1/2 |

Container ends were driven from a `nicolaka/netshoot` sidecar sharing the node's network
namespace; host ends were captured from a netshoot container on the host network. Neither
needs more than Docker's default capabilities.

## Results

| Requirement | VM ⟷ VM | VM ⟷ container | container ⟷ container |
|---|---|---|---|
| L2 transparency | pass | pass | pass |
| Link state | **fail** | **fail** | **fail** |
| Lifecycle: up, redeploy, down/resume, guest reboot | pass | pass | pass |
| Lifecycle: VM cold boot, container restart outside Sherpa | **fail** | **fail** | **fail** |
| Lifecycle: `sherpad` restart, destroy | pass | pass | pass |
| Packet capture | pass | pass | pass |
| Impairment, per direction | **fail** (symmetric only) | **fail** | **fail** |
| Jumbo frames | pass | pass | pass |
| Privileges | pass | pass | pass |

None of the failures is a limit of eBPF redirect. Each is missing handling in Sherpa, and a
bridge link has the same gaps or worse (see [Bridge links compared](#bridge-links-compared)).

### L2 transparency: pass

A stdlib-only raw-socket script sent one marked frame of each kind, each way, on links 0–2:
LLDP (`01:80:c2:00:00:0e`), STP (`…:00`), pause (`…:01`, `0x8808`), LACP (`…:02`, `0x8809`),
802.1X (`…:03`), GARP range (`…:20`), CDP (`01:00:0c:cc:cc:cc`, SNAP), 802.1Q, QinQ
(`0x88a8` outer, `0x8100` inner), and 1600- and 9000-byte frames. Every frame arrived
unchanged on all three pairs. Tags arrive offloaded to packet metadata and are put back on
the wire, so the receiver sees the same frame. Pause frames are the case a Linux bridge can
never forward, whatever its `group_fwd_mask`.

Real NOS neighbours:

- LLDP neighbours formed on every link: vEOS ⟷ vEOS, vEOS ⟷ cEOS, cEOS ⟷ cEOS, cEOS ⟷ SR Linux
  and vEOS ⟷ SR Linux. SR Linux needs its ports enabled first; Sherpa's startup config leaves
  them out.
- LACP port-channels came up with both members bundled (`PG+`) on vEOS ⟷ vEOS (links 3, 4),
  vEOS ⟷ cEOS (5, 6) and cEOS ⟷ cEOS (7, 8).

### Link state: fail

A link comes up with carrier at both ends. Nothing tells one end when the other goes:

- Stopping c2 deleted its host end; c1 `eth2` kept carrier, operstate up.
- Shutting down u2 deleted its tap; u1 `enp4s0` kept carrier.
- cEOS ⟷ cEOS is the one exception: `shutdown` on ceos2 Et1 made ceos1 Et3 `notconnect` within
  seconds. cEOS does that itself (both kernel interfaces went `DORMANT`); the host ends never
  lost carrier, so it says nothing about other NOSes.

A bridge link has the same gap: the bridge keeps carrier for every member.

### Lifecycle

Passes:

- `up`, then traffic on every link.
- `redeploy` of c2 (container end of a container ⟷ container link) and of u2 (VM ⟷ VM).
- `down` then `resume` of the whole lab. `down` pauses containers, so their interfaces survive;
  resume re-attaches every VM tap.
- A guest reboot (`systemctl reboot` in u2): QEMU keeps the process and the tap (same ifindex).
- A `sherpad` restart: traffic, LACP and LLDP carried on unchanged on every link.
- `redeploy` of c1 after the restart (container end of a VM ⟷ container and a container ⟷
  container link). The peers' pings failed until c1 sent first: its interfaces have new random
  MAC addresses and the peers' ARP entries were stale.
- `destroy` left no interface, container, Docker network, domain, libvirt network or volume.

Failures, both from changes Sherpa does not see:

- **VM cold boot.** `virsh shutdown` then `virsh start` u2: libvirt deleted the tap and
  created a new one with the same name, with no redirect program on it. u1's program still
  points at the old ifindex. All traffic was lost until `resume` re-attached both ends.
- **Container restart.** `docker stop` then `docker start` c2: the namespace went with the
  container, and with it both veth ends. The restarted container had no `eth1`, and `resume`
  did not recreate it; only a redeploy did. Containers have no restart policy, so this needs
  someone to restart the container, or a host reboot.

### Packet capture: pass

`tcpdump -i <host end>` on tpa0 (VM tap), tpb1 (container veth facing a VM) and tpa2
(container veth facing a container) saw every marked frame from both directions, with
ethertypes and tags intact.

### Impairment: fail (symmetric only)

`POST /api/v1/labs/{id}/links/{n}/impairment` with 20 ms delay raised ping RTT from about
0.3 ms to 40 ms on each pair (20 ms each way). Changing a running link to 50 ms and 10% loss
took effect at once (100 ms RTT, 15% of 40 pings lost); clearing it restored 0.3 ms.

netem sits on each host end's egress, so the kernel side is per direction already. The API
applies the same values to both ends, so there's no way to impair one direction.

### Jumbo frames: pass, by luck on VM ends

9000-byte frames and `ping -M do -s 8972` passed on every pair. VM taps are MTU 1500, though:
the P2p `<interface type='ethernet'>` in `libvirt_domain.jinja` has no `<mtu>`, unlike every
other interface type. Jumbo frames pass only because a redirect to egress does not check the
target's MTU. Setting `<mtu size='9600'/>` makes this deliberate and gives virtio guests the
host MTU.

### Privileges: pass

Everything above ran under `sherpad`'s current capabilities.

## Bridge links compared

Not probed here; this follows from the kernel's bridge rules and how libvirt and Docker attach
interfaces. A bridge link fails link state the same way, never forwards pause frames, needs
`group_fwd_mask` for the rest of `01:80:c2:00:00:0x`, and does not carry LACP (`…:02`) at all
(the kernel refuses mask bits 0–2).
It does survive a VM cold boot (libvirt re-adds the tap to its network's bridge) and a
container restart (Docker reattaches its networks), which eBPF does not yet. Both are fixable
within eBPF, so the gaps do not justify keeping a second method.

## Gaps to close

1. **Link state.** When one end's node stops or its host end goes, take the other end down:
   for a container, set its host end down so the container end loses carrier; for a VM, set
   the interface's link down through libvirt (`<link state='down'/>`). Bring both back when
   the link is whole again.
2. **VM cold boot.** When a tap is recreated, attach the redirect to it and point the peer
   at its new ifindex.
3. **Container restart.** When a lab container starts outside Sherpa, recreate its veths and
   attach their redirects.
4. **Per-direction impairment.** Let the API set each direction separately.
5. **VM tap MTU.** Add `<mtu>` to the P2p interface in `libvirt_domain.jinja` and
   `unikernel_domain.jinja`.

Gaps 1–3 need `sherpad` to notice changes it did not make: netlink link events for host ends,
libvirt lifecycle events and Docker container events. They are #394, likely with a per-link
state machine.

Fixing gap 4 turned up two more impairment bugs, fixed with it:

- `apply_netem` never sent reordering or corruption to the kernel, so neither had any effect.
- `up` applied the manifest's impairment but never stored it, and resume and redeploy only
  re-applied impairment with a delay or loss. A manifest impairment was lost on resume.

A redeployed node's peers can hold stale ARP entries (seen on c1 above). That isn't a gap:
VM and container interfaces both get a new random MAC on every build, as a replaced device
would.

## Fixes verified

On a lab with a VM ⟷ VM link impaired from the manifest (10 ms) and a container ⟷ container
link impaired through the API:

- Every host end, taps included, was MTU 9600, and the Ubuntu guest's interface came up at
  9600 on its own. `ping -M do -s 8972` passed.
- The manifest's 10 ms was on both ends after `up`, and again after `down`/`resume`.
- `src_to_dst` 30 ms put netem on the dst end only; a later `dst_to_src` 5 ms left it there
  (35 ms RTT). Clearing `dst_to_src` removed only its netem.
- Reorder 25% and corrupt 5% showed in `tc qdisc show` (`reorder 25% corrupt 5% gap 1`). With
  both, 100 pings lost 4% to corruption, and reordered replies came back with no delay.
- Redeploying c2 re-applied only the `src_to_dst` impairment, with reorder and corruption.
- `direction = "sideways"` returned 422.
