# Nodes

What a node does and reports inside the lab operations: how a virtual machine, a container and a unikernel boot, when each counts as ready, the state each reports, how each is stopped and started, and how a node failure shows. The lab-level outcome of up, down, resume, destroy and redeploy, and the status a lab reports, are in [lab lifecycle](lab-lifecycle.md); manifests (which kinds a model takes, which settings a kind takes), links, images, owner logins, consoles, ZTP, provisioning and Talos have their own specs.

Each behaviour lists its cases. A case is one test, whose name is its ID: `<behaviour>_<NNN>_<description>`, such as `node_state_003_a_container_removed_outside_sherpa_is_recorded_missing_and_its_lab_failed`. The level is the highest that can show the case: e2e through the `sherpa` CLI, integration against real libvirt, Docker and the database, unit only for logic with too many branches to cover above.

## `vm_start`: a virtual machine boots from its own disk

**Statement:** a virtual machine node boots from its own disk, with the resources its manifest sets.
**Relied on by:** the user, every other node booted from the same image. **Priority:** P0.

| Case | What it shows | Level |
|---|---|---|
| 001 | What a VM writes to its overlay disk changes neither the base image nor another node's disk on the same image | integration |
| 002 | `boot_disk.size` gives the node a disk of that size and leaves the base image's size as it was | integration |
| 003 | A `boot_disk.size` smaller than the base image is refused; one equal to it or larger is taken | unit |
| 004 | `boot_disk.clone_method = "copy"` gives the node a full copy with the base image's contents | integration |
| 005 | A boot disk is never created over a disk that exists, by overlay or by copy, and the existing disk keeps its contents | integration |
| 006 | A copy that fails leaves no new volume behind | integration |
| 007 | A base image that cannot back an overlay (encrypted, an external data file, itself an overlay, or a damaged header) is refused, not booted | unit |
| 008 | A VM runs with the manifest's `memory` and `cpu.count`, else its image's (no test at this level yet) | integration |
| 009 | A lab whose VM image cannot back an overlay is refused by up before anything is created, and comes up with `clone_method = "copy"` (no test yet) | integration |
| 010 | A node's disks in the storage pool, an overlay or a copy, and its config media there are private, mode 0600, whatever the mode of their source | integration |
| 011 | The manifest's `boot_disk.clone_method` reaches each VM node through up, with copy, overlay and the default kept apart | unit |

## `unikernel_start`: a unikernel boots its own copy of its image

**Statement:** a unikernel node boots its own copy of its image.
**Relied on by:** the user. **Priority:** P2.

| Case | What it shows | Level |
|---|---|---|
| 001 | A direct-kernel unikernel comes up and is `running` once its domain runs, with the image file unchanged (no test yet) | integration |
| 002 | `unikernel.cmdline` is the command line the kernel boots with (no test yet) | integration |
| 003 | A disk-boot unikernel boots from a copy of its disk, at `boot_disk.size` when set (no test yet) | integration |
| 004 | A redeploy of a unikernel is refused, and the node is left running (no test yet) | integration |

## `container_run`: a container runs as its manifest and model say

**Statement:** a container node runs as its manifest and its model say.
**Relied on by:** the user, the host's capacity. **Priority:** P0.

| Case | What it shows | Level |
|---|---|---|
| 001 | A container of a model with no start settings of its own (`generic`, `mysql_db`, `postgresql_db`) starts with the manifest's image, command and environment, and nothing added (no test at this level yet) | integration |
| 002 | The manifest's `environment_variables` are set in the container | integration |
| 003 | A variable the user sets replaces the one Sherpa sets under the same key | unit |
| 004 | `container.privileged` runs the container privileged | integration |
| 005 | A container with its ZTP off keeps the manifest's `container.volumes` and the image's own command, at up and after a redeploy | integration |
| 006 | `container.user` and `container.shm_size` apply to the container (no test yet) | integration |
| 007 | A container's hostname is its node name, and it keeps it across down, resume and redeploy (no test yet) | e2e |
| 008 | `memory` and `cpu.count` are a hard memory limit with no swap beyond it and a CPU quota | integration |
| 009 | A container that sets neither runs without limits, whatever its image's values (no test at this level yet) | integration |
| 010 | A `juniper_evolved` container's limit is its router's memory plus 2048 MB, with no CPU quota | unit |
| 011 | A `cisco_iol` or `cisco_ioll2` container's limit is its router's memory plus 128 MB, with the manifest's CPU quota | unit |
| 012 | A container that cannot be attached to one of its networks is removed, and the error is reported | integration |
| 013 | A `cisco_iol` container runs unprivileged, holds its management address and relays its interfaces | integration |

## `node_vm`: a privileged network OS container runs in a VM of its own

**Statement:** an `arista_eos`, `nokia_srlinux` or `sonic_linux` container node runs privileged on the Docker of a Flatcar VM of its own, which a unit in the VM starts from what sherpad put on the VM's config drive, boots on an overlay of its image's Docker store instead of copying the image, reports a failure with its reason, and has its links, management addresses, console, saved config and lifecycle as a node's.
**Relied on by:** the server's owner (a node's privileges reach its VM, not the server), the user. **Priority:** P0.

| Case | What it shows | Level |
|---|---|---|
| 001 | The node VM boots the base image's kernel with its command line, on its overlay disk and its Docker store overlay, with its config drive read-only | unit |
| 002 | Each data interface on a link or bridge is a tap named like its link end, and the management NIC is on the lab's management network | unit |
| 003 | Retired: the node VM has no vsock; see 048 | — |
| 004 | Retired: sherpad reads no vsock ID; see 048 | — |
| 005 | Values in the domain XML are escaped | unit |
| 006 | Retired: no files are archived into a container; see 012 | — |
| 007 | Retired: no files are archived into a container; see 012 | — |
| 008 | Updates, update reboots, SSH and console logins are off in the node VM and the Docker store VM | unit |
| 009 | Retired: sherpad drives no Docker in the VM; see 040 and 049 | — |
| 010 | Before Docker starts, the guest names each NIC, the management NIC `eth0`, by its lowercase MAC, and the VM runs no network manager | unit |
| 011 | The serial console runs the network OS CLI | unit |
| 012 | The container's volumes go on the config drive: a read-only one is mounted from the drive, a writable one is copied into the container when it is created | unit |
| 013 | The console runs each model's CLI | unit |
| 014 | Retired: the VM has no certificates; see 049 | — |
| 015 | Retired: no model has a step of its own after its container starts; see 044 | — |
| 016 | A node VM gets its container's memory, the manifest's or the image's, with 1024 MB on top, and its vCPUs | unit |
| 017 | The containers of the three models run in node VMs and are handled as VMs; other containers stay Docker's | unit |
| 018 | A lab with a node VM is refused before anything is created when the base image or its kernel is missing | unit |
| 019 | A node VM boots on its image's Docker store, which a store VM makes from the image's archive, its unit starts the container and reports it started, and removing it leaves no domain or volume | integration |
| 020 | The three models come up in node VMs and answer SSH with the owner's keys (no test yet) | e2e |
| 021 | A P2p link between two node VMs, and between a node VM and a VM, passes traffic both ways (no test yet) | e2e |
| 022 | An impairment set on a node VM's link applies, and clearing it removes it (no test yet) | e2e |
| 023 | `sherpa console` reaches each model's CLI (no test yet) | e2e |
| 024 | A config saved on the node survives down and resume, and its links are re-attached (no test yet) | e2e |
| 025 | A redeploy gives the node a new disk and config drive with its generated config only, and re-attaches its links (no test yet) | e2e |
| 026 | Destroy leaves no domain, volume, tap, eBPF filter, process or folder of the node VMs (no test yet) | e2e |
| 027 | An up that fails while creating node VMs is rolled back with nothing of the lab left (no test yet) | e2e |
| 028 | A restart of sherpad leaves the node VMs running (no test yet) | e2e |
| 029 | SR Linux's management address answers after up, after down and resume, and after a redeploy (no test yet) | e2e |
| 030 | A Docker store is named by the image's ID, and an ID that is not a SHA-256 gets none | unit |
| 031 | The VM that makes a Docker store has no NICs and no console, and its image archive is a read-only disk | unit |
| 032 | The kernel command line of a domain is replaced and the rest of its XML kept; XML without one is refused | unit |
| 033 | The `/usr` hash is read from the kernel's header, and a header without one is refused | unit |
| 034 | A node VM boots without a kernel console, and runs Ignition on its first boot only | unit |
| 035 | Each data interface on no link is listed with its fixed MAC, for the guest to make a veth whose peer is down in a namespace of its own | unit |
| 036 | Docker keeps its data on the Docker store disk, which the store's VM formats and a node VM mounts | unit |
| 037 | Destroy takes a node VM's Docker disk and never a Docker store | unit |
| 038 | A second lab with the same image boots on the store the first made, with no image copy (no test yet) | e2e |
| 039 | A node VM's first boot does not rebuild the library cache or the hardware database | unit |
| 040 | Docker in a node VM listens on its local socket only and sets up no iptables rules | unit |
| 041 | containerd in a node VM runs without the CRI plugins, on its usual socket | unit |
| 042 | A node VM mounts its root filesystem without zeroing its inode tables, so its disk holds only what it writes | unit |
| 043 | Retired: the node VM gives every model its management addresses; see 045 and 046 | — |
| 044 | A node VM's container is created in the VM's network namespace, with no Docker network, the lab's name servers and domain, its hostname resolving, its mounts, privileges and command, Docker's `unless-stopped` restart policy and no image pull | unit |
| 045 | A node VM's management NIC is given the node's IPv4 address, and its IPv6 address when the node and the lab's network have one, each with the network's prefix length and its gateway, as Docker gives a container on the network | unit |
| 046 | Before Docker starts, the guest is given the management addresses and a default route through each gateway on `eth0` | unit |
| 047 | A node in a node VM answers SSH on its management IPv4 and IPv6 addresses with ZTP on, with ZTP off, and with a config of its own that sets no management address, after up, resume and redeploy (no test yet) | e2e |
| 048 | The node VM has its serial console on the node's console socket and its status channel on its status socket | unit |
| 049 | A unit starts the container after Docker and the interface naming, from the arguments sherpad rendered | unit |
| 050 | The node VM mounts its config drive, and lists its writable volumes to copy off it into the container | unit |
| 051 | Both VMs report their status, and the store VM loads its image archive | unit |
| 052 | The config drive holds the Ignition config and each volume's file or folder, and a missing volume source fails it | unit |
| 053 | A failure the VM reports fails it with the reason, and a status other than the one awaited fails it | unit |
| 054 | The update installs `bzip2` when `bunzip2` is missing, and nothing otherwise | unit (bats) |
| 055 | A node VM whose container cannot be started is failed by `up` with the reason the VM reports, before the readiness wait ends (no test yet) | e2e |
| 056 | A data interface on a private bridge is a bridge interface of the domain, so libvirt puts its tap on the bridge at every start | unit |
| 057 | A first boot whose copy of the writable volumes into the container fails removes the container, starts nothing and reports why | unit |
| 058 | A container that a first boot left without its writable volumes is removed and created again with them at the next boot | unit |
| 059 | A boot after the container was made with its volumes only starts it | unit |

## `node_ready`: a node is running only once its ready check passes

**Statement:** a node is reported `running` only once its ready check passes.
**Relied on by:** the user, provisioners and scripts that run once up returns. **Priority:** P0.

| Case | What it shows | Level |
|---|---|---|
| 001 | On port 22 a port that answers but is not an SSH server is not ready | unit |
| 002 | On any other port an accepted TCP connection is ready | unit |
| 003 | A closed port is not ready | unit |
| 004 | An SSH server that sends lines before its identification line is still read as an SSH server | unit |
| 005 | A virtual machine is checked on its model's port, SSH or the Talos API for `talos_linux`, unless `ready_check.port` sets another; a container or unikernel has no port unless it sets one | unit |
| 006 | A `sonic_linux` container is checked on SSH unless the manifest sets another port or skips the check | unit |
| 007 | A Windows node with cloud-init is not ready on the image's own SSH server, only once its SSH server shows Sherpa's ready marker | integration |
| 008 | The ready marker counts only as a comment of the SSH identification line, whole, not as part of another word or of the version | unit |
| 009 | A Windows node needs the marker only when it is checked on SSH and built with cloud-init | unit |
| 010 | A container running for less than 5 seconds is not ready yet; at 5 seconds, with no port, it is | unit |
| 011 | A container whose `ready_check.port` already answers is still waited on for its 5 seconds | unit |
| 012 | A container that has kept running but whose `ready_check.port` does not answer is waited for, not ready | unit |
| 013 | A container that has kept running and whose `ready_check.port` answers is ready | unit |
| 014 | A container Docker reports paused, restarting or being removed is waited for, neither ready nor failed | unit |
| 015 | A container with `ready_check.port` is `starting`, with `Waiting for port <port>` shown, until the port answers, then `running` (no test yet) | integration |
| 016 | With `ready_check.skip` a VM is `running` when up returns, before its port answers (no test at this level yet) | integration |
| 017 | With `ready_check.skip` a container is still started, and is `running` at once (no test at this level yet) | integration |
| 018 | With `ready_check.skip` a container's `ready_check.port` is not waited for | unit |
| 019 | A node not ready within `ready_timeout` is reported as not ready, stays `starting`, and up still succeeds (no test yet) | integration |
| 020 | A unikernel with `ready_check.port` is ready only once its domain runs and the port answers (no test yet) | integration |
| 021 | Resume waits for a node it boots on the port recorded for it at up, and for a unikernel on its domain and its port | unit |
| 022 | Resume does not wait for a node it unpauses, one with a skipped check, or a container with no recorded port | unit |
| 023 | Resume waits for a Windows node's ready marker, as up does | unit |
| 024 | Resume's wait is the `ready_timeout` of the manifest the lab was created with, or the default when none is stored | integration |
| 025 | Resume's result for a node says it started and is ready, or that it was not ready within the timeout | unit |
| 026 | A node resume boots is `starting` until its recorded check passes, then `running`; one not ready in time stays `starting` and resume still succeeds (no test with its ID yet; `lab_down_resume_002` waits on a VM) | e2e |
| 027 | A redeployed node waits on its recorded port, at once on a skipped check, and otherwise until it runs | unit |
| 028 | A redeploy records the ready check of the manifest it is given before it waits, so a changed port or skip is the one waited on from then on | integration |
| 029 | The scanner records a running VM `running` only once its recorded port answers; until then it is `starting` | unit |
| 030 | The scanner probes a node on its recorded port when that is not SSH | unit |
| 031 | A container whose recorded port does not answer stays `starting` scan after scan | unit |
| 032 | A node with no recorded port is `running` as soon as the scanner sees it run | unit |
| 033 | A node with a skipped check is `running` as soon as the scanner sees it run, whatever its port | unit |
| 034 | The scanner does not probe a node that is not running, or one already recorded `running` | unit |
| 035 | The scanner waits for a Windows node's ready marker, as up does | integration |
| 036 | A cEOS container is checked on SSH only with the config Sherpa generates, and an SR Linux container unless it has a config of its own; otherwise, with no `ready_check.port`, it is ready once its container runs, as under runc, since its management address may not answer | unit |

Cases 001 to 020 are up, 021 to 026 resume, 027 and 028 redeploy, 029 to 035 the server's scanner.

## `node_state`: a node's state follows its VM or container

**Statement:** a node's reported state follows what its virtual machine or container is doing.
**Relied on by:** the user, the web UI, scripts reading `sherpa inspect`. **Priority:** P0.

| Case | What it shows | Level |
|---|---|---|
| 001 | While up builds a lab, neither the lab nor any of its nodes is recorded stopped or missing | integration |
| 002 | A container stopped and started outside Sherpa is recorded `stopped`, then `running` | integration |
| 003 | A container removed outside Sherpa is recorded `missing`, and its lab `failed` | integration |
| 004 | Inspect reports a container that exited since it was recorded `running` as `stopped` at once, the lab's status with it, and records nothing | integration |
| 005 | Inspect reports a container that is gone as `missing` | unit |
| 006 | Inspect reports a container as the scanner would record it: a `failed` or `building` one as recorded, a dead one `failed`, a paused one `stopped` | unit |
| 007 | Inspect reports a running container still waiting for its recorded port as `starting` | unit |
| 008 | Inspect reports a container as recorded when Docker does not answer | unit |
| 009 | Inspect reports a virtual machine as recorded | unit |
| 010 | A node an operation starts is `starting`, and only its ready check makes it `running` | unit |
| 011 | Nothing libvirt or Docker shows moves a node that is `building` | unit |
| 012 | When libvirt or Docker cannot be asked, that runtime's nodes keep their recorded state | unit |
| 013 | A node an operation stopped while another waited on it is not recorded `running` by the waiting one | integration |
| 014 | What the scanner saw before an operation moved a node does not overwrite the operation's state | integration |
| 015 | A node state this version does not know reads as `unknown`, and the states it knows (`building`, `starting`, `running`, `stopped`, `failed`, `missing`) read back as written | unit |
| 016 | A VM shut down or started outside Sherpa is recorded `stopped`, then `running` once its ready port answers (no test yet) | integration |
| 017 | A node between states is left as recorded by what the scanner sees (no test yet) | integration |
| 018 | A paused node is recorded `stopped` (no test yet) | integration |
| 019 | A `missing` node found again is recorded `stopped` (no test yet) | integration |
| 020 | The web UI offers a node only the actions its state allows: stop when it runs, start when it is stopped or failed, only redeploy when missing, none while building | unit |

## `node_down`: down stops each node as its kind and state allow

**Statement:** down stops each node as its kind and its state allow.
**Relied on by:** the user. **Priority:** P0.

| Case | What it shows | Level |
|---|---|---|
| 001 | Down asks a virtual machine to shut down, powers a unikernel off and pauses a container | unit |
| 002 | A VM's result says it shut down, or that it was forced off and why: still running after 60 seconds, or the request refused, with the reference of the logged error | unit |
| 003 | Down pauses only a running container; one already paused or not running is reported as such and is not an error | unit |
| 004 | A second down of a lab that is down succeeds, each node reported already stopped or paused (no test yet) | integration |
| 005 | Down leaves a node an operation is building alone, and the refusal carries its `building` state | unit |
| 006 | `--node` downs that node alone (no test with its ID yet; `tests/labs/talos-linux` uses it) | e2e |

## `node_resume`: resume brings each node back as its kind and state allow

**Statement:** resume brings each node back as its kind and its state allow.
**Relied on by:** the user. **Priority:** P0.

| Case | What it shows | Level |
|---|---|---|
| 001 | Resume boots a node that is shut off or exited, unpauses a paused one and leaves a running one alone | unit |
| 002 | Resume refuses a crashed VM and a dead container, saying to redeploy it, and a node between states, saying which | unit |
| 003 | Resume of a `missing` node is refused, saying to redeploy it | integration |
| 004 | A container stopped outside Sherpa is started by resume and reachable again | e2e |
| 005 | `--node` resumes that node alone (no test with its ID yet; `tests/labs/talos-linux` uses it) | e2e |
| 006 | A `cisco_iol` container keeps a config saved with `write memory` across a cold boot, and one with nothing saved comes back with its startup config | e2e |

## `node_failure`: a failed node stays failed, with the reason

**Statement:** a node that fails is reported `failed` with the reason, and stays `failed` until the user acts on it.
**Relied on by:** the user. **Priority:** P0.

| Case | What it shows | Level |
|---|---|---|
| 001 | A container found exited, dead, never started or gone before it was ready has failed, however long it ran | unit |
| 002 | A container that exits while its `ready_check.port` is waited for has failed, and is not left `starting` | unit |
| 003 | A container not running right after its start is a failed node even with `ready_check.skip` (no test yet) | integration |
| 004 | A container that exits after it was ready but before up returns is reported and recorded `failed` (no test yet) | integration |
| 005 | The error for a container with no exit code or log still says how it ended and where to find its log | unit |
| 006 | A failed container's log lines are reported without terminal control characters | unit |
| 007 | A crashed VM or a dead container is recorded `failed`, not `stopped` | unit |
| 008 | A `failed` node stays `failed` scan after scan, whether its container or VM is then running, stopped or gone, and its lab is `failed` | integration |
| 009 | A `failed` node can be resumed and redeployed | unit |
| 010 | When the server starts, a node an up or a redeploy was building is recorded `failed` | integration |

`lab_up_004`, `lab_up_005` and `lab_redeploy_005` in [lab lifecycle](lab-lifecycle.md) show a failed node through up and redeploy: the exit code and log lines in the error, the rest of the lab kept, the non-zero exit. The cases here are the ways a node comes to be failed and stays so.
