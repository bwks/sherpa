# MicroVM sandboxes for agent workloads

> Status: investigation for #347, nothing implemented. Hands-on measurements of four QEMU machine
> setups driven through libvirt, and a recommendation. Scope set by the user: no new hypervisor
> (QEMU through libvirt and the `virt` crate only; NVX/OpenVMM, Cloud Hypervisor and Firecracker
> are out), and the use case is sandbox VMs for agent workloads: isolated, fast to start, cheap,
> disposable, with a workspace in and results out. NIC count and network-OS compatibility do not
> matter.

## Summary

- QEMU's `microvm` machine type defines and boots through libvirt 10.0.0, with virtio-mmio or
  with PCIe. It is a second-class citizen there: it needs explicit XML for every device, a
  `qemu:commandline` passthrough to be fast (and for PCIe), it cannot hotplug, disk `iotune`
  fails on virtio-mmio, and a guest with exactly 2048 MiB does not boot.
- A stripped q35 with direct kernel boot needs no workaround at all, supports every libvirt
  feature tried, and cold-boots 1.3 s slower than microvm (5.1 s against 3.7 s here, nested).
- Both are far ahead of the way Sherpa defines VMs today (9.3 s for the same guest).
- About 1.07 s of every start is libvirt, before the vCPUs run: it runs `virt-aa-helper` and
  `apparmor_parser -r` four times. Starts are also serialized: eight in parallel still start at
  0.7 sandboxes a second.
- Restoring a saved memory image takes 1.4–1.7 s on every variant. On the stripped q35, restore
  followed by hot-plugging a scratch disk and a virtiofs workspace was usable in 2.4 s, faster
  than any cold boot. microvm cannot do this, because the guest never sees hot-plugged devices.
- **Recommendation:** build sandboxes on the stripped q35 (variant C) first, as a new, lighter
  resource with its own API rather than as lab nodes. Keep microvm with PCIe (variant B) as a
  later option if the smaller device model is wanted; do not use microvm with virtio-mmio
  (variant A).

## Method

### Where it ran

The dev guest (`tests/environments/dev`), which is itself a KVM guest of the host. Every sandbox
below is therefore a nested (L2) guest, so absolute times are higher than they would be on bare
metal. The comparison between variants is what matters.

| | Dev guest |
|---|---|
| CPU, memory | 12 vCPUs (Xeon E5-2620 v4, `host-passthrough`), 32 GiB, no swap |
| Kernel | 6.8.0-142-generic (Ubuntu 24.04) |
| QEMU | 8.2.2 (`1:8.2.2+ds-0ubuntu1.18`), the same as the host |
| libvirt | 10.0.0 (`10.0.0-2ubuntu8.19`), the same as the host |
| Nested KVM | works: `/dev/kvm` present, `kvm_intel nested=Y`, every variant ran with `type='kvm'` |
| Security driver | AppArmor (libvirt default on Ubuntu) |
| virtiofsd | not installed. Installed from the Ubuntu package (1.10.0) for the test, then removed |

Everything was built by hand with `virsh define`, `virsh create` and small scripts in a scratch
directory, `/var/tmp/microvm-347`, now removed. No Sherpa code was changed. `sherpad` had no labs
running.

### Guest

Alpine Linux 3.24.4 with the `linux-virt` kernel (6.18.63), built from the minirootfs tarball and
`apk` in a chroot. Alpine rather than an Ubuntu minimal cloud image because:

- it boots in under two seconds of guest time with OpenRC, so the differences between machine
  types are not buried under systemd and cloud-init;
- the base is small (93 MiB as a compressed qcow2);
- the `linux-virt` kernel has `VIRTIO_MMIO`, `VIRTIO_PCI` and ACPI built in, so one kernel and
  one initramfs boot all four variants;
- Claude Code runs on it: inside a variant A sandbox, `apk add nodejs npm libgcc libstdc++
  ripgrep git` and `npm install -g @anthropic-ai/claude-code` gave `claude --version` →
  `2.1.325 (Claude Code)`.

An Ubuntu minimal image was not measured. It would add glibc and cloud-init at the cost of a
larger image and a slower userspace boot; the machine-type results would not change.

The same disk image is used by all four variants: an MBR disk with one ext4 partition, GRUB for
the disk-boot baseline, and `/boot/vmlinuz-virt` and `/boot/initramfs-virt` copied out for direct
kernel boot. The guest's init does three things for the experiment: prints `SBX-READY` with the
kernel uptime on the console, starts a root shell on vsock port 1024 (`socat`), and runs
`user-data` from a disk labelled `cidata` if there is one.

### Variants

| | Machine | Boot | Devices |
|---|---|---|---|
| **A** | `microvm` | direct kernel, qboot (built in) | virtio-mmio: disk, vsock. ISA serial |
| **B** | `microvm`, `pcie=on` | direct kernel, qboot | virtio-pci behind PCIe root ports: disk, vsock. ISA serial |
| **C** | `q35`, stripped | direct kernel, SeaBIOS (also tried with qboot) | virtio-pci: disk, vsock. ISA serial. No USB, video, balloon |
| **D** | `q35`, as Sherpa's `libvirt_domain.jinja` | SeaBIOS, boot from disk (GRUB) | virtio disk, SATA CD-ROM, virtio NIC, xHCI, VNC and virtio-vga, guest agent channel, balloon, vsock |

All: 2 vCPUs, 512 MiB, `host-passthrough`, a qcow2 overlay on the shared base. Variant D has
vsock added so it can be measured the same way; Sherpa does not define one today.

### Measuring

"Usable" means a command sent to the guest's vsock shell is answered. The clock starts when
`virsh start` (or `virsh create`) is invoked and the vsock port is polled every 10 ms. Each
figure is five runs on a fresh overlay; tables give the median and the range.

## Results

### Headline numbers

| | A microvm, mmio | B microvm, PCIe | C stripped q35 | D q35 as today |
|---|---|---|---|---|
| Cold boot to usable | **3.74 s** (3.71–3.81) | 3.81 s (3.80–3.90) | 5.06 s (4.99–5.06) | 9.29 s (9.11–9.33) |
| ...as libvirt generates it, no passthrough | 6.46 s (6.34–6.61) | not run | same | same |
| Guest kernel uptime at ready | 1.85 s | 1.90 s | 2.99 s | 3.73 s |
| Restore from managed save to usable | 1.44 s (1.43–1.44) | 1.49 s (1.46–1.50) | 1.51 s (1.49–1.54) | 1.70 s (1.68–1.72) |
| Create overlay, start, usable, destroy, clean up | 4.20 s (4.11–4.25) | 4.28 s (4.21–4.29) | 5.50 s (5.43–5.69) | 9.64 s (9.63–10.09) |
| Idle host RSS of QEMU (512 MiB guest) | 174 MiB | 177 MiB | 184 MiB | 193 MiB |
| PCI devices the guest sees | 0 | 6 | 9 | 18 |
| XML workarounds | many | many | none | none |
| Disk `iotune` | **fails** | works | not run (standard path) | not run |
| Hotplug after start | rejected | accepted, guest never sees it | **works** | not run |
| virtiofs, 2000 small files create + sync | 5.4 s | about 2 s | 1.6 s | about 2 s |

C with qboot in place of SeaBIOS (`<loader type='rom'>/usr/share/qemu/qboot.rom</loader>`) was
4.99 s (4.93–5.02): no real gain, so SeaBIOS stays.

### 1. libvirt support

All four define and run.

What libvirt adds on its own, and whether the XML can remove it:

| Added by default | microvm (A, B) | q35 (C, D) | Removed with |
|---|---|---|---|
| USB controller | yes, and the define then fails: it is a PCI device | yes | `<controller type='usb' model='none'/>` |
| virtio memballoon | yes | yes | `<memballoon model='none'/>` |
| PS/2 mouse and keyboard `<input>` | listed in the XML, but QEMU creates no i8042 on microvm | real i8042, PS/2 keyboard and mouse | cannot be removed from the XML |
| PCIe root ports | B only: one per PCI device | one per PCI device | no; they are how devices attach |
| SATA (AHCI) controller, SMBus, LPC bridge | no | yes, built into q35 | no |
| Video, graphics | no | no, unless asked for | not asked for in A–C |
| `<audio type='none'/>` | yes | yes | not a guest device |

Workarounds needed, all on microvm:

1. **Every virtio device needs `<address type='virtio-mmio'/>` on A.** Without it the define
   fails with `XML error: No PCI buses available`. With it, addressing works: libvirt emits
   `virtio-blk-device`, `vhost-vsock-device`, `virtio-net-device` and the guest binds them from
   the ACPI table (`LNRO0005`).
2. **The default USB controller must be turned off** (same error otherwise).
3. **The balloon must be turned off** or given a virtio-mmio address.
4. **PCIe (B) needs a passthrough.** libvirt has no XML for `pcie=on`. It works with
   `<controller type='pci' index='0' model='pcie-root'/>` plus a `qemu:commandline` of
   `-machine pcie=on`; libvirt then assigns PCI addresses and adds root ports as on q35.
5. **Without an RTC the kernel stalls for 2.7 s.** libvirt starts microvm as
   `-machine microvm,usb=off,dump-guest-core=off,memory-backend=microvm.ram,acpi=on`, with no
   RTC, and the kernel's `rtc_cmos` driver waits on it (`rtc_cmos rtc_cmos: broken or not
   accessible`). Either `-machine rtc=on` by passthrough or `initcall_blacklist=cmos_init` on
   the kernel command line fixes it (6.46 s → 3.74 s). Every microvm number below uses
   `rtc=on`. A `qemu:commandline` taints the domain (`custom-argv`).
6. **Exactly 2048 MiB does not boot on microvm** (A and B, with and without libvirt). The kernel
   hangs after `ACPI: Core revision`. 512, 1024, 1536, 1800, 2000, 2047, 2049, 2100, 2560, 3072,
   4096 and 8192 MiB all boot; q35 boots with 2048. Cause not found.
7. **`iotune` fails on a virtio-mmio disk**, at start and live:
   `unable to execute QEMU command 'block_set_io_throttle': Device
   '/machine/peripheral/virtio-disk0/virtio-backend' not found`. libvirt assumes the PCI proxy
   device.
8. **No hotplug.** On A, `attach-interface` and `attach-device` fail with `No PCI buses
   available`. On B, `attach-device` reports success but the guest never sees the device, even
   after a PCI rescan.

Not specific to a variant:

- libvirt changes the owner of the overlay and of the shared backing file to `libvirt-qemu:kvm`
  while a domain uses them. The base image has to live where that is acceptable.
- AppArmor only lets libvirtd run `virtiofsd` from `/usr/{lib,libexec}`. A copy elsewhere fails
  with `cannot execute binary ...: Permission denied`.

XML that worked. Variant A:

```xml
<domain type='kvm' xmlns:qemu='http://libvirt.org/schemas/domain/qemu/1.0'>
  <name>sbx-a</name>
  <memory unit='MiB'>512</memory>
  <vcpu>2</vcpu>
  <os>
    <type arch='x86_64' machine='microvm'>hvm</type>
    <kernel>/path/vmlinuz-virt</kernel>
    <initrd>/path/initramfs-virt</initrd>
    <cmdline>root=/dev/vda1 rootfstype=ext4 modules=ext4 console=ttyS0,115200 quiet</cmdline>
  </os>
  <features><acpi/></features>
  <cpu mode='host-passthrough'/>
  <on_poweroff>destroy</on_poweroff>
  <on_reboot>destroy</on_reboot>
  <on_crash>destroy</on_crash>
  <devices>
    <emulator>/usr/bin/qemu-system-x86_64</emulator>
    <disk type='file' device='disk'>
      <driver name='qemu' type='qcow2'/>
      <source file='/path/overlay.qcow2'/>
      <target dev='vda' bus='virtio'/>
      <address type='virtio-mmio'/>
    </disk>
    <controller type='usb' model='none'/>
    <memballoon model='none'/>
    <serial type='pty'><target type='isa-serial' port='0'/></serial>
    <console type='pty'><target type='serial' port='0'/></console>
    <vsock model='virtio'>
      <cid auto='no' address='101'/>
      <address type='virtio-mmio'/>
    </vsock>
  </devices>
  <qemu:commandline>
    <qemu:arg value='-machine'/>
    <qemu:arg value='rtc=on'/>
  </qemu:commandline>
</domain>
```

Variant B is the same without the `<address type='virtio-mmio'/>` lines, with
`<controller type='pci' index='0' model='pcie-root'/>` added and the passthrough value
`pcie=on,rtc=on`.

Variant C, with no passthrough and no explicit addresses:

```xml
<domain type='kvm'>
  <name>sbx-c</name>
  <memory unit='MiB'>512</memory>
  <vcpu>2</vcpu>
  <os>
    <type arch='x86_64' machine='q35'>hvm</type>
    <kernel>/path/vmlinuz-virt</kernel>
    <initrd>/path/initramfs-virt</initrd>
    <cmdline>root=/dev/vda1 rootfstype=ext4 modules=ext4 console=ttyS0,115200 quiet</cmdline>
  </os>
  <features><acpi/></features>
  <cpu mode='host-passthrough'/>
  <on_poweroff>destroy</on_poweroff>
  <on_reboot>destroy</on_reboot>
  <on_crash>destroy</on_crash>
  <devices>
    <emulator>/usr/bin/qemu-system-x86_64</emulator>
    <disk type='file' device='disk'>
      <driver name='qemu' type='qcow2'/>
      <source file='/path/overlay.qcow2'/>
      <target dev='vda' bus='virtio'/>
    </disk>
    <controller type='usb' model='none'/>
    <memballoon model='none'/>
    <serial type='pty'><target type='isa-serial' port='0'/></serial>
    <console type='pty'><target type='serial' port='0'/></console>
    <vsock model='virtio'><cid auto='no' address='103'/></vsock>
  </devices>
</domain>
```

Optional devices, the same on every variant apart from the mmio address on A:

```xml
<!-- virtiofs workspace; needs shared memory and the virtiofsd package -->
<memoryBacking><source type='memfd'/><access mode='shared'/></memoryBacking>
<filesystem type='mount' accessmode='passthrough'>
  <driver type='virtiofs'/>
  <source dir='/path/workspace'/>
  <target dir='workspace'/>
</filesystem>

<!-- scratch disk, and a read-only seed disk labelled cidata -->
<disk type='file' device='disk'>
  <driver name='qemu' type='raw'/>
  <source file='/path/scratch.raw'/>
  <target dev='vdb' bus='virtio'/>
</disk>

<!-- one NIC, with a cap -->
<interface type='network'>
  <source network='sandbox-net'/>
  <model type='virtio'/>
  <bandwidth>
    <inbound average='1000' peak='1000' burst='128'/>
    <outbound average='1000' peak='1000' burst='128'/>
  </bandwidth>
</interface>

<!-- limits -->
<memtune><hard_limit unit='MiB'>768</hard_limit></memtune>
<cputune><period>100000</period><quota>50000</quota></cputune>
```

### 2. Attack surface

What the guest sees (`lspci`, `/sys/bus/virtio`, `/sys/bus/pnp`, `/proc/interrupts`,
`/proc/ioports`, `dmesg`), and what QEMU instantiates (`info qtree`):

| | A | B | C | D |
|---|---|---|---|---|
| PCI devices | 0 | 6: host bridge, 3 root ports, disk, vsock | 9: host bridge, 3 root ports, LPC, AHCI, SMBus, disk, vsock | 18: as C with 7 root ports, plus virtio-vga, xHCI, NIC, virtio-serial, balloon |
| virtio devices | 2 (blk, vsock) | 2 | 2 | 6 (gpu, net, console, blk, balloon, vsock) |
| PnP devices | serial | serial | serial, PS/2 keyboard, PS/2 mouse, system board | as C |
| Interrupt lines in use | 5 | 11 | 15 | 27 |
| I/O port ranges | 15 | 15 | 29 | 33 |
| `dmesg` lines at boot | 279 | 381 | 430 | 524 |
| QEMU device objects | 38, of which 24 are empty virtio-mmio slots | 45 | 39 | 53 |

Legacy and chipset emulation present in QEMU:

| | A | B | C | D |
|---|---|---|---|---|
| PIC, PIT, IOAPIC, RTC, serial, `fw_cfg` | yes | yes | yes | yes |
| PCI host bridge and config space | no | yes (generic PCIe host) | yes (q35 MCH) | yes |
| ICH9 LPC, AHCI, SMBus with 8 EEPROMs | no | no | yes | yes |
| i8042, PS/2 keyboard and mouse, `vmport`, `vmmouse` | no | no | yes | yes |
| HPET, PC speaker, port 92, i8257 DMA | no | no | yes | yes, HPET off |
| VGA, xHCI, virtio-serial, balloon, IDE CD-ROM | no | no | no | yes |

A is the smallest by a clear margin. B gives back the PCI host bridge and root ports. C keeps
the q35 chipset, which cannot be removed: it is the price of the standard path. Further
stripping of q35 (`vmport`, HPET, PS/2 off through `<features>`) was not tried.

### 3. Start time

Cold boot and restore are in the headline table. Where a cold boot of A goes (3.74 s):

| Phase | Time | Evidence |
|---|---|---|
| libvirt, before the vCPUs start | 1.07 s | `virsh start --paused` returns in 1.07–1.08 s |
| QEMU start, qboot, kernel load | about 0.8 s | the rest of the gap between host time and guest uptime |
| Kernel to init | about 0.5 s | `dmesg` |
| initramfs, root mount, OpenRC to the marker | about 1.35 s | marker at uptime 1.85 s |

- The libvirt second is the AppArmor driver. An `strace` of libvirtd during one start shows
  `virt-aa-helper -r` followed by `apparmor_parser -r` four times before QEMU is executed.
  Compiling the profile alone takes 0.07 s; loading it four times is the cost.
- The floor without libvirt: the same microvm started directly by QEMU is usable in 2.72 s
  (2.70–2.73).
- q35 spends about 1.1 s more in the kernel than microvm: PCI enumeration and the AHCI probe.
- D spends about 3.5 s more than C before the kernel: SeaBIOS with option ROMs, the CD-ROM boot attempt,
  GRUB.

Restore:

- `virsh managedsave` takes 0.75 s and writes 100–120 MiB for an idle 512 MiB guest.
- Restore is 1.44–1.70 s on every variant, of which 1.07 s is again libvirt.
- A saved image belongs to one domain. `virsh restore --xml` accepts a changed disk path
  (usable in 1.57 s) but refuses a changed name or UUID (`Target domain uuid ... does not match
  source`). One image cannot be fanned out into several running sandboxes through libvirt.
- **A domain with a virtiofs device cannot be saved**, mounted or not: `cannot migrate domain:
  Migration disabled: vhost-user backend lacks VHOST_USER_PROTOCOL_F_LOG_SHMFD feature`.
  Shared `memfd` memory without virtiofs saves and restores normally.
- On C, the way round that works: restore a guest saved without a workspace, then hot-plug the
  scratch disk and the virtiofs workspace. Restore, both attaches and both mounts took **2.39 s**
  in one run. This needs spare `pcie-root-port` controllers in the XML (`No more available PCI
  slots` otherwise). On B the attach is accepted and the guest never sees the device; on A it
  is refused.

### 4. Disposability

A qcow2 overlay per sandbox on a shared base works on every variant. The overlay is 0.2 MiB when
created and `qemu-img create` takes 0.03 s.

Lifecycle with a transient domain (`virsh create`, no define or undefine), variant A:

| Step | Time |
|---|---|
| Create the overlay and write the XML | 0.06 s |
| `virsh create` to usable | 3.80 s |
| `virsh destroy` and delete the overlay | 0.34 s |
| Total | 4.20 s (4.11–4.25) |

Idle memory, host RSS of the QEMU process after 30 s: 174 MiB (A), 177 (B), 184 (C), 185 (C
with qboot), 193 (D). PSS is about 22 MiB lower. The floor does not follow the guest's size: a
256 MiB guest was 171 MiB, and `init_on_alloc=0` changed nothing (175 MiB). The guest itself
reports 34 MiB used. Why the floor is that high was not found.

Density, variant A, 512 MiB each, started eight at a time:

| Running | Batch | Start rate | Each, create to usable | RSS each | Total RSS | Host CPU busy, idle | One more sandbox |
|---|---|---|---|---|---|---|---|
| 10 | 14.2 s | 0.7 /s | 9.5 s | 175 MiB | 1.7 GiB | 2.7 % | 3.93 s |
| 20 | +10 in 14.6 s | 0.7 /s | 9.8 s | 175 MiB | 3.5 GiB | 4.9 % | 4.21 s |
| 40 | +20 in 27.0 s | 0.7 /s | 10.3 s | 175 MiB | 7.0 GiB | 6.9 % | 4.35 s |

- The run stopped at 40 on purpose, with 23.7 GiB still available. `MemAvailable` fell by about
  150 MiB per idle sandbox.
- What hurts first is the **start rate**, not memory or CPU: libvirt starts domains one after
  another, so eight parallel requests each waited 9.5 s for a 3.8 s boot. Sixty sandboxes need a
  minute and a half to come up.
- Idle sandboxes are cheap: 40 used 7 % of 12 CPUs, and one more still started in 4.35 s.
- Idle memory is not the real limit either. An agent uses its allocation: at 2 GiB each, 32 GiB
  holds about 12 working sandboxes whatever the idle figure says. Memory admission (#514) has to
  count the allocation, not the RSS.
- Destroying all 40 and deleting their overlays took 5.4 s.

### 5. Work in and out

| | A | B | C | D |
|---|---|---|---|---|
| virtiofs mounts (shared `memfd` backing) | yes | yes | yes | yes |
| virtio-blk scratch disk | yes | yes | yes | yes |
| Seed disk labelled `cidata`, as a virtio disk | yes | yes | yes | not read (SATA CD-ROM; this image loads no CD driver) |
| Task on the kernel command line | yes | yes | yes | n/a, boots through GRUB |
| vsock | yes | yes | yes | yes |

Throughput inside the guest:

| | virtiofs workspace | virtio-blk scratch (raw, ext4) |
|---|---|---|
| Write 256 MiB and fsync | 78–83 MB/s | 540–606 MB/s |
| Read it back, cold cache | 0.47–1.0 GB/s | 1.7–2.2 GB/s |
| Create 2000 small files and sync | A 5.4–5.5 s, C 1.5–1.7 s | 0.14–0.26 s |
| Read and delete them | A 4.9–5.1 s, C 1.0–1.1 s | 0.06–0.07 s |
| 512 MiB guest to host over vsock (`socat` to Python) | 3.3–3.9 s on every variant, about 150 MiB/s | |

- virtiofs works on microvm, but it is the slow path everywhere: 7 times slower for bulk writes
  and 10 to 35 times slower for small files, which is what a checkout or a build is. On
  virtio-mmio it is three times slower again than on PCI.
- virtiofs writes land on the host as **root**: libvirt runs `virtiofsd` as root with no ID
  mapping, so a file the guest's root creates in the workspace is `root:root` on the host,
  set-uid bit included if it sets one.
- virtiofs blocks save and restore (section 3) and needs the `virtiofsd` package on the host.
- A scratch disk has none of those problems. Getting results off it needs care: the host must
  not mount a filesystem an untrusted guest wrote. Stream the results out over vsock instead.

Getting a task and credentials in:

| | Kernel command line | Seed disk (NoCloud style) | vsock |
|---|---|---|---|
| Ease | trivial: one XML element, read `/proc/cmdline` | easy: Sherpa's `iso` crate already writes these; guest needs `cloud-init`, `tiny-cloud` or a ten-line script | needs an agent in the guest and a protocol |
| Size | about 2 KiB | any | any, and can change after boot |
| Who can read it on the host | anyone who can read the domain XML, libvirt's per-domain log, or `ps` (it is on QEMU's command line) | whoever can read the file; it stays on disk until deleted | nobody: nothing is written down |
| Who can read it in the guest | every process (`/proc/cmdline`) | root, for the life of the VM (no hot-unplug on microvm) | only the agent that receives it |
| Verdict | task parameters only, never credentials | fine for the task and static configuration; weak for credentials | the right place for credentials: push short-lived ones after boot, keep them in `tmpfs` |

### 6. Host to guest control channel

`<vsock model='virtio'>` works on all four variants: `vhost-vsock-device` on A,
`vhost-vsock-pci` on the others. The guest needs `vmw_vsock_virtio_transport`; the host loads
`vhost_vsock` on first use. Every measurement in this document went over it.

- The CID is host-wide and must be unique among running guests. libvirt can assign it
  (`auto='yes'`); the tests assigned it by hand.
- A paused guest does not answer; it answers again after resume and after restore, with the same
  CID.
- `AF_VSOCK` has no permissions: any process on the host can connect to a guest's port, and any
  process in the guest can connect to the host's. Both ends have to treat the other as
  untrusted.
- `qemu-guest-agent` over a virtio-serial channel was not tried. It would give exec and file
  transfer without writing an agent.

### 7. Network

Tested on A and B unless noted.

| | Result |
|---|---|
| No NIC | the default in A–C; the guest has `lo` only |
| One NIC on an isolated libvirt network | DHCP lease, gateway answers, no route out (`Network unreachable`) |
| One NIC on a NAT libvirt network | DHCP lease, reaches the internet |
| Deny egress for one sandbox | an nftables `forward` rule dropping that sandbox's MAC: its ping lost 100 %, its neighbour on the same network 0 %; removing the rule restored it |
| Two sandboxes on one isolated bridge | they reach each other. Per-sandbox isolation needs port isolation or a bridge each; `<port isolated='yes'/>` was not tried |
| Plain Linux bridge, `<interface type='bridge'>`, as Sherpa's private bridges | works, both taps forwarding |
| Tap with Sherpa's P2p eBPF redirect | works: `network::attach_p2p_redirect` on two `<interface type='ethernet'>` taps; `p2p_redirect` attached, ping 0 % loss, 1472-byte payload passed, 64 MiB of TCP in 0.29 s |
| `domif-setlink` | works on A and B, bridge and tap: guest carrier goes 0 and back to 1, and traffic on the eBPF link stops and resumes |
| NIC hotplug | fails on A (`No PCI buses available`) |

`br-sherpa0` did not exist on the dev guest during the test (no labs), so a bridge was made the
same way instead. Nothing here depends on the machine type: the tap, the bridge and the eBPF
program are all on the host side of virtio-net.

### 8. Resource limits

| Limit | A | B |
|---|---|---|
| vCPU count, memory size | apply; the guest sees 2 CPUs and 479 MiB | same |
| CPU quota (`<cputune>` 50 % per vCPU) | applies: two busy vCPUs used 5.08 CPU-seconds in 5.2 s, half of uncapped | 5.07 |
| Memory hard limit (`<memtune>`) | applies: the scope's `memory.max` is 805306368 | same |
| Disk `iotune` in the XML | **domain fails to start** | applies: 10 MiB/s cap measured at 10.3 MB/s |
| Disk `blkdeviotune --live` | **fails**, same error | applies: 5 MiB/s cap measured at 5.2 MB/s |
| NIC `<bandwidth>`, and `domiftune --live` | applies (host `tc` on the tap): 1000 KiB/s cap measured at 960 KiB/s, against 41 MiB/s uncapped | not run |
| `setvcpus --live`, `setmem --live` | refused: no hotpluggable vCPUs, no balloon | same |

CPU, memory and network limits are enforced on the host and do not care about the machine type.
Disk throttling is the one that breaks, on virtio-mmio only.

### 9. Console and lifecycle

A, B and C behaved the same:

| | Result |
|---|---|
| `virsh console` | login prompt and a command, through the ISA serial port as a pty |
| Suspend, resume | paused, no answer on vsock; running and answering after resume |
| Managed save, restore | works (section 3), not with virtiofs |
| `virsh shutdown` | ignored by this guest until it loads `button` and `evdev` and runs `acpid`; then it shuts down in 3 s. A guest image matter, the same on q35 |
| `poweroff` in the guest | domain is shut off in 3 s |
| `reboot` in the guest | domain is destroyed, as `on_reboot` says |
| `virsh destroy` | 0.33 s |
| `virsh reset` | accepted; the domain then ends, because a reset counts as a reboot and `on_reboot` is destroy |
| `systemctl restart libvirtd` | persistent and transient domains kept running with their uptime; vsock, `virsh console` and `domif-setlink` worked afterwards |

Restarting libvirtd also restarted `sherpad`, because `sherpad.service` has
`Requires=libvirtd.service`. It came back within a second.

### 10. Guest image

| | Size |
|---|---|
| Base image, compressed qcow2 | 93 MiB (110 MiB used in a 1 GiB filesystem) |
| Kernel, `vmlinuz-virt` 6.18.63 | 12.7 MiB |
| Initramfs, mkinitfs features `base ext4 virtio` | 6.7 MiB |
| With Node.js, npm, git, ripgrep and Claude Code | 703 MiB used; the overlay grew to 489 MiB |

How the kernel and initramfs are produced: `apk add linux-virt mkinitfs` in the image runs
`mkinitfs`, which writes `/boot/vmlinuz-virt` and `/boot/initramfs-virt`; the build copies both
out next to the image. They are a pair with the modules in the image: the initramfs loads
`virtio_blk` and `ext4`, and the root filesystem loads `vmw_vsock_virtio_transport` and
`virtiofs` from `/lib/modules`. Updating means rebuilding the image and replacing all three
together, never one alone. A sandbox image is therefore a triple (disk, kernel, initramfs) plus
a kernel command line, which is what Sherpa's unikernel direct-kernel boot already carries.

Two build traps met here, worth keeping:

- Convert the raw image to qcow2 only after `sync` and a flush of the loop device. GRUB's writes
  to the loop device were still in the buffer cache, and the first qcow2 did not boot.
- Do not `mount --rbind /dev` into the chroot on a host with systemd services that have private
  mount namespaces. The mounts propagated into six of them and pinned the loop devices.

## Which variant

| | For | Against |
|---|---|---|
| **A** microvm, virtio-mmio | fastest, smallest device model | passthrough needed to be fast; no `iotune`; no hotplug; virtiofs three times slower; 2048 MiB bug; an XML address on every device |
| **B** microvm, PCIe | as fast as A; `iotune` and virtiofs as on q35 | two passthrough options; no hotplug; 2048 MiB bug; a combination libvirt does not model |
| **C** stripped q35 | no workaround; everything works; restore plus hotplug is the fastest path measured | 1.3 s slower cold boot; the q35 chipset stays |
| **D** as today | nothing to build | 9.3 s; 18 PCI devices |

Recommend **C**. The 1.3 s is real, but it is smaller than the second libvirt takes on every
start, and C is the only variant where the faster design (a warm or saved guest that gets its
workspace hot-plugged) is possible. It also needs nothing libvirt does not model, which matters
for a control plane whose first goal is reliability. B is the fallback if a smaller device model
is a requirement; A has no advantage over B that is worth its failures.

## Design: lab node, or a new resource

**A new, lighter resource with its own API.** Reasons:

- **Different lifecycle.** A lab is declared in a manifest, brought up once and kept for hours or
  weeks, with `down`, `resume` and `redeploy`. A sandbox is created by a call, lives for one
  task, returns results and is gone. Nothing in it is worth a manifest.
- **The lab pipeline would dominate.** A node gets a management address, DNS, SSH keys and
  config, ZTP media, and readiness is decided over SSH. A sandbox needs none of that, and its
  whole start budget is a few seconds.
- **Different trust.** Lab nodes are the user's own network devices on shared bridges. A sandbox
  runs code an agent wrote: no NIC by default, limits always set, nothing shared with a lab
  unless asked for.
- **Different caller.** Labs are driven by a person with a file. Sandboxes are driven by
  programs, many at a time, which need create, exec, copy and destroy as API operations.
- **State is simpler.** A lab node's states and recovery rules against "starting, running,
  finished, gone" with a time limit.

What each would need:

| | As a lab node | As a sandbox resource |
|---|---|---|
| Server | a fourth `NodeKind`; a node model with kernel, initramfs and command line (the unikernel kind already has direct kernel boot); a readiness check that is not SSH; a way to skip ZTP, DNS and SSH config; exec and file copy bolted on to nodes | a `sandbox` service: create, exec, copy in and out, status, destroy; an Askama domain template (variant C); an overlay per sandbox in the storage pool; vsock CID allocation; a network policy; limits; a sweeper for expired sandboxes; reconciliation at startup (domains without a row are destroyed, rows without a domain are marked gone) |
| CLI | a manifest per sandbox, then `sherpa up` and `sherpa destroy` | `sherpa sandbox run`, `create`, `exec`, `cp`, `list`, `destroy` |
| Database | nothing new, but every sandbox is a lab row and a node row | a `sandbox` table (owner, image, state, CID, sizes, network policy, created, expires, exit status) |
| Images | a node model per sandbox image | the image store learns the triple: disk, kernel, initramfs |
| Fits the use case | poorly | yes |

Both share the `libvirt` crate, the storage pool, the image store, authentication and the
`network` crate. A sandbox that has to reach a lab can be given a NIC on a lab bridge later:
section 7 shows nothing stops it.

### Resource model

- **Sandbox image:** a disk, a kernel, an initramfs and a command line, versioned together, with
  the guest agent inside.
- **Sandbox:** image, vCPUs, memory, scratch size, time limit, network policy (`none`, or
  `egress` with a limit), owner. Always: a qcow2 overlay on the read-only base, vsock, a serial
  console, `on_reboot` and `on_crash` set to destroy, CPU and memory limits, no balloon, no USB,
  no video.
- **Workspace in:** a scratch virtio disk, filled over vsock. virtiofs only as an opt-in for
  live sharing with a trusted guest, given its speed, the root-owned writes and the lost
  save/restore.
- **Task in:** over vsock (exec). The command line for static parameters only.
- **Credentials in:** over vsock after boot, short-lived, never on the command line or a seed
  disk.
- **Results out:** a stream over vsock. The host never mounts the guest's disk.
- **End:** destroy the domain, delete the overlay and the scratch disk.

## PR plan

1. **Sandbox image.** A documented, repeatable build for the Alpine image, kernel and initramfs
   (with `acpid`, the vsock module and the agent), and image-store support for the triple.
   Shippable alone: the image can be imported and listed.
2. **Sandbox resource, minimal.** The `sandbox` table, the domain template (variant C, no NIC,
   CPU and memory limits, vsock), create, list and destroy in the server, API and CLI, the
   time-limit sweeper and startup reconciliation. Shippable: a sandbox can be started, reached on
   `virsh console`, and thrown away.
3. **Exec and data.** The vsock agent protocol: run a command with streamed output and an exit
   status, copy files in and out, push credentials. `sherpa sandbox run`, `exec` and `cp`.
   Shippable: the use case works end to end.
4. **Network policy and limits.** `none` and `egress`, with per-sandbox nftables rules and port
   isolation; disk `iotune` and NIC `bandwidth`; admission against host memory, counting the
   allocation.
5. **Fast start.** A warm pool or saved guests with the workspace hot-plugged (section 3), and a
   look at the AppArmor second and the serialized start. Only if the measured start time is not
   good enough.

User documentation (`../sherpa-docs`) changes with PRs 2 to 4, and `docs/architecture/` gets a
sandbox document with PR 2.

## Open questions

- Is the stripped q35 acceptable, or is the smaller device model of microvm a requirement worth
  the passthrough and the lost features?
- What start time is the target? A cold boot is 4–5 s here, nested. If that is enough, PR 5 can
  wait.
- Who may start sandboxes, and is the code in them treated as hostile? That decides whether
  virtiofs is offered at all.
- The guest agent: write a small one, or use `qemu-guest-agent` (not tried)?
- Alpine (musl, 93 MiB) or Ubuntu minimal (glibc, cloud-init, not measured) as the image, and
  where images are built and published.
- Does a sandbox ever need to join a lab network?
- How fine must egress control be? Per sandbox is easy; per destination needs a proxy.
- The AppArmor cost: 1.07 s a start and 0.7 starts a second. Whether it can be reduced without
  turning the security driver off was not explored.

## Not done

- Bare-metal timings: everything ran nested.
- An Ubuntu guest; ARM.
- `iotune` and hotplug on D, `bandwidth` on B; `<port isolated='yes'/>`; `qemu-guest-agent`;
  further stripping of q35.
- The cause of the 2048 MiB hang and of the 175 MiB idle floor.
