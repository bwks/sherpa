# Manifests

Whether a `manifest.toml` is accepted, and what it means once it is read: how the file is found and parsed, the names nodes may have, which kinds a model takes, which settings a kind and a model take, the rules a virtual machine's tuning, the links and bridges, the Talos clusters, the provisioners and the lab's own settings must meet, what `sherpa validate` checks and reports, and where the CLI and the server check differently.

What a setting does once it is accepted belongs to the area that carries it out: [lab lifecycle](lab-lifecycle.md), [nodes](nodes.md), [images](images.md), [links and bridges](links-and-bridges.md), [accounts and auth](accounts-and-auth.md), [consoles](consoles.md), [ZTP](ztp.md), [provisioning](provisioning.md), [client lab files](client-lab-files.md), [tailnet](tailnet.md) and [Talos](talos.md). Checks those specs already hold are not repeated here:

- an up of an invalid manifest leaving nothing behind: `lab_preflight_002`; a node's image and version: `image_selection` and `lab_preflight_001`; CPUs another lab holds and memory that does not fit: `lab_preflight_005` to `007`;
- `ztp.method` against the model: `ztp_setting_001`, `004`, `005`, `006` and `lab_preflight_004`; a `ztp.config` on a node that takes none: `custom_config`;
- `container.privileged` and `container.volumes` from a user who is not an admin: `access_control_014`;
- files listed without their contents, file names in a request, an entry that is not `KEY=VALUE` at the CLI: `client_content_010` to `012`; a manifest with provisioners sent to the server: `provisioner_secrecy_002`; secret file names and modes: `container_secrets_010`, `012`, `013`;
- the tailnet key and a key sent for a lab without `[tailscale]`: `tailnet_key`;
- a cluster member or a cluster's Vault with its own `ztp.config`: `machine_config_007` and `008`;
- the interface a name means inside a node: `node_interfaces_001`; the range of an impairment sent to a running lab: `link_impair_009`.

Each behaviour lists its cases. A case is one test, whose name is its ID: `<behaviour>_<NNN>_<description>`, such as `node_naming_002_a_name_with_a_path_a_quote_or_markup_is_refused`. The level is the highest that can show the case: e2e through the `sherpa` CLI, integration against a running sherpad, unit for the parsing and validation rules, which have too many branches to cover above. Most cases here are unit. The e2e cases are the ones whose subject is the command itself: its exit status and what it prints.

## Where each check runs

A manifest is checked in five places, and they do not run the same checks. `sherpa up` and `sherpa redeploy` do not run `sherpa validate` first: the CLI parses the file, runs a few checks, and the server refuses the rest.

| Check | `sherpa validate` | CLI at up | CLI at redeploy | Server at up | Server at redeploy |
|---|---|---|---|---|---|
| Parsing: unknown keys, types, required keys | yes | yes | yes | yes | yes |
| `[tailscale]` `auth_key_env` | yes | yes | | yes | |
| The lab's `name`, `ready_timeout` and node settings that cannot be 0 | yes | yes | yes | yes | yes |
| Node names, duplicate names, node count | yes | | | yes | |
| A node's `kind` against its model | yes | | | yes | yes |
| Settings against the node's kind | yes | | | yes | yes |
| `interfaces.data_count`, VM tuning, `container.platform` | yes, against the model's built-in settings | | | yes, against the node's image | yes, against the node's image |
| `ztp.method` | yes | | | yes | yes |
| Pinned CPUs shared inside the manifest | yes | | | yes | |
| Link ends, interface bounds, `p2p`, impairments | yes, against the model's built-in settings | | | yes, against the node's image | |
| An interface used twice | yes | | | yes | |
| Talos clusters | yes | | | yes | the node's cluster and role, against the lab's stored definition |
| Provisioners | yes | yes | yes | yes, on the manifest as written | yes, on the manifest as written |
| `description` | yes | yes | | yes | |
| Text file modes | yes | yes | yes | yes | yes |
| `environment_variables` keys | yes | yes | yes | yes | yes |
| Windows startup script names | | yes | yes | yes | yes |
| `text_files`, `startup_scripts`, `systemd_units` against the node's model, kind and ZTP | yes, against the model's built-in settings | yes, against the model's built-in settings | yes, against the model's built-in settings | yes, against the node's image | yes, against the node's image |
| Images and versions, a `ztp.config` its node takes (#772), host access, files the client read, memory, CPUs held | | | | yes | yes |

A redeploy reads one node of the manifest it is sent. The lab's description, links, bridges, clusters and `[tailscale]` in it are not checked and not stored; `ready_timeout` and `provision_concurrency` are stored. The node's cluster and role in it must be those of the lab's stored definition, and the node is rebuilt with the stored ones: `cluster_redeploy_009` and `010` in [talos.md](talos.md).

## `manifest_file`: a lab is read from `manifest.toml` in the current folder, strictly

**Statement:** the CLI reads a lab from the file `manifest.toml` in the current folder, as TOML with `name` and `nodes` required and every other key optional; a key it does not know, a value of the wrong type or a word a key does not take is refused, naming the file, the place and the key, and the server reads the manifest it is sent by the same rules. `sherpa new` writes an example of the file.
**Relied on by:** the user, whose typing mistake is reported and not ignored; every other behaviour here, which checks the manifest as read. **Priority:** P0.

| Case | What it shows | Level |
|---|---|---|
| 001 | `sherpa validate` in a folder with no `manifest.toml` exits 1 and says the file was not found | e2e |
| 002 | A file that is not TOML is refused, naming the file | unit |
| 003 | A manifest without `name`, or without `nodes`, is refused, naming the missing key | unit |
| 004 | An unknown key is refused in every table of a manifest, naming the key: the top level, a node, each group of a node, a link, an impairment, a bridge, a text file, a systemd unit, a volume, a Talos cluster, `[tailscale]`, a provisioner and a provisioner's file | unit |
| 005 | Every key a manifest takes is read as the setting it names: the lab's own, a virtual machine with every shared group and `vm`, a container with `container`, a unikernel with `unikernel`, text files, links and bridges | unit |
| 006 | A key left out stays unset and is not given a value: a node's `kind`, its `ztp.method` and its groups; a manifest without `[tailscale]` has no tailnet and `enabled` left out is off; a provisioner file's `mode` is 600 and a secret file's 444; a cluster's `cni` is `flannel` and it has no workers | unit |
| 007 | A manifest written out as TOML has a table only for the groups a node sets, and reads back the same | unit |
| 008 | A value of the wrong type, a number outside its key's range (a `memory` over 65535, a `cpu.count` over 255, a negative count) or a word the key does not take (`kind`, `cpu.mode`, `numa.memory_mode`, `boot_disk.clone_method`, `cache` and `io`, `ztp.method`, `vm.watchdog`, `container.platform`, `cni`, a provisioner's `type`) is refused, saying what the key takes (no test yet) | unit |
| 009 | `sherpa validate` on a manifest with an unknown key exits 1 and prints the file, the line and column, the key and the keys that table takes | e2e |
| 010 | `sherpa new` writes a `manifest.toml` of two Linux nodes and a link between them under a generated name, and `sherpa validate` passes it | e2e |
| 011 | `sherpa new` leaves an existing `manifest.toml` as it is and says so, and with `--force` writes over it | e2e |
| 012 | A manifest sent to the server with an unknown key or a value of the wrong type is refused as the file is, and nothing is created (no test yet) | integration |

A folder with no `manifest.toml` at `sherpa up`, `redeploy` and `provision`, and a manifest that does not load beside a `lab-info.toml`, are lab identity: `lab_identity_007` and `008` in [lab lifecycle](lab-lifecycle.md), and `saved_lab_files`.

## `validate_command`: `sherpa validate` checks a manifest with no server

**Statement:** `sherpa validate` checks the folder's `manifest.toml` with no login and no server, each node against its model and kind's built-in settings, prints each check as it passes, and exits 0 once all pass or 1 at the first that fails, with its message; `sherpa up` and `sherpa redeploy` run only the CLI's own checks and report the server's refusal of anything else.
**Relied on by:** the user, before an up; scripts and the lab tests, by its exit status. **Priority:** P1.

| Case | What it shows | Level |
|---|---|---|
| 001 | A valid manifest passes with every check ticked, `Manifest validation passed!` and exit status 0, on a machine with no login and no server configured | e2e |
| 002 | A manifest that fails a check exits 1 with that check's message, and no later check is printed | e2e |
| 003 | A node is checked against its model and kind's built-in settings: a link on an interface past the built-in data interface count is refused, and taken once the node sets `interfaces.data_count` | unit |
| 004 | A `ztp.config` on a node whose ZTP setting takes none passes `sherpa validate`, and up refuses it (#772) (no test yet) | unit |
| 005 | `sherpa up` of a manifest the server refuses exits 1 with the server's message, which names the check that failed and the node | e2e |

What `sherpa validate` does not check, because only the server knows it: whether a node's image and version are there, the settings of an image an admin has edited, the memory and CPUs free on the server, and whether the user may ask for host access.

## `node_naming`: a node's name is a plain name, used once

**Statement:** a node's name is 1 to 63 ASCII letters, digits, `-` or `_`, starting with a letter or digit, no two nodes of a lab share one, and a lab has at most 234 nodes.
**Relied on by:** the host names, container and domain names, file names in the lab folder, DNS names and web pages a node's name is put in; the lab's address block, which has 234 node addresses. **Priority:** P0.

| Case | What it shows | Level |
|---|---|---|
| 001 | A plain name is taken: letters of either case, digits, `-` and `_` after the first character, and 63 characters | unit |
| 002 | An empty name, one that starts with `-` or `_`, one with a path separator, a dot, a space, a quote, markup or a non-ASCII letter, and one of 64 characters, are refused, naming the name and the rule | unit |
| 003 | Two nodes with one name are refused, naming it | unit |
| 004 | A manifest with 235 nodes is refused, saying a lab takes at most 234 | unit |

The server runs these at up before anything is created (`lab_preflight_002`). The lab's own `name` follows the same rule: `lab_settings_006` and `007`.

## `node_kind`: a node is one of the kinds its model takes

**Statement:** a node's model is one of the model names Sherpa knows, and its kind is its `kind`, or else its model's default kind; a kind the model does not take is refused naming the kinds it takes, and a `generic` node, which has no default kind, must set one.
**Relied on by:** the user; every check and every setting that depends on the kind; image selection, which gives a node an image of its kind only. **Priority:** P0.

`arista_eos`, `juniper_evolved` and `sonic_linux` come as a virtual machine, the default, or a container. `generic` comes as any of the three kinds and has no default. Every other model takes one kind.

| Case | What it shows | Level |
|---|---|---|
| 001 | A node without `kind` is its model's default kind: a virtual machine for `arista_eos`, `juniper_evolved` and `sonic_linux` | unit |
| 002 | A `generic` node without `kind` is refused, naming the node and the three kinds, and with each kind is a node of that kind | unit |
| 003 | A `kind` the model does not take is refused, naming the node, the kind, the model and the kinds it comes as | unit |
| 004 | A model and kind read from JSON, as a stored record or a request holds them, are refused when the model does not take the kind | unit |
| 005 | Every model and kind has built-in settings, and they are of that model and kind | unit |
| 006 | A message names a model with its kind only when the model takes several: `cisco_asa`, but `arista_eos, container` | unit |
| 007 | Every model is taken by the name it is shown as, from a manifest, from JSON and on the command line | unit |
| 008 | A model name Sherpa does not know is refused, naming it, and a name that was renamed is refused naming the model and kind to use instead, from a manifest, from JSON and on the command line | unit |
| 009 | A `--model` option lists every model name as a possible value | unit |

## `node_settings`: a node sets only what its kind and its model take

**Statement:** a node that sets a key its kind does not take, or the group of another kind, is refused naming the node, the key and the kind, and for a model that takes several kinds the kind that takes it; a setting only some models take, or that a model bounds, is refused on a node it does not fit; `text_files`, `startup_scripts` and `systemd_units` are refused, naming the node, the setting and the model, on a node whose first-boot config does not carry them.
**Relied on by:** the user, whose setting would otherwise be dropped without a word. **Priority:** P1.

| Setting | Virtual machine | Container | Unikernel |
|---|---|---|---|
| `memory`, `cpu.count`, `interfaces.data_count`, `ready_check.port`, `ready_check.skip` | yes | yes | yes |
| `cpu.mode`, `cpu.topology`, `cpu.pinning`, `cpu.emulator_pinning`, `numa.node`, `numa.memory_mode` | yes | | |
| `boot_disk.size` | yes | | yes |
| `boot_disk.clone_method`, `boot_disk.cache`, `boot_disk.io`, `boot_disk.io_thread_cpus`, `interfaces.queues` | yes | | |
| `ztp.method`, `ztp.config`, `environment_variables` | yes | yes | |
| `vm` | yes | | |
| `container` | | yes | |
| `unikernel` | | | yes |

The first-boot config a node gets follows from the ZTP setting it runs with, the manifest's or else its image's, and that config alone takes a node's files and scripts. The client checks a node against its model and kind's built-in image at `sherpa validate`, `sherpa up` and `sherpa redeploy`; the server checks it against the image it builds the node from, at up and at redeploy, before anything is built or torn down.

| ZTP setting | Nodes | `text_files` | `startup_scripts` | `systemd_units` |
|---|---|---|---|---|
| `cloud-init` | Linux and BSD virtual machines (cloud-init), Windows (cloudbase-init) | yes | yes | |
| `ignition` | Flatcar | yes | | yes |
| any other, `disabled` among them | network devices, every container, every unikernel | | | |

A node with its own `ztp.config` takes none of the three: its config replaces the one Sherpa generates.

| Case | What it shows | Level |
|---|---|---|
| 001 | A `container` group on a virtual machine is refused, naming the node, the key, the kind and the model | unit |
| 002 | On a model that takes several kinds the refusal names the kind that takes the setting | unit |
| 003 | Each setting is taken by the kinds the table above gives it and refused on a node of any other kind (no test yet) | unit |
| 004 | The first setting a node's kind does not take is the one named, and a setting left unset is never refused | unit |
| 005 | Retired: file and script settings were taken on every virtual machine and container; see 016 to 021 | — |
| 006 | `container.platform` on a node that is not a `juniper_evolved` container, another model's or the model's virtual machine, is refused, naming the model and kind | unit |
| 007 | A `juniper_evolved` container takes as many data interfaces as its platform has: 12 on `bt`, which it is without `platform`, and 36 on `bx` | unit |
| 008 | A `juniper_evolved` container with fewer vCPUs or less memory than its image is refused, and one with more is taken | unit |
| 009 | A `juniper_evolved` container's memory may be at most 63487 MB, which leaves room for the container's own 2048 MB | unit |
| 010 | An `interfaces.data_count` of more data interfaces than the model has is refused, naming the node, the model, the count and the most it takes | unit |
| 011 | A count that, with the image's reserved interfaces, is past the last interface index is refused as an overflow | unit |
| 012 | Every model's built-in data interface count fits the interfaces the model has | unit |
| 013 | A redeploy whose manifest gives the node a setting its kind does not take is refused, and the node stays as it was (no test yet) | integration |
| 014 | A `memory`, `cpu.count`, `boot_disk.size`, `interfaces.data_count` or `ready_check.port` of 0 is refused, naming the node and the field, and 1 is taken | unit |
| 015 | A redeploy whose manifest sets one of those to 0 is refused before any node is torn down, naming the node and the field | unit |
| 016 | `text_files`, `startup_scripts` and `systemd_units` on a container are each refused, naming the node, the setting, the model and, for a model of several kinds, the kind | unit |
| 017 | Each of them on a network-device virtual machine, such as `arista_eos` or `juniper_srxv3`, is refused | unit |
| 018 | Each of them on a node whose model takes it but whose ZTP is off is refused, saying ZTP is disabled | unit |
| 019 | Each of them on a node with its own `ztp.config` is refused, saying so | unit |
| 020 | `text_files` and `startup_scripts` are taken on cloud-init Linux, BSD and Windows virtual machines, and `text_files` and `systemd_units` on Flatcar | unit |
| 021 | `startup_scripts` on Flatcar, and `systemd_units` on a cloud-init or Windows virtual machine, are refused | unit |

## `vm_tuning`: a virtual machine's tuning fits its vCPUs, disks and interfaces, and is in its domain

**Statement:** a virtual machine's `cpu`, `numa`, `boot_disk` and `interfaces` tuning is refused unless it fits the node's vCPU count, its image's disks and interfaces and the other nodes' pinned CPUs; what is taken is what its domain is defined with, and a virtual machine that sets none, and no watchdog, has none.
**Relied on by:** the user tuning a large virtual machine; libvirt, which refuses or misplaces a domain with settings that do not add up. **Priority:** P2.

| Case | What it shows | Level |
|---|---|---|
| 001 | A virtual machine with every tuning setting, each fitting the others, is taken | unit |
| 002 | `cpu.pinning` without `cpu.count` is refused, saying to set it; `cpu.topology` and `interfaces.queues` need it too | unit |
| 003 | A `cpu.topology` whose sockets, cores and threads do not multiply to `cpu.count` is refused, giving both numbers | unit |
| 004 | A `cpu.pinning` that does not list one host CPU for each vCPU is refused | unit |
| 005 | A `cpu.pinning` that lists a host CPU twice is refused, naming the CPU | unit |
| 006 | `cpu.emulator_pinning` or `boot_disk.io_thread_cpus` on a host CPU pinned to one of the node's own vCPUs is refused, naming the CPU | unit |
| 007 | An empty `cpu.emulator_pinning` or `boot_disk.io_thread_cpus` is refused | unit |
| 008 | `numa.memory_mode` without `numa.node` is refused | unit |
| 009 | `boot_disk.io = "native"` without `boot_disk.cache = "none"` is refused | unit |
| 010 | `boot_disk.io_thread_cpus` on a node whose image does not have virtio disks is refused, naming the model and its disk bus | unit |
| 011 | `interfaces.queues` of more than `cpu.count` is refused, giving the range | unit |
| 012 | `interfaces.queues` on a node whose image does not have virtio interfaces is refused, naming the model and its interface type | unit |
| 013 | Two nodes of a manifest that pin a vCPU to the same host CPU are refused, naming the CPU and both nodes | unit |
| 014 | A node's QEMU or I/O threads on a host CPU pinned to another node's vCPU are refused, naming the CPU and both nodes | unit |
| 015 | The QEMU and I/O threads of two nodes may share host CPUs | unit |
| 016 | `interfaces.queues = 0` is refused | unit |
| 017 | A domain defined with no tuning has no pinning, NUMA, I/O thread, topology, queue, cache or I/O element | unit |
| 018 | `cpu.pinning`, `cpu.emulator_pinning` and `boot_disk.io_thread_cpus` are the domain's vCPU pins, emulator pin and one pinned I/O thread | unit |
| 019 | A node's settings become its domain's tuning: the pins, the NUMA node held strictly unless told otherwise, and the queues, on the management interface only when it is virtio; a node with none gives none | unit |
| 020 | `numa.node` places the domain's memory on that node, strictly, or as `numa.memory_mode` says | unit |
| 021 | `cpu.mode = "host-passthrough"` and `cpu.topology` are the domain's CPU mode and topology | unit |
| 022 | `boot_disk.cache` and `boot_disk.io` are on the domain's disk driver | unit |
| 023 | `interfaces.queues` is the queue count of each virtio interface of the domain | unit |
| 024 | A domain has a watchdog only when its node sets `vm.watchdog`, one, with that action | unit |
| 025 | A virtual machine brought up with `cpu.pinning` and `numa.node` runs its vCPUs on those host CPUs and holds its memory on that node (no test yet) | integration |

`sherpa validate` checks cases 010 and 012 against the model's built-in disk bus and interface type; up and redeploy check them against the image the node gets. A host CPU another lab's node holds is `lab_preflight_005`, and a CPU or NUMA node the server does not have is refused at admission, in lab lifecycle.

## `manifest_links`: each end of a link or a bridge is a free data interface of a defined node

**Statement:** each end of a link, and each link of a bridge, is written `node::interface` and names a node of the manifest and one of that node's data interfaces, which no other link or bridge uses; an end on the management interface, on a reserved interface or past the node's data interface count is refused, and so is a link's impairment that netem cannot apply.
**Relied on by:** the user; the link and bridge build, which is given node and interface indexes; `sherpa link impair` and the API, which name a link by its place in `links`. **Priority:** P0.

| Case | What it shows | Level |
|---|---|---|
| 001 | A link's two ends resolve to their nodes, the nodes' places in the manifest, and the interfaces' indexes | unit |
| 002 | Links are numbered from 0 in the order the manifest lists them | unit |
| 003 | An end that names a node the manifest does not define is refused, naming it, at either end | unit |
| 004 | An end that is not written `node::interface` is refused | unit |
| 005 | An interface name the node's model does not have is refused, naming it (no test yet) | unit |
| 006 | The management interface is refused in a link and in a bridge, saying so | unit |
| 007 | An interface a model reserves is refused, and the first interface after the reserved ones is taken | unit |
| 008 | An interface past the node's last data interface is refused, naming the highest index the node takes, in a link and in a bridge, and the last data interface is taken | unit |
| 009 | An interface used twice is refused, naming the node and the interface's index: by two links, by a link and a bridge, and by two bridges | unit |
| 010 | A bridge's links resolve as a link's ends do, and the bridge is named on the host by its place in `bridges` and the lab's ID | unit |
| 011 | A bridge link that names a node the manifest does not define is refused, naming it | unit |
| 012 | A link that sets `p2p`, to either value, is refused, naming the link and saying to delete the key | unit |
| 013 | A link impairment with a delay or jitter over 274877 ms, or a percentage outside 0 to 100, is refused, naming the link and the field, and one within is taken | unit |
| 014 | At up the server refuses a manifest of bridges alone that puts one interface on two bridges, naming the node and the interface's index | unit |

The count that bounds case 008 is the node's `interfaces.data_count`, or else its image's (`node_settings_010`, `node_interfaces_008`).

## `manifest_clusters`: a Talos cluster is defined from the lab's own nodes

**Statement:** a `[[talos_clusters]]` entry has a name that is a lowercase DNS label, used once, one or three controllers, members that are `talos_linux` nodes of the manifest, each in one cluster once, and an `oidc` and a `vault` that name a `keycloak_server` and a `hashicorp_vault` node of the manifest; a lab has at most two clusters with three controllers and at most two with Cilium.
**Relied on by:** the cluster build, which takes every member, role and address from the definition; etcd, which needs a majority of controllers. **Priority:** P1.

| Case | What it shows | Level |
|---|---|---|
| 001 | `sherpa validate` refuses a manifest whose cluster has two controllers, exits 1 and says a cluster needs 1 or 3 | e2e |
| 002 | A cluster name that is empty, has an uppercase letter or `_`, or starts or ends with `-`, is refused | unit |
| 003 | Two clusters with one name are refused, naming it | unit |
| 004 | A cluster with no, two or four controllers is refused, naming the cluster and its count, and one with three is taken | unit |
| 005 | A member that is not a node of the manifest is refused, naming the cluster and the node | unit |
| 006 | A member that is not a `talos_linux` node is refused, naming its model | unit |
| 007 | A node listed twice, as a worker twice, as a controller and a worker, or in two clusters, is refused, naming it | unit |
| 008 | A third cluster with three controllers is refused: a lab has two shared cluster addresses | unit |
| 009 | Two Cilium clusters are taken and a third is refused: a lab has load balancer addresses for two | unit |
| 010 | `oidc` and `vault` that name a `keycloak_server` and a `hashicorp_vault` node are taken; one that names no node, or a node of another model, is refused, naming the model it must be | unit |
| 011 | A node's cluster and role are those of the list it is in, the members of a cluster are its controllers in the order written, then its workers, and a node in no list is in no cluster | unit |
| 012 | Retired: a redeploy no longer takes the node's cluster and role from the manifest it is sent; see `cluster_redeploy_009` and `010` | — |

## `manifest_provisioners`: a provisioner is on a node it can run on and names what it needs

**Statement:** an `ssh` provisioner is taken on a Linux or BSD virtual machine and a `container` provisioner on a container; each must have something to do, a file's `dst` on the node is an absolute or `~/` path of plain name parts, a mode is octal digits up to 777, and the env files and the secret files of a node each have a name of their own.
**Relied on by:** the CLI, which puts `dst` and the env file names into commands and paths on the node; the user, told before the lab is built and not after. **Priority:** P1.

| Case | What it shows | Level |
|---|---|---|
| 001 | A `container` provisioner on a virtual machine is refused, naming the node | unit |
| 002 | A provisioner with nothing to do is refused: an `ssh` one with no files, env files or scripts, and a `container` one with no files | unit |
| 003 | Two secret files of a node with one `name` are refused, naming the name and the path both would be mounted at | unit |
| 004 | An empty `src`, of a provisioner file or a secret file, and an empty script path are refused | unit |
| 005 | An `ssh` provisioner with files, env files and scripts is taken on Linux and BSD virtual machines: Ubuntu, Alpine, Flatcar, FreeBSD and OpenBSD | unit |
| 006 | An `ssh` provisioner on a network device virtual machine, a container, a Windows node or a unikernel is refused, naming the node | unit |
| 007 | An `ssh` provisioner is taken on a `talos_linux` node and the other Linux virtual machines it cannot run on (#782) (no test yet) | unit |
| 008 | A file `dst` is refused unless it is absolute or starts with `~/`, names a file, and has only parts of letters, digits and `. _ - + @`: no empty, `.` or `..` part, no space, quote, `$` or `;` | unit |
| 009 | A mode is refused unless it is octal digits up to 777, with a message giving the mode | unit |
| 010 | A provisioner file's `mode` that is not such digits is refused, naming the file's `dst` | unit |
| 011 | An env file must name a file with a plain name, and two env files of a node with one file name are refused, naming it | unit |

`sherpa validate`, up, redeploy and `sherpa provision` run these on the manifest; the server runs them on the manifest as written before it stores it. What a request's secret files must be is `container_secrets_010`, `012` and `013`.

## `boot_settings`: a node's first-boot settings are well formed

**Statement:** a text file's `permissions` are octal digits up to 777, in the files as written and in the contents the CLI sends, and `sherpa validate`, up and redeploy, in the CLI and on the server, take an `environment_variables` entry only as `KEY=VALUE` whose key is a name of letters, digits and `_` that does not start with a digit.
**Relied on by:** the node's first-boot config, where the key is written into a shell file and a mode into a file's permissions. **Priority:** P1.

| Case | What it shows | Level |
|---|---|---|
| 001 | A text file whose `permissions` are not octal digits up to 777 is refused, naming the node and the file's `dst`, in the files as written and in the contents the CLI read | unit |
| 002 | An entry with a name as its key is taken: a literal value, a `$VAR`, an empty value and a value with `=` in it | unit |
| 003 | An entry with no `=`, an empty key, a key that starts with a digit or a key with another character is refused, naming the entry or the key and the node | unit |
| 004 | Retired: `sherpa validate` now refuses an entry whose key is not a name; see 008 | — |
| 005 | Retired: a redeploy now runs both checks; see 006 and 007 | — |
| 006 | A redeploy whose manifest has an `environment_variables` key that is not a name is refused before any node is torn down, naming the key and the node | unit |
| 007 | A redeploy whose manifest gives a Flatcar node a text file mode that is not octal digits up to 777 is refused before any node is torn down, naming the node and the file's `dst` | unit |
| 008 | `sherpa validate` refuses a manifest with an `environment_variables` key that is not a name, naming the key and the node | unit |

## `lab_settings`: the lab's own settings are checked before it is built

**Statement:** a lab's `name` is 1 to 63 ASCII letters, digits, `-` or `_`, starting with a letter or digit, as a node's is; its `description` is at most 1000 characters with no control character but line breaks and tabs, a lab with `[tailscale]` enabled names the environment variable that holds its key, never the key, `provision_concurrency` is from 1 to 255 and `ready_timeout` is at least 1.
**Relied on by:** the status messages, log fields and web UI download file name (`<name>-<lab ID>.zip`) the lab's name is put in; every page and command that shows a lab's description; the tailnet key, which is never in a manifest; the CLI's provisioning and the server's wait for nodes to be ready. **Priority:** P2.

| Case | What it shows | Level |
|---|---|---|
| 001 | A description that is left out, empty, or 1000 characters of several lines with tabs and non-ASCII letters is taken | unit |
| 002 | A description of 1001 characters is refused, giving its length | unit |
| 003 | A description with a control character other than a line break or a tab, such as an escape or a NUL, is refused | unit |
| 004 | With `[tailscale]` enabled, an `auth_key_env` that is left out, empty, starts with a digit, or is a key, a `$NAME` or a `NAME=value`, is refused; a name of letters, digits and `_` is taken | unit |
| 005 | A `provision_concurrency` of 0 is refused and 1 is taken | unit |
| 006 | A plain lab name is taken: letters of either case, digits, `-` and `_` after the first character, and 63 characters | unit |
| 007 | An empty lab name, one that is path-like or shell-like, one with a dot, a space, a quote, markup, a line break or a non-ASCII letter, and one of 64 characters, are refused, naming the name and the rule | unit |
| 008 | A `ready_timeout` of 0 is refused and 1 is taken | unit |
| 009 | A redeploy whose manifest has a lab name the rule refuses is refused before any node is torn down, naming the name | unit |
| 010 | A redeploy whose manifest has a `ready_timeout` of 0 is refused before any node is torn down | unit |

The CLI checks the lab's name, `ready_timeout` and the node settings that cannot be 0 at `sherpa validate`, up and redeploy, and the server at up before anything is created and at redeploy before the node is torn down; a lab stored before the rule is refused at its next up or redeploy like any other manifest.
