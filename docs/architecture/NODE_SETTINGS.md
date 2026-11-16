# Node Settings

A node's settings come from two places: its image definition (`shared::data::NodeConfig`), and
the overrides its manifest node sets (`topology::Node`). Both group the settings the same way,
and both nest by group on the wire: in the image API's JSON and the OpenAPI schema, and in the
manifest's TOML. No field repeats its group's name: `cpu.count`, not `cpu.cpu_count`.

## Image definition

`NodeConfig` (`crates/shared/src/data/node.rs`) holds the image fields and one field per group.
Every field is required, because an image says how its nodes are built.

| Field | Type | Fields |
|---|---|---|
| top level | | `id`, `model`, `version`, `repo`, `os_variant`, `kind`, `default`, `memory` |
| `cpu` | `CpuSettings` | `count`, `architecture`, `model`, `vmx` |
| `machine` | `MachineSettings` | `kind` (the QEMU machine type), `bios` |
| `boot_disk` | `BootDiskSettings` | `bus` |
| `cdrom` | `CdromSettings` | `image`, `bus` |
| `interfaces` | `InterfaceSettings` | `data_count`, `kind`, `mtu`, `prefix`, `first_index`, `management`, `reserved_count` |
| `ztp` | `ZtpSettings` | `method` (the `Ztp` enum; `disabled` is off), `username`, `password`, `password_auth` |
| `unikernel` | `Option<UnikernelSettings>` | `boot_mode`; `None` for every other kind |

`NodeConfig` and every group carry `#[serde(deny_unknown_fields)]`, so JSON with the flat names
of releases before the grouping (`cpu_count`, `hdd_bus`, `ztp_username`) is refused, not read
as defaults. There is no `#[serde(flatten)]` and no rename. The groups' doc comments are the
schema's descriptions.

### Base shapes

The 63 constructors (`NodeConfig::ubuntu_linux()` and the rest) each start from one of five
bases in `crates/shared/src/data/node_base.rs` and state only what differs, with struct update
syntax: a whole group (`boot_disk: BootDiskSettings { .. }`), or the fields of a group that
differ, keeping the rest of it from the base (`cpu: CpuSettings { count: 2, ..base.cpu }`).

| Base | Models | Shape |
|---|---|---|
| `cloud_init_vm(model)` | Linux, BSD and Windows VMs, app VMs, `generic_vm` | cloud-init, q35/SeaBIOS, virtio disk, 1 jumbo-MTU virtio data interface |
| `network_os_vm(model, memory, os_variant, interfaces)` | network OS VMs | config CD-ROM ZTP, pc/SeaBIOS, virtio disk |
| `app_container(model)` | application containers | no ZTP, 1 macvlan data interface |
| `network_os_container(model, memory, os_variant, interfaces)` | network OS containers | config volume ZTP |
| `unikernel(model)` | unikernels | direct-kernel boot, no data interfaces |

The bases are `pub(super)`, used only by the constructors. They are one level deep: no base is
built from another, and no model is built from another model, so changing a base changes only
the models built from it. The network OS bases take memory, OS variant and interfaces as
arguments because every model on them sets its own, so none can inherit a placeholder.

### Settings a kind does not use

Every image carries every group, so the image API and the `node_image` table have one shape for
all kinds. Not every kind reads them all:

| Setting | VM | Container | Unikernel |
|---|---|---|---|
| `machine.kind` | yes | | yes |
| `machine.bios`, `boot_disk.bus`, `cdrom.image`, `cdrom.bus` | yes | | |

`NodeKind::uses_machine_type` and `NodeKind::uses_firmware_and_disks` say which. An image of a
kind that does not use a setting carries its default (`pc`, `sea_bios`, `virtio`, no CD-ROM
image, `virtio`): the container and unikernel bases set the defaults, a unit test holds every
model to them, and migration 18 resets images stored with other values (see
[`DATABASE.md`](DATABASE.md#versions)). The image edit form still writes what it is given, and
nothing reads it.

The read-only views leave out the settings the image's kind does not use: the client's
`image show` table (`table::image::render_image_detail_table`), and the web UI's image page and
node page, whose templates ask the image's `NodeKind`. The image API's JSON keeps every group.

Containers do not read `cpu.architecture`, `cpu.model`, `cpu.vmx`, `interfaces.kind` or
`interfaces.mtu` either, and `interfaces.kind` differs between the two container bases; see
#614, and #613 for images that carry only their kind's groups.

The `node_image` table keeps one flat column per field; `db::persistence` maps the groups to
the columns by hand (see [`DATABASE.md`](DATABASE.md)).

`shared::util::image_filename` takes the image's `unikernel` group: a `disk_boot` unikernel
boots `disk.qcow2`, any other unikernel `kernel.elf`, and every other kind `virtioa.qcow2`.

## Manifest node

`topology::Node` (`crates/topology/src/node.rs`) is one struct for every kind of node. Every
setting is optional: one left out keeps the image's value, or leaves the domain untuned.

| Group | Type | Fields |
|---|---|---|
| top level | | `name`, `model`, `image`, `version`, `memory`, `environment_variables`, `text_files`, `binary_files`, `systemd_units`, `startup_scripts`, `user_scripts`, `provisioners` |
| `cpu` | `shared::data::NodeCpu` | `count`, `mode`, `topology`, `pinning`, `emulator_pinning` |
| `numa` | `shared::data::NodeNuma` | `node`, `memory_mode` |
| `boot_disk` | `shared::data::NodeBootDisk` | `size`, `clone_method`, `cache`, `io`, `io_thread_cpus` |
| `interfaces` | `shared::data::NodeInterfaces` | `data_count`, `queues` |
| `ztp` | `NodeZtp` | `method`, `config` |
| `ssh` | `NodeSsh` | `authorized_keys`, `authorized_key_files` |
| `ready_check` | `NodeReadyCheck` | `port`, `skip` |
| `vm` | `Option<NodeVm>` | `watchdog` |
| `container` | `Option<NodeContainer>` | `user`, `privileged`, `shm_size`, `commands`, `volumes`, `platform` |
| `unikernel` | `Option<NodeUnikernel>` | `cmdline` |

The four hardware groups live in `shared::data::performance` because the `template` crate,
which depends only on `shared`, renders them into the libvirt domain
(`template::DomainTuning::from_node_settings`). The others live in `topology`, beside the types
they hold (`VolumeMount`, `AuthorizedKeyFile`). The manifest groups are their own structs, not
the image's: every field is an `Option`.

The shared groups are plain fields with `#[serde(default)]`, so code reads `node.cpu.count`
directly. A group left as its default is not serialized (`skip_serializing_if = "is_default"`),
so `sherpa new` writes no empty tables. The three kind groups are `Option`s, so an empty
`[nodes.container]` on a VM is still seen and refused.

`Node` and every group carry `#[serde(deny_unknown_fields)]`. A flat key of an earlier manifest
(`cpu_count = 2`, `skip_ready_check = true`, `[nodes.performance_tuning]`) fails to parse with
serde's `unknown field` error, which names the key and lists the keys the table takes. There is
no mapping of old keys to new ones.

The `*_data` fields (`text_files_data`, `ssh_authorized_key_files_data` and the rest) are the
contents the client read, sent to the server; they stay flat.

`NodeExpanded` carries the same groups. Its `ssh` group is `ExpandedSsh`, whose
`authorized_key_files` holds the keys the client read rather than the file names. The server's
`process_manifest_nodes` fills in `boot_disk.size` from `NodeModel::default_boot_disk_size()`
(Talos) when the manifest sets none.

## Which kinds take what

`topology::validate::check_node_kind_settings` refuses a key the node's kind does not take,
with a message naming the node, the key and the kind, for example:
`Manifest - node: 'web01' sets `container`, which does not apply to virtual_machine nodes; 'ubuntu_linux' is a virtual_machine`.
It runs in `sherpa validate`, in the server's `up` before images are leased, and in `redeploy`.

| Key | VM | Container | Unikernel |
|---|---|---|---|
| `memory`, `cpu.count` | yes | yes | yes |
| `cpu.mode`, `cpu.topology`, `cpu.pinning`, `cpu.emulator_pinning` | yes | | |
| `numa.*` | yes | | |
| `boot_disk.size` | yes | | yes |
| `boot_disk.clone_method`, `.cache`, `.io`, `.io_thread_cpus` | yes | | |
| `interfaces.data_count` | yes | yes | yes |
| `interfaces.queues` | yes | | |
| `ztp.*`, `ssh.*` | yes | yes | |
| `ready_check.port` | yes | yes | yes |
| `ready_check.skip` | yes | yes | yes |
| `environment_variables`, `text_files`, `binary_files`, `systemd_units`, `startup_scripts`, `user_scripts` | yes | yes | |
| `vm` | yes | | |
| `container` | | yes | |
| `unikernel` | | | yes |

- A `disk_boot` unikernel's disk is cloned at `boot_disk.size` (`node_ops::generate_unikernel_setup`).
- A container's `cpu.count` and `memory` are Docker limits (`node_resources::planned_resources`,
  stored on the node record, then `ContainerSpec` in `container::run_container`): `memory` is a
  hard limit, `HostConfig.memory` in bytes, with `memory_swap` equal to it so the container
  cannot swap, and `cpu.count` a CPU quota, `HostConfig.nano_cpus` (count × 10⁹). A container
  that sets neither is not limited: the image's values are not limits, as several are
  placeholders (`generic_container` runs any image). The quota is not pinning: it holds no
  particular host CPUs, and must be no more than the server's CPUs
  (`validate::check_container_cpus`).
- `juniper_cevolved` sizes its EVO VM from `cpu.count` and `memory`, else the image's
  (`cevolved_env_vars`, checked against the image's minimums by `check_ptx_platform`). Its
  container gets a memory limit of that memory plus
  `konst::JUNIPER_CEVOLVED_CONTAINER_OVERHEAD_MIB` (2048 MiB), always, and no CPU quota, which
  would throttle the EVO VM beside the processes the container also runs. `check_ptx_platform`
  keeps the memory low enough for the overhead to fit the node's memory field (a `u16`).
- `cisco_iol` and `cisco_ioll2` give IOL the node's `memory`, else the image's (768 MiB), as
  its `-m`, the size of the whole IOL process. The container's memory limit is that plus
  `node_resources::CISCO_IOL_CONTAINER_OVERHEAD_MIB` (128 MiB) for the launcher and the bridge, always,
  so IOL fits inside the limit. `cpu.count` is a CPU quota as for any container; IOL is
  single-threaded, and the image's count is 1.
- A container has no model port. Without `ready_check.port` its readiness is that it keeps
  running for `CONTAINER_SETTLE` (5 seconds); with one, it must also answer on that port on
  its management IPv4, checked as a VM's port is (`node_ops::check_vm_ready`; an SSH handshake
  on port 22, a TCP connection on any other). `node_ops::container_ready_port` gives the port,
  and none when `ready_check.skip` is set: a skipped check wins over a port.
- `ready_check` is stored on the node's record when `up` creates it and when `redeploy`
  rebuilds it (`node.ready_port` and `node.skip_ready_check`, from
  `node_ops::stored_ready_check`): the port, else a VM's model port, and the skip. `resume`,
  `redeploy`'s wait, `inspect` and the scanner read that record, not the lab's definition, so a
  container whose port does not answer stays `Starting` scan after scan, and a VM with a port
  that is not SSH is probed on it. See
  [`STATE_MACHINES.md`](STATE_MACHINES.md#5-node-readiness-and-host-keys).
- `container.platform` is for `juniper_cevolved` only (`check_ptx_platform`).

The other checks on these settings:

- `topology::validate::check_vm_tuning` checks a VM's CPU, NUMA, disk and interface tuning
  against its own `cpu.count` and its image: `cpu.topology` and `cpu.pinning` match the vCPU
  count, pinned host CPUs are not repeated or shared with its emulator or I/O threads,
  `numa.memory_mode` needs `numa.node`, `boot_disk.io = "native"` needs
  `boot_disk.cache = "none"`, I/O threads need virtio disks, and `interfaces.queues` needs virtio
  interfaces and at most `cpu.count` queues.
- `topology::validate::check_cpu_pin_overlap` checks pins across nodes;
  `server::validate::check_host_resources` checks they exist on the server.
- `NodeCpuPins::of(name, &node.cpu, &node.boot_disk)` gives a node's pins: its vCPU pins, and
  its QEMU thread CPUs (`cpu.emulator_pinning` and `boot_disk.io_thread_cpus`).

## Stored definitions

The database records each lab's manifest as its definition (see
[`DATABASE.md`](DATABASE.md#lab-definitions)): `up` writes it, and `redeploy` the node it
rebuilds. The server saves no copy of the manifest beside the lab. Labs from before definitions
got theirs at startup in 0.6.4 and 0.6.5, from the manifest an older release saved; 0.7.0 no
longer imports. A lab created by a release before the grouping had a saved manifest in the old
shape, which did not parse, so it was not imported and the lab has no stored definition: `resume`
falls back to the default readiness timeout and the image's interface counts, and the web node
page shows the image's ZTP setting.
