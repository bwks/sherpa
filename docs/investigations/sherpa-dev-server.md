# Sherpa dev server: a fast, nested Sherpa VM built by Sherpa

> Status: decided. Tracked in #402. The manifest settings are built (`[nodes.performance_tuning]`,
> documented in `docs/architecture/SERVER.md`); the installer THP change and the dev environment
> are not.

## Goal

A persistent development server for Sherpa, built as a Sherpa lab by the host's Sherpa. Inside it,
the agent has root: it can build `sherpad`, install it, restart it and run the lab tests without
waiting for someone to run `sudo` on the host. It must be fast, and it must run nested VMs, since
the lab tests start vEOS, Ubuntu and other VMs inside it.

## What exists

- **Test environments** (`tests/environments/ubuntu-2404`, `ubuntu-2604`):
  - each is a Sherpa lab with one `ubuntu_linux` VM (16 vCPU, 32 GiB, 100 GiB disk);
  - `tests/harness/test_env.py` installs a release build in it with Sherpa's installer and runs
    the VM code suite;
  - its `check` already verifies nested KVM in the guest.

  These environments are disposable by design: they must contain nothing beyond cloud-init, so the
  installer is tested. A dev server is the opposite, a long-lived box with a toolchain and a
  checkout. It should be its own environment, reusing the manifest pattern and the harness's
  `up` and `ssh` plumbing, not a change to these.
- **Nested virtualization works today.** `kvm_intel nested=Y` on the host, and `cpu_model =
  host-model` resolves to `Broadwell-IBRS` plus 74 `vmx` features, so guests get `/dev/kvm`.
- **Provisioning:** a manifest can already provision a node with `text_files`, `systemd_units`,
  `startup_scripts` and SSH `provisioners`. That is enough to install the toolchain and
  dependencies from the manifest.
- **Missing (before the `performance_tuning` settings were added):** none of the performance
  settings below. The domain template
  (`crates/template/templates/libvirt/libvirt_domain.jinja`) is limited to:
  - a vCPU count;
  - `host-model` or a named CPU model;
  - memory in MiB, typed `u16`, so at most 64 GiB;
  - disks with a driver name and format only (no cache, I/O mode or I/O threads);
  - network interfaces without multiqueue.

  There is no pinning, NUMA placement, hugepages or guest CPU topology.

## Host facts (dev server, bare metal)

| | |
|---|---|
| CPU | 2 × Xeon E5-2620 v4 (Broadwell), 8 cores × 2 threads each, 32 threads |
| NUMA | node 0: CPUs 0–7, 16–23; node 1: CPUs 8–15, 24–31; about 80 GB each |
| Thread siblings | `n` and `n+16` (for example 8 and 24) |
| KVM | `nested`, `ept`, `vpid`, `enable_apicv`, `enable_shadow_vmcs`, `preemption_timer` all on |
| Hugepages | none reserved; CPU supports 2 MiB and 1 GiB pages |
| THP | `enabled = never`, `defrag = madvise` |
| QEMU / libvirt | QEMU 8.2.2, libvirt 10.0.0 |
| Storage | `/opt/sherpa` on NVMe (1.8 TB, 1.5 TB free); `/opt/sherpa/images` is 183 GB |
| Other load | the host also runs users' labs (two VMs, 4 and 8 vCPUs, at the time of writing) |

## Performance options

| Option | What it does | Expected gain | Cost and risk | Needs |
|---|---|---|---|---|
| `host-passthrough` CPU | Exposes the host CPU exactly, including all `vmx` features and cache info | Best nested performance; no feature filtering | Guest is tied to this CPU model, which is irrelevant for a non-migrating VM | A `CpuModels::HostPassthrough` variant and template arm |
| Guest CPU topology | `<topology sockets='1' cores='N/2' threads='2'/>`, matching the pinned host threads | The guest scheduler knows which vCPUs are SMT siblings | None | Template change |
| vCPU pinning | `<cputune><vcpupin>` each vCPU to one host thread on one NUMA node, siblings paired | Stable cache locality and no cross-socket migration | Pinning does not reserve CPUs: other labs can still run there unless the host isolates them (next row) | Node setting and template |
| CPU isolation (host) | Keep other VMs and containers off the pinned CPUs, for example `AllowedCPUs=` on `machine.slice` and Docker's cgroup, or `isolcpus` | The dev server's CPUs are not shared | Host-level change; takes capacity from users' labs | Host config (sudo), and a decision |
| Emulator and I/O thread pinning | `<emulatorpin>` and `<iothreadpin>` on a spare sibling pair | Keeps QEMU housekeeping and disk I/O off the vCPUs | Uses one core | Template |
| NUMA memory | `<numatune><memory mode='strict' nodeset='1'/>` | Memory local to the pinned CPUs; avoids remote accesses across QPI | The node needs that much free memory, and page cache is reclaimable | Node setting and template |
| Hugepages | `<memoryBacking><hugepages/>` from 2 MiB pages, reserved per NUMA node at runtime, or 1 GiB pages reserved at boot | Fewer TLB misses; this helps nested guests most, since every L2 page walk goes through two levels | Reserved memory is taken from the host for good; 1 GiB pages need a kernel command line change and a reboot | Host sysctl or cmdline (sudo), and a template setting |
| THP `madvise` | QEMU already advises guest RAM; the host currently ignores it (`never`) | Much of the hugepage gain without reserving memory | Host-wide setting; a small risk of compaction stalls | One sysfs write (sudo); no Sherpa change |
| virtio-blk with tuning | Disk on `virtio`, `cache='none' io='native'` (or `io_uring`), `discard='unmap'`, on an I/O thread | Large gain for `cargo` builds and Docker; SATA emulation, as the devbox model uses, is the slowest bus | `cache='none'` needs O_DIRECT, which the NVMe filesystem supports | Disk driver options in the template |
| Full disk copy | `boot_disk_clone_method = "copy"` instead of a qcow2 overlay | Removes backing-file lookups; can be preallocated | Disk space | Already supported |
| virtio-net multiqueue | `<driver queues='N'/>` on the management interface | Faster image pulls and rsync | None | Template |
| Memory size | 48–64 GiB | Room for `cargo`, Docker and nested lab VMs | `memory` is `u16` MiB, so at most 64 GiB unless it is widened | Possibly widen the field |

Not worth doing here: CPU frequency governor changes (host-wide; measure first), `kvm-clock`
tuning (already the default), and memory ballooning (not present).

## Proposed sizing

Dedicate NUMA node 1 to the dev server:
- **vCPUs:** 14, pinned as sibling pairs to host threads 9–15 and 25–31.
- **QEMU's own threads:** the emulator and I/O threads go on 8 and 24.
- **Memory:** 48 GiB, strict on node 1.
- **Disk:** 300 GiB on virtio with an I/O thread. That leaves room for the Rust `target`
  directory, Docker, and the subset of images the lab tests need, about 30–40 GB (Ubuntu, vEOS,
  cEOS, Alpine, FRR, netshoot).

Node 0 stays for users' labs. Whether node 1 is also isolated from them is a decision (below).

## Building it with Sherpa

1. **Manifest:** an environment directory, for example `tests/environments/dev`, with one
   `ubuntu_linux` 24.04 node using the sizing and tuning above. Its provisioners and startup
   scripts:
   - install the Rust toolchain, the eBPF toolchain and build dependencies;
   - run Sherpa's installer, so the guest has Docker, libvirt and SurrealDB;
   - add the agent's SSH key.
2. **Images:** not part of the build. They are copied from the host's `/opt/sherpa/images` as
   needed and registered with `sherpa server image import`. The Arista images can't be pulled from
   a public registry.
3. **Development loop**, run entirely inside the guest as root:
   - sync the working tree from the host with `rsync`;
   - `cargo build --release -p sherpad`, then install it and restart `sherpad`;
   - run the lab tests.

   Nothing on the host needs `sudo` after the server is built.
4. **Sherpa changes** needed so the manifest can express the tuning:
   - a `host-passthrough` CPU model;
   - guest CPU topology;
   - pinning (vCPU, emulator, I/O thread) and NUMA memory;
   - hugepages backing;
   - disk cache, I/O mode and I/O threads;
   - interface queues.

   This is a user-facing manifest feature and needs its own issue, docs and validation (for
   example, pinned CPUs must exist on the host and must not overlap between two VMs).

## Risks

- **Nested lab VMs are slower than on the host.** Even tuned, lab tests with several VMs, like
  p2p-links, take longer inside the dev server. Host runs stay the final check before a release.
- **Pinning without isolation** can be worse than no pinning, if another lab's busy VM lands on
  the pinned threads.
- **Transparent hugepages are not guaranteed.** On fragmented memory, some guest RAM starts as
  4 KiB pages until `khugepaged` merges it, and a VM start can stall briefly while the kernel
  compacts memory. Check `AnonHugePages` in the QEMU process's `smaps`.
- **THP is host-wide.** `madvise` affects any process that asks for huge pages, not only QEMU.
  This host had it set to `never`, and nothing in its configuration says why.
- **Two Sherpa versions.** The guest runs the development build; the host keeps the released one.
  The guest's management network must not overlap the host's lab subnets. That holds because it
  is a lab of the host: its labs live behind its NAT.

## Decisions

- **CPU:** NUMA node 1, 14 vCPUs pinned as sibling pairs to host threads 9–15 and 25–31. The
  emulator and I/O threads go on 8 and 24. The guest topology is 1 socket × 7 cores × 2 threads.
- **CPU isolation:** none, only pinning. Other labs may still be scheduled on those threads.
- **Hugepages:** transparent hugepages, `madvise`, set host-wide. Reserved 1 GiB pages were
  rejected: they take 48 GiB from the host permanently and need a reboot. With `madvise`, QEMU
  already asks for 2 MiB pages for guest RAM, so no Sherpa setting is needed. Sherpa's installer
  should set `transparent_hugepage=madvise`.
- **No separate measuring step:** the tuning is built straight into the manifest.
- **Node images** are not copied while the server is built; they are copied in as needed.
