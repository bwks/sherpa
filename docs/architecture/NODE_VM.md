# Node VMs

The container models that need root-like privileges to boot, `arista_ceos` (`arista_eos`
container), `nokia_srlinux` and the `sonic_linux` container, each run in a Flatcar Container
Linux VM of their own. Inside it, a normal Docker runs the node's image unmodified with
`--privileged` and the settings a container on the server gets. A privileged container that
escapes reaches that VM's kernel, not the server's.

`ModelKind::runs_in_node_vm` (`crates/shared/src/data/model_kind.rs`) is the one list of these
models. Every other container model runs on the server's Docker as before.

## Shape

```mermaid
flowchart LR
  subgraph host[Sherpa server]
    S[sherpad] -->|bollard over vsock, mutual TLS| V
    S -->|libvirt: domain, overlay disk, config drive| L[libvirtd]
    D[(server Docker: node image)] -->|docker save stream, once per image| ST[(flatcar-docker-ID.qcow2, the image's Docker store)]
    B[(node_vm/flatcar-VERSION.qcow2, read-only base)] --> O[(NODE-LAB-hdd.qcow2 overlay)]
    ST --> OD[(NODE-LAB-docker.qcow2 overlay)]
    K[(node_vm/flatcar-VERSION.vmlinuz)] -->|direct kernel boot| V
    T1[tap tpaN-LAB] <-->|eBPF redirect, netem| T2[tap of the peer node]
    M[brm-LAB bridge, lab management network]
    R[console relay] --- SOCK[/console socket NODE-INDEX.serial/]
  end
  subgraph V[Flatcar VM NODE-LAB]
    P[docker-vsock.socket: vsock 2376 to 127.0.0.1:2376] --> DK[dockerd, TLS, client certificate required]
    DK --> C[node container, --privileged]
    MV[sherpa-nics.service: moves data NICs into the container on each start] --> C
    CON[sherpa-console.service on ttyS0: docker exec CLI] --> C
    MAC[macvlan on sherpa0, node's management addresses] --> C
  end
  O --> V
  OD -->|/var/lib/docker| V
  ISO[(NODE-LAB.iso, label config-2, Ignition)] --> V
  V ---|virtio NIC| T1
  V ---|virtio NIC sherpa0| M
  V ---|serial| SOCK
```

## Base image

- Flatcar stable, the OpenStack image (`flatcar_production_openstack_image.img.bz2`), whose
  Ignition reads its config from a config drive. The QEMU image reads Ignition only from
  `fw_cfg`; booting with Ignition from `fw_cfg` measured the same as the config drive.
- Its kernel, `flatcar_production_image.vmlinuz` of the same release, which node VMs boot
  directly (below).
- The release is pinned in two places that must match: `FLATCAR_NODE_VM_VERSION` in
  `crates/server/src/services/node_vm/mod.rs`, and `NODE_VM_FLATCAR_VERSION` with
  `NODE_VM_FLATCAR_SHA512` and `NODE_VM_KERNEL_SHA512` in the "node VM image" section of
  `scripts/sherpa_install.sh` and `scripts/sherpa_update.sh` (the section is the same in both).
- `ensure_node_vm_image` downloads the compressed image and the kernel from
  `stable.release.flatcar-linux.net` as `sherpa`, checks each against its pinned SHA-512 (the
  values Flatcar publishes in their `.DIGESTS`), and puts them at
  `/opt/sherpa/images/node_vm/flatcar-VERSION.qcow2` and `flatcar-VERSION.vmlinuz`, mode `0644`.
  A file already there is never changed: every node VM disk is an overlay on the image, and every
  node VM boots the kernel. A new release is a new file name, so running node VMs keep theirs.
- The script also loads `vhost_vsock` and adds it to
  `/etc/modules-load.d/sherpa-node-vm.conf`; uninstall removes that file.
- `up` and `redeploy` refuse a lab with a node VM before creating anything when the base image
  or its kernel is missing (`ServiceError::NodeVmBaseImageMissing`).

## Boot

The domain boots the kernel directly (`<os><kernel>`, `<cmdline>`), not through GRUB:

- The command line is the one Flatcar's GRUB builds, with `verity.usrhash` read from the
  kernel's header (64 hex digits at offset 64, where GRUB reads it) and `/usr` on the first
  `/usr` partition, which updates never change.
- It has no `console=`: the kernel's messages never reach the serial console, which is the
  network OS's. Printing them there is slow in a VM, and slower nested.
- `flatcar.first_boot=detected` is on the command line of the first boot, so Ignition runs.
  Once the VM's Docker answers, Ignition has run, and sherpad defines the domain again without it
  (`libvirt::set_node_vm_cmdline`): later boots skip Ignition, as GRUB's would.

## Docker stores

The node's image is not copied into each node VM. Each image has a Docker store: a disk whose
Docker data has the image loaded, which every node VM of the image boots an overlay of.

- **Name.** `flatcar-docker-ID.qcow2` in the storage pool, `ID` the first 16 hex digits of the
  image's ID on the server's Docker. An image imported again under the same name has a new ID,
  so it gets a store of its own.
- **Made** at the first `up` or `redeploy` that needs it (`store::ensure_docker_store`), while
  the node VMs are created. A Flatcar VM `sherpa-flatcar-docker-ID`, with no NICs and no console,
  boots on an overlay of the base image with an empty 32 GiB qcow2 as its second disk. Its
  Ignition formats that disk ext4, labelled `sherpa-docker`, and mounts it at
  `/var/lib/docker`, with Docker on vsock as in a node VM. sherpad streams `docker save` of the
  image into its `docker load` (`container::copy_image_to_node_vm`), shuts it down over ACPI, and
  removes it. The disk is made as `flatcar-docker-ID.qcow2.part`, mode `0644`, and renamed to
  the store once the VM has shut down, so a store that exists is complete. Nodes of one image
  wait for the one VM that makes its store.
- **Used.** A node VM's second disk is `NODE-LAB-docker.qcow2`, a qcow2 overlay on the store.
  Its Ignition mounts it at `/var/lib/docker` before containerd and Docker start, with
  `noinit_itable`, so no node zeroes inode tables into its overlay. The VM's Docker has the image
  when it starts; `start_node_container` still copies it when it is missing.
- **Kept.** Stores belong to no lab: destroy never deletes one (`node_vm_037`).

The alternatives measured, and why they were not taken:

- **A read-only disk shared by every node, with Docker's data on it.** dockerd refuses a
  read-only data root (it changes its mode at start). A data root on an overlayfs of the
  read-only store and a writable layer is refused by the `overlay2` driver ("driver not
  supported"), and dockerd then picks no other driver. With the containerd image store, Docker
  keeps images in the system containerd's root, which must be writable and is under the same
  overlay limit. Docker has no additional read-only image stores; those are a
  `containers/storage` feature of Podman and CRI-O, which Flatcar does not ship. Binding the
  store's layer folders read-only into a writable data root depends on Docker's private layout;
  it did not run and was not pursued. A qcow2 backing file shares the store at the block layer
  instead: the node's Docker sees a writable disk, and the server's disk holds the store once.
- **The image baked into the base image, one disk chain.** The baked disk would have booted once,
  so it would need Flatcar's first boot state back (`flatcar-reset`) before each node's Ignition,
  and a new Flatcar release would mean baking every image again. A second disk keeps the base
  image as Flatcar ships it.
- **Starting the container from a systemd unit at boot, not over the API.** The API calls after
  Docker answers take about 2 s (create, file upload, start); the unit would save at most that,
  and would need the container's settings and files in Ignition.
- **Restoring a saved VM state.** A restore answered in under 5 s, against about 40 s for a boot,
  but the state file is the VM's memory (0.95 GB for an idle 2 GiB VM), and a restored VM keeps the
  saved one's NIC MACs, vsock ID and Docker certificates, which are per node.

## Per node

| What | Where | Removed by |
|---|---|---|
| Domain `NODE-LAB` | libvirt, persistent | destroy (lab suffix), redeploy, rollback |
| Disk, a qcow2 overlay on the base image | storage pool, `NODE-LAB-hdd.qcow2` | destroy (lab volume suffix), redeploy |
| Docker disk, a qcow2 overlay on the image's Docker store | storage pool, `NODE-LAB-docker.qcow2` | destroy (lab volume suffix), redeploy |
| Config drive, ISO 9660 labelled `config-2` holding `openstack/latest/user_data` | storage pool, `NODE-LAB.iso` | destroy, redeploy |
| sherpad's CA, client certificate and key for the VM's Docker, `0600` | `LAB_DIR/NODE/node-vm/` | destroy (lab folder), redeploy |

The two overlays hold everything the node writes, including the network OS's saved config, so a
saved config survives `down` and `resume`. Redeploy and destroy delete them.

The domain (`crates/libvirt/src/node_vm.rs`, XML written with `quick-xml`) is q35 with
`host-passthrough` CPU, no video and no balloon: the base image's kernel and its command line,
the two overlay disks on virtio, the config drive on SATA, the management NIC on the lab's
libvirt management network, one virtio NIC per data interface on a link or bridge whose tap is
named like the node's link end, a vsock device with a context ID libvirt picks, a virtio RNG,
and the serial console on the node's console socket. A balloon with free page reporting was
measured and gave nothing: without an image load, a node VM never fills its memory.

## First boot

Ignition runs once, on the first boot of a disk (see Boot). Its JSON is generated at run time from
`template::IgnitionConfig` (`crates/server/src/services/node_vm/ignition.rs`); none is checked
in. It sets:

- Updates and reboots off: `/etc/flatcar/update.conf` (`SERVER=disabled`, `REBOOT_STRATEGY=off`),
  and `update-engine` and `locksmithd` masked.
- No login: no users, `sshd` and its vsock socket masked, the serial getty masked. The OpenStack metadata units of the
  image are masked: the VM has no network of its own.
- Networking: the management NIC is renamed `sherpa0` by MAC and kept up with no address; every
  other NIC is unmanaged.
- Docker: its data on the Docker disk (see Docker stores); it listens on `127.0.0.1:2376` with `--tlsverify`, its server certificate and the VM's CA,
  besides its local socket, with no default bridge. `docker-vsock.socket` listens on vsock port
  2376 and hands each connection to `systemd-socket-proxyd`, which connects it to that listener.
- The NIC mover, the console, and for SR Linux the start step (below), as scripts in
  `/opt/sherpa` with their units.

## Docker's API: vsock with mutual TLS

sherpad reaches each VM's Docker over vsock with mutual TLS. The other candidates:

- **TLS over TCP on the management network.** The VM would need a management address of its own
  besides its container's. A lab's management `/24` gives every host offset from 21 to 254 to a
  node index, so there is no spare address without shrinking the largest lab or adding a second
  network with its own addressing.
- **SSH.** bollard's SSH transport needs the `openssh` crate, which is not in `Cargo.lock`, and it
  still needs an address to reach.
- **vsock.** It needs no address and no network: only the host reaches a guest's vsock, and
  guests cannot reach each other's. Any process on the host can connect to any guest's context
  ID, though, so vsock alone is not authenticated. Mutual TLS on top closes that.

Each VM gets a CA of its own (`NodeVmCerts::issue`). It signs the VM's server certificate, whose
name is the domain name, and sherpad's client certificate; its key is dropped once both are
issued. dockerd accepts only a client certificate of that CA, and sherpad trusts only that CA
for the VM. The certificates are valid for ten years; there is no renewal.

The client is `container::node_vm_docker` (`crates/container/src/node_vm/transport.rs`), a bollard
custom transport. Each request opens its own connection: a non-blocking `AF_VSOCK` socket from
`nix`, polled by tokio's `AsyncFd`, wrapped in `tokio-rustls` with the aws-lc-rs provider, and an
HTTP/1 connection from `hyper` kept for upgrades. The context ID is read from the running
domain's XML each time a client is made, so it can change across restarts.

## Links

Each data interface of the node on a link or bridge is a virtio NIC whose host side is a tap
named like the node's link end (the name a container's host veth has). Its other data interfaces
are made in the VM: each NIC adds a PCIe device that QEMU starts and the kernel probes, which
cost a 52-interface cEOS about 30 s of boot (nested). libvirt makes the taps when it starts the
domain, so a node VM's links are attached in phase 11b with the VMs' (`attach_vm_links`), and
re-attached after a cold boot or a redeploy as a domain's are (`RecreatedEnd::Domain`). The eBPF
redirect, netem impairment and carrier handling are those of every P2p link
([P2P-ARCHITECTURE.md](P2P-ARCHITECTURE.md)). A node on no link gets no data NICs, as a
container on the server gets no veths.

In the VM, `sherpa-nics.service` runs `move-nics.sh`: at start and on every `start` event of the
node's container, it renames each data NIC, found by its MAC in `/etc/sherpa/interfaces`, to a
temporary name, moves it into the container's network namespace and names it as the network OS
expects (`eth1`, `e1-1`, ...). When the container's namespace goes, the kernel hands its NICs back
to the VM, and the next start moves them again. For each data interface on no link, listed in
`/etc/sherpa/placeholders` with a MAC sherpad picked, it makes a veth with that MAC, moves that
end into the container under the interface's name, and leaves the other down in the VM, so the
interface is there without carrier, as a disabled interface of a container on the server is. The
MAC stays the same across starts: cEOS makes new SSH host keys when its interfaces' MACs change.
The script then runs `/opt/sherpa/on-start` when
the model has one: SR Linux's flushes Docker's copy of the management address once SR Linux has
moved management into its own namespace, which Docker sets again at every start.

## Management

The VM's Docker has a macvlan network on `sherpa0` named like the lab's management network, with
the lab's subnets. The container gets the node's IPv4 address and, when the lab has IPv6, the
IPv6 address recorded for the node: the VM's Docker knows none of the lab's other addresses, so
an address it picked could be the router's. The container's management MAC is random per
creation.

## Console

The serial console is on the node's console socket, which the console relay serves as for any
VM, so `sherpa console` works for these nodes; their VNC console is refused. `sherpa-console`
runs `console.sh` on `ttyS0`: it waits for Enter, so the CLI starts with someone at the console,
gives the line a size, and runs `docker exec -it` of the model's CLI in the container (`Cli`,
`sr_cli`, `bash` for SONiC). systemd starts it again when the CLI exits.

## Lifecycle

- **Up.** Phase 11 creates each node VM, in parallel with the other VMs
  (`create_node_vms`): the image's Docker store when there is none, certificates, config drive,
  the two overlay disks, domain. Readiness (phase 13) starts each node's container in the
  background (`start_node_container`): wait for the VM's Docker, drop `flatcar.first_boot` from
  the domain, make the management network, create the container with Docker's
  `unless-stopped` restart policy, copy the node's files from its volumes into it, and start it.
  The node is ready once SSH answers on its management address (`ModelKind::ready_port`).
- **Status, down, resume, scanner, console.** `NodeConfig::runtime_kind` is `VirtualMachine` for
  these nodes, so they follow the VM paths: `down` shuts the domain down over ACPI, which stops
  Docker and the container cleanly; `resume` starts the domain, Docker starts the container
  again, the mover moves its NICs, and the links are re-attached.
- **Redeploy.** Removes the VM, its two disks, its config drive and its certificates, and the server
  container of the same name if one is left, then creates the VM again
  (`redeploy_node_vm`).
- **Destroy and rollback.** The domain and its volumes go with the lab's other domains and
  volumes, by the lab's name suffix; the certificates go with the lab folder. A failure while
  creating a node VM removes what that node had.
- **sherpad restart.** The VMs are libvirt's; a restart of sherpad leaves them running.

## Resources

A node VM gets the manifest's or the image's `memory` with 1024 MiB on top, and the manifest's or
image's `cpu.count` (`planned_resources`); admission counts that. In the VM the container has no
limits: the VM is its limit.

Measured in a nested guest, lab up with the three models and an Alpine VM, Docker stores made:

| Node | VM memory | QEMU PSS on the host | Used in the VM | Page cache in the VM | Disks on the server | Domain start to SSH-ready | runc: container start to SSH |
|---|---|---|---|---|---|---|---|
| cEOS | 5120 MiB | 3070 MiB | 1591 MiB | 1519 MiB | 354 MiB | 133 s | 71 s |
| SR Linux | 5120 MiB | 3781 MiB | 1962 MiB | 1747 MiB | 342 MiB | 51 s | not reached |
| SONiC | 2048 MiB | 1628 MiB | 768 MiB | 791 MiB | 346 MiB | 101 s | 65 s |

The whole lab is up in about 150 s. A node VM's time to ready is about 45 s from domain start to
its Docker answering, 2 s to start its container, and then the network OS's own boot. The
first up with an image adds making its store: about 140 s with the three made at once. A store
takes 1.8 to 3.6 GB once on the server; each node's disks hold only what it writes.

## Limits

- A shared bridge (`[[bridges]]` with Docker networks) does not reach a node VM's container.
- Writes to a read-write volume stay in the container: they never reach the server's file, and a
  changed source on the server is not read again until a redeploy.
- Secret files are copied into the container, on the node's disk, not mounted from RAM.
- The config drive, with the VM's server key, stays in the storage pool until destroy or
  redeploy.
- `frr_linux` and the `juniper_evolved` container still run privileged on the server.
- A Docker store stays in the storage pool when its image is deleted or imported again; nothing
  removes stores yet.
