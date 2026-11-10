# Host OOM kills of lab VMs (#514)

## Symptom

On 2026-11-09 the host's OOM killer killed two VMs on the dev server (2 NUMA nodes of about
82 GB, 157 GB in all):

1. **Global OOM.** The guests on the host were configured with about 148 GB together: dev
   48 GiB (`numa_node = 1`), agent 24 GiB (`numa_node = 0`), the ubuntu-2404 and ubuntu-2604
   release test VMs 30 GiB each, unpinned, and dev-windows 16 GiB. As the guests touched their
   memory, the host ran out.
2. **NUMA node 0 OOM.** A VM pinned with `numatune mode='strict' nodeset='0'` (from
   `performance_tuning.numa_node`) could not take memory from node 1. An unpinned test VM had
   filled node 0, and the kernel killed it.

The release test harness reported the dead guest as `exited 255`, the SSH exit code when the
connection drops.

## Findings

- Nothing checked memory before starting a node. `up`, `redeploy` and resume started VMs
  whatever the host had left. The only server-side check was CPU pins
  (`check_cpu_pins_on_server`), run in `up`'s planning phase and stored in phase 3, with no
  lock across labs, so two concurrent `up`s could both pass it (#413).
- Free memory is the wrong number to check. QEMU allocates guest memory lazily, so a lab that
  fits at start can OOM later as earlier guests touch theirs. Through the `virt` crate only
  MemFree is available (`virNodeGetFreeMemory`, `virNodeGetCellsFreeMemory`), which also counts
  reclaimable page cache as used. `virNodeGetMemoryStats` is not wrapped in `virt` 0.4.3.
- Sherpa emits a virtio `memballoon` but never sets `currentMemory` or a balloon target, and
  emits no `memoryBacking`, so a domain's maximum memory is what it may take.
- `virNodeGetInfo().nodes` is not a reliable NUMA node count; libvirt documents that it may
  report 1. The capabilities XML lists the cells.
- `strict` (and `restrictive`) NUMA memory has nowhere else to go. Unpinned guests take memory
  where their vCPUs first touch it, and the kernel does not move it off a node later, so an
  admission check cannot make `strict` safe beside unpinned guests. Only `preferred` falls
  back to other nodes.

## Changes

- Server-side admission (0.5.18): committed memory against the host total and against each
  pinned NUMA node, with a host reserve (`host_memory.reserve_mib`) and an overcommit percentage
  (`host_memory.overcommit_percent`) in `sherpa.toml`. Refused by
  default. See "Admission: memory and CPU pins" in
  [`../architecture/SERVER.md`](../architecture/SERVER.md) and section 31 of
  [`../architecture/STATE_MACHINES.md`](../architecture/STATE_MACHINES.md).
- The NUMA node count now comes from the capabilities XML.
- `performance_tuning.numa_memory_mode` (0.5.19): `strict` (the default when `numa_node` is set)
  or `preferred`, rendered as `<numatune><memory mode>`. A `preferred` VM counts only against
  the server total in admission, requested or running; only `strict` memory is held to its
  NUMA node. The dev and agent environments use `preferred`. Settled behaviour is in
  "Admission: memory and CPU pins" in [`../architecture/SERVER.md`](../architecture/SERVER.md)
  and in the user docs' manifest reference (`numa_memory_mode`).
- Provisioning trusts the project `mise.toml` in guest checkouts (separate change).

## Open

- The release test harness: run releases one at a time when the server refuses them, and
  report a guest killed by the host OOM killer (`virsh domstate --reason`, the kernel log's
  `oom-kill` line naming the domain's `machine-qemu…scope`) instead of `exited 255`.
- Whether the release test VMs need 30 GiB: record their peak memory (the domain scope's
  cgroup `memory.peak`) before changing it.
- Containers have no memory limits, so admission cannot count them; the host reserve covers
  them.
