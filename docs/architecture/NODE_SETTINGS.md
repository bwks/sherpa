# Node Settings

A node's settings come from two places: its image definition (`shared::data::NodeConfig`), and
the overrides its manifest node sets (`topology::Node`). Both group the settings the same way,
and both nest by group on the wire: in the image API's JSON and the OpenAPI schema, and in the
manifest's TOML. No field repeats its group's name: `cpu.count`, not `cpu.cpu_count`.

## Model and kind

A node model takes one or more kinds, `virtual_machine`, `container` or `unikernel`
(`NodeKind`). `NodeModel::kinds()` lists them in a fixed order, the first being the model's
default: the kind a node gets when its manifest sets no `kind`. It is an exhaustive `match`, so
a new model does not compile without an entry. `arista_eos` and `juniper_evolved` take
`virtual_machine` (their default) and `container`; `generic` takes all three and has no default
(`NodeModel::default_kind()` is `None`), so a `generic` node or image request must name its kind.
Every other model takes one kind.

### Names

`NodeModel` is read only through its `FromStr`: its serde `Deserialize` (manifests, API JSON, the
database's text) and its clap parser (`NodeModelParser`, its `ValueParserFactory`) both call it.
So a name is accepted or refused the same way everywhere. The names removed in 0.7.0 are in
`shared::data::REMOVED_MODELS`, each with the model and kind that replace it; `FromStr` knows them
only to refuse them, as `Unknown node model 'arista_ceos': it was renamed in 0.7.0. Use model =
"arista_eos" with kind = "container".` The table also drives the startup move of the old image
folders (see [`SERVER.md`](SERVER.md#image-folders-of-renamed-models)) and is to be deleted in
0.8.0.

| Removed | Model | Kind |
|---|---|---|
| `arista_veos` | `arista_eos` | `virtual_machine` |
| `arista_ceos` | `arista_eos` | `container` |
| `cisco_asav` | `cisco_asa` | `virtual_machine` |
| `cisco_ftdv` | `cisco_ftd` | `virtual_machine` |
| `cisco_iosxrv9000` | `cisco_iosxr9000` | `virtual_machine` |
| `cisco_iosv` | `cisco_ios` | `virtual_machine` |
| `cisco_iosvl2` | `cisco_iosl2` | `virtual_machine` |
| `cisco_csr1000v` | `cisco_csr1000` | `virtual_machine` |
| `cisco_cat8000v` | `cisco_cat8000` | `virtual_machine` |
| `cisco_cat9000v` | `cisco_cat9000` | `virtual_machine` |
| `cisco_nexus9300v` | `cisco_nexus9300` | `virtual_machine` |
| `juniper_vrouter` | `juniper_router` | `virtual_machine` |
| `juniper_vswitch` | `juniper_switch` | `virtual_machine` |
| `juniper_vevolved` | `juniper_evolved` | `virtual_machine` |
| `juniper_cevolved` | `juniper_evolved` | `container` |
| `juniper_vsrxv3` | `juniper_srxv3` | `virtual_machine` |
| `generic_vm` | `generic` | `virtual_machine` |
| `generic_container` | `generic` | `container` |
| `generic_unikernel` | `generic` | `unikernel` |

Every model name drops the "v" or "c" its kind used to carry, whether or not the name left is a
product name: `cisco_csr1000` and `cisco_nexus9300` name the software, not that hardware. Two
names keep a qualifier so they stay distinct: `cisco_iosxr9000` (the 9000 platform's IOS XR) and
`juniper_srxv3` (vSRX 3.0). `cisco_ios` and `cisco_iosl2` sit beside `cisco_iol` and
`cisco_ioll2`; no code matches model names by prefix or substring, and image folders are
compared by whole path components.

`shared::data::ModelKind` (`crates/shared/src/data/model_kind.rs`) is a model with one of its
kinds. Its fields are private. `ModelKind::new(model, Option<NodeKind>)` refuses a kind the model
does not take, naming the kinds it takes, and gives the default kind for `None`, refusing `None`
for `generic`; `ModelKind::default_for(model)` takes the first kind, which for `generic` is
`virtual_machine`, so code reading a kind a user may have left out uses `new`. JSON goes through
`ModelKind::new` too
(`#[serde(try_from)]`). So a `ModelKind` in hand is a valid pair, and what depends on both hangs
off it:

| On `ModelKind` | What |
|---|---|
| `node_config()` | the built-in image settings an import, pull or scan starts from |
| `ztp_methods()` | the ZTP methods the pair takes (see [`ZTP.md`](ZTP.md)) |
| `ready_port()` | the pair's readiness port: SSH, or the Talos API for Talos Linux, for a virtual machine; SSH for the `sonic_linux` container, whose start script starts its SSH server last; `None` for any other container or a unikernel |
| `has_ssh()`, `default_boot_disk_size()`, `keeps_data_interfaces_up()`, `needs_host_checksums()` | per pair |
| `all()` | every model with each of its kinds, for tests and for walks such as the container scan's |
| `Display` | the model, and its kind when the model takes several (`arista_eos, container`), for messages such as the login-name and SSH-key refusals |

The interface functions in `shared::util::interface` (`interface_to_idx`, `interface_from_idx`,
`node_model_interfaces`, `container_interface_name`), `shared::validators::check_ztp`, and the
per-model helpers in `server::services::node_ops` (capabilities, QEMU arguments, cloud-init
quirks, interface types, CPU features, checksum offload, `stored_ready_check`) take a
`ModelKind`. Each matches on the model; a model with two kinds matches on the kind inside its
arm. `NodeConfig::model_kind()` gives an image's pair, failing for a stored row whose model does
not take its kind.

What stays per model: the guest account rules (`guest_account::key_rules` and the login-name
rule), which are about the OS and the same for each kind of a model. Their messages name the
pair.

The interface enums are per pair: `AristaEosVmInt` and `AristaEosContainerInt`,
`JuniperEvolvedVmInt` and `JuniperEvolvedContainerInt`. So are the built-in image settings
(`NodeConfig::arista_eos_vm()`, `arista_eos_container()`, `juniper_evolved_vm()`,
`juniper_evolved_container()`, `sonic_linux_vm()`, `sonic_linux_container()`, `generic_vm()`,
`generic_container()`, `generic_unikernel()`) and the EOS templates (`arista_eos_vm.jinja`,
`arista_eos_container.jinja`). The built-in container repos of the merged models are
`localrepo/arista_eos` and `localrepo/juniper_evolved`.

`sonic_linux` gained its container kind after the merge: the VM stays its default, and the
container (`docker-sonic-vs`, 2 CPUs, 1024 MiB, 32 data interfaces `eth1` to `eth32` with the
default naming) takes `volume` ZTP. Its interfaces need no enum of their own: inside the
container the image maps `ethN` to `Ethernet(4 × (N − 1))` through `lanemap.ini`, as the VM does.
Its start settings and ZTP are in [`ZTP.md`](ZTP.md#sonic-container).

Which pair a node is: its manifest `kind`, or its model's default (see
[Manifest node](#manifest-node)). Which pair an image request is: the request's `kind`, or the
model's default (`services::image_lifecycle::image_model_kind`). Images are keyed by model,
kind and version (`node_image`'s unique key, see [`DATABASE.md`](DATABASE.md)); `up`'s
`get_node_image`, `validate::version`, and the image locks (`model/kind/version`) all go by the
pair.

## Image definition

`NodeConfig` (`crates/shared/src/data/node.rs`) holds the settings every kind of image takes,
and exactly one kind's group: `settings` is a `KindSettings`, an enum with one variant per kind,
so an image cannot hold another kind's settings and its kind is which variant it holds
(`NodeConfig::kind()`). Every field is required, because an image says how its nodes are built.

| Field | Type | Fields |
|---|---|---|
| top level | | `id`, `model`, `version`, `default`, `memory` |
| `cpu` | `CpuSettings` | `count` |
| `interfaces` | `InterfaceSettings` | `data_count`, `management`, `reserved_count` |
| `settings` | `KindSettings` | one of the three groups below |

| Group | Type | Fields |
|---|---|---|
| `vm` | `VmSettings` | `os_variant`, `cpu` (`architecture`, `model`, `vmx`), `machine` (`kind`, the QEMU machine type, and `bios`), `boot_disk` (`bus`), `cdrom` (`image`, `bus`), `interfaces` (`kind`, the device, and `mtu`), `ztp` (`method`) |
| `container` | `ContainerSettings` | `repo`, `ztp` (`method`) |
| `unikernel` | `UnikernelSettings` | `boot_mode`, `cpu` (`architecture`, `model`), `machine` (`kind`), `interfaces` (`kind`, `mtu`) |

Each setting is in the group of the kinds whose code reads it, checked against its readers when
#613 split them (#614):

| Setting | VM | Container | Unikernel | Read by |
|---|---|---|---|---|
| `memory`, `cpu.count`, `interfaces.data_count`, `interfaces.management`, `interfaces.reserved_count` | yes | yes | yes | every kind |
| `os_variant` | yes | | | the cloud-init account (`node_ops`), the SSH provisioner check |
| `cpu.architecture`, `cpu.model`, `machine.kind` | yes | | yes | the libvirt domain templates |
| `cpu.vmx`, `machine.bios`, `boot_disk.bus`, `cdrom.*` | yes | | | the VM domain template, `vm_disks` |
| `interfaces.kind`, `interfaces.mtu` | yes | | yes | the domain templates; `up::ztp::domain_interfaces` and `redeploy::vm` for the MTU |
| `ztp.method` | yes | yes | | `effective_ztp`; a unikernel has none, and `NodeConfig::ztp()` gives it `disabled` |
| `repo` | | yes | | `NodeConfig::container_repo` |
| `boot_mode` | | | yes | `node_ops::generate_unikernel_setup`, `image_filename` |

No container code reads a CPU, machine, disk, CD-ROM or interface device setting: Docker makes a
container's interfaces (`up::networks`, the `container` crate), and the image's `interfaces.kind`
was never passed to it. `interfaces.prefix` and `interfaces.first_index`, which nothing read for
any kind, are gone; interface names come from the interface enums in `shared::util::interface`.

`NodeConfig::vm()`, `container()` and `unikernel()` give the image's group when it is that kind's,
and `interface_device()` the `interfaces` of a VM or unikernel. Code that builds a VM or a
unikernel takes its group with an error naming the image when it is another kind's
(`node_ops::vm_settings`, `node_ops::unikernel_settings`).

On the wire the kind is the `kind` field, and the group sits beside it, named as the manifest
names the kind groups: `vm`, `container` or `unikernel`. JSON goes through `NodeConfigFields`
(`#[serde(try_from, into)]`), which holds one optional group per kind, so the OpenAPI schema
lists all three. Reading it refuses a group that is not its kind's (`Node image 'ubuntu_linux'
is a virtual_machine image, which does not take the `container` group`) with the per-kind model
in `shared::data::kind_settings` (see [Which kinds take what](#which-kinds-take-what)), and an
image without its kind's group.

`NodeConfig`, the wire struct and every group carry `#[serde(deny_unknown_fields)]`, so JSON with
the flat names of releases before the grouping (`cpu_count`, `hdd_bus`, `ztp_username`), or with
a setting in the shared groups that 0.7.0 moved into a kind's group (`cpu.model`,
`interfaces.mtu`, a top-level `os_variant`, `repo` or `machine`) or removed
(`interfaces.prefix`), is refused, not read as defaults. serde's error names the field. No API
takes a `NodeConfig` from a client: the web image editor posts an `ImageForm` and `image.import`
an `ImportRequest`, and both deny unknown fields too, so a form or an import that sends a removed
field is refused naming it, not saved without it. The groups' doc comments are the schema's
descriptions.

### Base shapes

The 63 constructors (`NodeConfig::ubuntu_linux()` and the rest) each start from a base in
`crates/shared/src/data/node_base.rs` and state only what differs, with struct update syntax: a
whole group, or the fields of a group that differ, keeping the rest of it from the base
(`machine: MachineSettings { kind: MachineType::Q35, ..vm.machine }`).

| Base | Models | Shape |
|---|---|---|
| `image(model, memory, interfaces, settings)` | every model, through the others or directly | one vCPU, the kind's group as given |
| `cloud_init_vm(model)` and its `vm` group `cloud_init_vm_settings()` | Linux, BSD and Windows VMs, app VMs, the `generic` VM | cloud-init, q35/SeaBIOS, virtio disk, 1 jumbo-MTU virtio data interface |
| `network_os_vm_settings(os_variant, interfaces)` | network OS VMs, passed to `image` | config CD-ROM ZTP, pc/SeaBIOS, virtio disk |
| `app_container(model, repo)` | application containers | no ZTP, 1 data interface |
| `network_os_container(model, memory, repo, interfaces)` | network OS containers | config volume ZTP |
| `unikernel(model, boot_mode)` | unikernels | pc machine, virtio interfaces, no data interfaces |

The bases are `pub(super)`, used only by the constructors. A VM's base comes in two parts, the
image and its `vm` group, because Rust cannot update one variant of an enum in place: a
constructor that changes the group builds it from the group base and passes it to `image`. No
model is built from another model, so changing a base changes only the models built from it.

### The image views and the editor

The client's `image show` table (`table::image::render_image_detail_table`) shows the shared rows
and then the image's own group's, each named by its JSON key (`vm.machine.bios`,
`container.repo`). The web UI's image page, node page and image editor show and take only the
image's own groups: the templates ask `config.vm()`, `container()`, `unikernel()` and
`interface_device()`. The editor's form (`images_web::ImageForm`) has the shared fields and every
kind's fields as options; `updated_image_config` refuses a field the image's kind does not take,
with the same per-kind model, and one its kind needs that is missing. It keeps the image's model,
kind, version, management interface and a unikernel's boot mode.

The `node_image` table keeps one column per field, the kind-only ones nullable and each tied to
its kinds by a `CHECK`; `db::persistence` maps the groups to the columns by hand (see
[`DATABASE.md`](DATABASE.md)).

`shared::util::image_filename` takes the image's `KindSettings`: a `disk_boot` unikernel boots
`disk.qcow2`, any other unikernel `kernel.elf`, a VM `virtioa.qcow2`, and a container has none.

## Manifest node

`topology::Node` (`crates/topology/src/node.rs`) is one struct for every kind of node. Every
setting is optional: one left out keeps the image's value, or leaves the domain untuned.

`kind` sits beside `model`: an optional `NodeKind`. `topology::validate::check_node_model_kind`
refuses a kind the model does not take, naming the kinds it takes; it runs in `sherpa validate`
and in the server's `up` before anything else reads the node's kind. The kind is never inferred
from a settings group or an `image`.

| Group | Type | Fields |
|---|---|---|
| top level | | `name`, `model`, `image`, `version`, `memory`, `environment_variables`, `text_files`, `binary_files`, `systemd_units`, `startup_scripts`, `user_scripts`, `provisioners` |
| `cpu` | `shared::data::NodeCpu` | `count`, `mode`, `topology`, `pinning`, `emulator_pinning` |
| `numa` | `shared::data::NodeNuma` | `node`, `memory_mode` |
| `boot_disk` | `shared::data::NodeBootDisk` | `size`, `clone_method`, `cache`, `io`, `io_thread_cpus` |
| `interfaces` | `shared::data::NodeInterfaces` | `data_count`, `queues` |
| `ztp` | `NodeZtp` | `method`, `config` |
| `ready_check` | `NodeReadyCheck` | `port`, `skip` |
| `vm` | `Option<NodeVm>` | `watchdog` |
| `container` | `Option<NodeContainer>` | `user`, `privileged`, `shm_size`, `commands`, `volumes`, `platform` |
| `unikernel` | `Option<NodeUnikernel>` | `cmdline` |

The four hardware groups live in `shared::data::performance` because the `template` crate,
which depends only on `shared`, renders them into the libvirt domain
(`template::DomainTuning::from_node_settings`). The others live in `topology`, beside the types
they hold (`VolumeMount`). The manifest groups are their own structs, not
the image's: every field is an `Option`.

The shared groups are plain fields with `#[serde(default)]`, so code reads `node.cpu.count`
directly. A group left as its default is not serialized (`skip_serializing_if = "is_default"`),
so `sherpa new` writes no empty tables. The three kind groups are `Option`s, so an empty
`[nodes.container]` on a VM is still seen and refused.

`Node` and every group carry `#[serde(deny_unknown_fields)]`. A flat key of an earlier manifest
(`cpu_count = 2`, `skip_ready_check = true`, `[nodes.performance_tuning]`) fails to parse with
serde's `unknown field` error, which names the key and lists the keys the table takes. There is
no mapping of old keys to new ones.

The per-node `ssh` group (`authorized_keys`, `authorized_key_files`) was removed in favour of
the keys on the owner's account (#754). `Node` has no `ssh` field, so a node that still sets it,
as a table or inline, is refused with the same `unknown field` error; there is no dedicated
message.

The `*_data` fields (`text_files_data`, `systemd_units_data` and the rest) are the
contents the client read, sent to the server; they stay flat.

`NodeExpanded` carries the same groups, and `model_kind`, the node's resolved `ModelKind`, in
place of the model: `NodeExpanded::new` resolves it once, so nothing downstream resolves it
again. The server's `process_manifest_nodes` fills in
`boot_disk.size` from `ModelKind::default_boot_disk_size()` (Talos) when the manifest sets
none.

## Which kinds take what

`shared::data::kind_settings` is the one per-kind model: each setting with the kinds that take
it (`kind_settings::VM`, `CONTAINER`, `VM_AND_UNIKERNEL` and the rest), the kind groups' names
(`vm`, `container`, `unikernel`), and `first_not_taken`, the first set setting a kind does not
take. The manifest's node check, the image JSON's group check and the image editor's form check
all use it.

`topology::validate::check_node_kind_settings` refuses a key the node's kind (its
`model_kind.kind()`) does not take,
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
  placeholders (a `generic` container runs any image). The quota is not pinning: it holds no
  particular host CPUs, and must be no more than the server's CPUs
  (`validate::check_container_cpus`).
- A `juniper_evolved` container sizes its EVO VM from `cpu.count` and `memory`, else the image's
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
- A container has no model port, except the `sonic_linux` container's SSH. Without a port its
  readiness is that it keeps running for `CONTAINER_SETTLE` (5 seconds); with one, it must also
  answer on that port on its management IPv4, checked as a VM's port is
  (`node_ops::check_vm_ready`; an SSH handshake on port 22, a TCP connection on any other).
  `node_ops::container_ready_port` gives the port, the manifest's `ready_check.port` else the
  pair's, and none when `ready_check.skip` is set: a skipped check wins over a port.
- `ready_check` is stored on the node's record when `up` creates it and when `redeploy`
  rebuilds it (`node.ready_port`, `node.skip_ready_check` and `node.ready_sshd_marker`, from
  `node_ops::stored_ready_check`): the port, else the pair's model port, the skip, and whether a
  Windows node's SSH server must show Sherpa's ready marker. `resume`, `node.host_keys`,
  `redeploy`'s wait, `inspect` and the scanner read that record, not the lab's definition, so a
  container whose port does not answer stays `Starting` scan after scan, and a VM with a port
  that is not SSH is probed on it. See
  [`STATE_MACHINES.md`](STATE_MACHINES.md#5-node-readiness-and-host-keys).
- `container.platform` is for a `juniper_evolved` container only (`check_ptx_platform`).

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
