# Juniper vSRX 3.0 - KVM/QEMU Deployment Notes

## Overview

The vSRX 3.0 is a virtual security appliance built on FreeBSD. It runs nested KVM internally
(hypervisor-within-a-hypervisor). This architecture makes it sensitive to specific QEMU/KVM
host settings that don't affect most other virtual machines.

## Host Requirements

### CPU

- Intel x86_64 with VT-x/VT-d enabled
- Minimum: 2 vCPUs / 4 GB RAM
- CPU model: `SandyBridge` (recommended) or `host`
- `vmx` must be enabled (nested virtualization)

### PML (Page Modification Logging)

PML **must be disabled** on Intel Xeon E5/E7 v4 hosts. PML is a VT-x feature that tracks
dirty memory pages in hardware. It interferes with vSRX's nested KVM, causing the FreeBSD
bootloader to hang during early boot (typically at `netstack/init.4th`).

**Check if PML is enabled:**

```bash
cat /sys/module/kvm_intel/parameters/pml
```

**Disable PML:**

```bash
# 1. Stop all running VMs

# 2. Add modprobe config
echo "options kvm_intel pml=0" | sudo tee /etc/modprobe.d/kvm-vsrx.conf

# 3. Reload the kvm_intel module
sudo modprobe -r kvm_intel && sudo modprobe kvm_intel

# 4. Verify PML is disabled (should show "N")
cat /sys/module/kvm_intel/parameters/pml
```

If `modprobe -r` fails (other VMs still running), a reboot is required.

**Impact of disabling PML:** Live migration will be slower due to software-based dirty page
tracking. General VM performance impact is minimal for most workloads.

### SMBIOS Entry Point (QEMU 8.1+)

Starting with QEMU 8.1, the default SMBIOS entry point changed from 32-bit to 64-bit.
vSRX (FreeBSD-based) expects 32-bit SMBIOS and will crash with a `Fatal trap 12: page fault`
during early boot if 64-bit is used.

**Fix:** Add `-machine smbios-entry-point-type=32` to QEMU arguments.

Sherpa handles this automatically via `QemuCommand::juniper_vsrxv3()`.

## CPU Configuration

### CPU Model

Use `SandyBridge` as the CPU model. This is the model recommended by Juniper and used by
vrnetlab.

### CPU Features

Several CPU features must be **disabled** for vSRX to boot on KVM. These features cause the
FreeBSD bootloader to hang or crash during early initialization.

| Feature    | Reason                                              |
|------------|-----------------------------------------------------|
| `xsaveopt` | Causes FreeBSD bootloader hang on KVM               |
| `bmi1`     | Incompatible with vSRX FreeBSD kernel                |
| `avx2`     | Incompatible with vSRX FreeBSD kernel                |
| `bmi2`     | Incompatible with vSRX FreeBSD kernel                |
| `erms`     | Incompatible with vSRX FreeBSD kernel                |
| `invpcid`  | Incompatible with vSRX FreeBSD kernel                |
| `rdseed`   | Incompatible with vSRX FreeBSD kernel                |
| `adx`      | Incompatible with vSRX FreeBSD kernel                |
| `smap`     | Incompatible with vSRX FreeBSD kernel                |
| `abm`      | Incompatible with vSRX FreeBSD kernel                |

The `vmx` feature must be **enabled** (required for nested virtualization).

Sherpa handles this via `cpu_features_for_model()` in `node_ops.rs`.

## QEMU/Libvirt Settings

| Setting         | Value              | Notes                                         |
|-----------------|--------------------|-----------------------------------------------|
| Machine type    | `pc-i440fx-8.0`    | Avoids SMBIOS 64-bit default in newer types   |
| CPU model       | `SandyBridge`      | Matches Juniper recommendation and vrnetlab   |
| SMBIOS          | `sysinfo` mode     | Used with `smbios-entry-point-type=32`        |
| HDD bus         | `virtio`           | Primary disk                                  |
| CDROM bus       | `ide`              | Config/bootstrap ISO                          |
| NIC type        | `virtio`           | All interfaces                                |
| Memory          | 4096 MB            | Minimum for vSRX                              |
| vCPUs           | 2                  | Minimum for vSRX                              |

## Boot Sequence

The vSRX boot process:

1. SeaBIOS initializes hardware
2. FreeBSD bootloader loads (`loader.rc`, `support.4th`, `platform.4th`)
3. `netstack/init.4th` initializes the network stack packages
4. FreeBSD kernel loads and starts Junos

If boot hangs at step 3 (`netstack/init.4th` with spinning backslash), check:
1. PML is disabled (most common cause on Xeon E5/E7 v4)
2. CPU features are correctly disabled (see table above)
3. SMBIOS entry point is set to 32-bit

## Troubleshooting

### Hang at `netstack/init.4th`

**Symptom:** Boot output shows verified files, then hangs with a spinning `\` character.

**Most likely cause:** PML enabled on Intel Xeon E5/E7 v4 host. See PML section above.

**Other causes:** Missing CPU feature disables, or incorrect SMBIOS entry point type.

### Fatal trap 12: page fault

**Symptom:** Kernel panic with `Fatal trap 12: page fault while in kernel mode` during
`mtx_platform_early_bootinit`.

**Cause:** SMBIOS 64-bit entry point (QEMU 8.1+ default). Fix by adding
`-machine smbios-entry-point-type=32` or using machine type `pc-i440fx-8.0`.
  
### 100% CPU usage

**Symptom:** vSRX VM consumes 100% of allocated vCPUs.

**Note:** This is normal for vSRX. The data plane runs in a polling loop. Use
`virsh schedinfo` with `vcpu_quota` to throttle if needed.

### No console output

**Symptom:** VM starts but serial console shows nothing.

**Check:** Verify the serial console is configured as `isa-serial` on the correct
loopback address and port. Ensure the VM hasn't crashed silently (check `virsh domstate`
and host `dmesg` for KVM errors).

## References

- [Juniper vSRX KVM System Requirements](https://www.juniper.net/documentation/us/en/software/vsrx/vsrx-consolidated-deployment-guide/vsrx-kvm/topics/concept/security-vsrx-system-requirement-with-kvm.html)
- [Install vSRX with KVM](https://www.juniper.net/documentation/us/en/software/vsrx/vsrx-consolidated-deployment-guide/vsrx-kvm/topics/task/security-vsrx-with-kvm-installing.html)
- [vrnetlab vSRX implementation](https://github.com/srl-labs/vrnetlab/tree/main/juniper/vsrx)
- [Proxmox FreeBSD SMBIOS issue](https://forum.proxmox.com/threads/fatal-trap-12-after-upgrade-to-8-1-3.138087/)
- [Juniper Community - vSRX boot stuck](https://community.juniper.net/discussion/vsrx-stops-booting-at-a-stage-in-proxmox-ve)
