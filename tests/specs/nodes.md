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
| 005 | Down leaves a node an operation is building alone, saying to wait | unit |
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
