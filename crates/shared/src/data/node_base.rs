//! The shapes the built-in node models are built from.
//!
//! Each model's constructor in `node` starts from one of these bases and states only what differs
//! from it, with struct update syntax. A virtual machine's base comes in two parts: the image,
//! and its `vm` group, which a constructor that changes the group builds on and passes to
//! [`image`]. No model is built from another model, so changing a base changes only the models
//! built from it.

use super::cpu::CpuModels;
use super::disk::DiskBuses;
use super::interface::MgmtInterfaces;
use super::node::{
    BiosTypes, BootDiskSettings, CdromSettings, ContainerSettings, CpuArchitecture, CpuSettings,
    InterfaceDeviceSettings, InterfaceSettings, InterfaceType, KindSettings, MachineSettings,
    MachineType, NodeConfig, NodeModel, OsVariant, UnikernelBootMode, UnikernelCpuSettings,
    UnikernelMachineSettings, UnikernelSettings, VmCpuSettings, VmSettings, Ztp, ZtpSettings,
};
use crate::konst::{MTU_JUMBO_INT, MTU_STD};

/// An image of `model` with one vCPU, `memory` MiB, `interfaces`, and its kind's `settings`.
#[must_use]
pub(super) fn image(
    model: NodeModel,
    memory: u16,
    interfaces: InterfaceSettings,
    settings: KindSettings,
) -> NodeConfig {
    NodeConfig {
        id: None,
        model,
        version: "0.0.0".to_owned(),
        default: true,
        memory,
        cpu: CpuSettings { count: 1 },
        interfaces,
        settings,
    }
}

/// A Linux VM provisioned by cloud-init: one vCPU, 1 GiB, and one data interface after `eth0`,
/// with the `vm` group of [`cloud_init_vm_settings`].
#[must_use]
pub(super) fn cloud_init_vm(model: NodeModel) -> NodeConfig {
    image(
        model,
        1024,
        InterfaceSettings {
            data_count: 1,
            management: MgmtInterfaces::Eth0,
            reserved_count: 0,
        },
        KindSettings::VirtualMachine(cloud_init_vm_settings()),
    )
}

/// The `vm` group of a Linux VM provisioned by cloud-init: a q35 machine with `SeaBIOS`, a
/// virtio boot disk, and jumbo-MTU virtio interfaces.
#[must_use]
pub(super) fn cloud_init_vm_settings() -> VmSettings {
    VmSettings {
        os_variant: OsVariant::Linux,
        cpu: VmCpuSettings {
            architecture: CpuArchitecture::X86_64,
            model: CpuModels::HostModel,
            vmx: false,
        },
        machine: MachineSettings {
            kind: MachineType::Q35,
            bios: BiosTypes::SeaBios,
        },
        boot_disk: BootDiskSettings {
            bus: DiskBuses::Virtio,
        },
        cdrom: CdromSettings {
            image: None,
            bus: DiskBuses::Sata,
        },
        interfaces: InterfaceDeviceSettings {
            kind: InterfaceType::Virtio,
            mtu: MTU_JUMBO_INT,
        },
        ztp: ZtpSettings {
            method: Ztp::CloudInit,
        },
    }
}

/// The `vm` group of a network operating system VM, provisioned from a config CD-ROM: a pc
/// machine with `SeaBIOS` and a virtio boot disk. Every such model has its own OS and interface
/// device, so they are arguments rather than defaults.
#[must_use]
pub(super) fn network_os_vm_settings(
    os_variant: OsVariant,
    interfaces: InterfaceDeviceSettings,
) -> VmSettings {
    VmSettings {
        os_variant,
        cpu: VmCpuSettings {
            architecture: CpuArchitecture::X86_64,
            model: CpuModels::HostModel,
            vmx: false,
        },
        machine: MachineSettings {
            kind: MachineType::Pc,
            bios: BiosTypes::SeaBios,
        },
        boot_disk: BootDiskSettings {
            bus: DiskBuses::Virtio,
        },
        cdrom: CdromSettings {
            image: None,
            bus: DiskBuses::Sata,
        },
        interfaces,
        ztp: ZtpSettings { method: Ztp::Cdrom },
    }
}

/// An application container from `repo`: one vCPU, 1 GiB, no provisioning, and one data
/// interface after `eth0`.
#[must_use]
pub(super) fn app_container(model: NodeModel, repo: Option<&str>) -> NodeConfig {
    image(
        model,
        1024,
        InterfaceSettings {
            data_count: 1,
            management: MgmtInterfaces::Eth0,
            reserved_count: 0,
        },
        KindSettings::Container(ContainerSettings {
            repo: repo.map(str::to_owned),
            ztp: ZtpSettings {
                method: Ztp::Disabled,
            },
        }),
    )
}

/// A network operating system container from `repo`, provisioned from a config volume: one
/// vCPU. Every such model has its own memory and interface layout, so they are arguments rather
/// than defaults.
#[must_use]
pub(super) fn network_os_container(
    model: NodeModel,
    memory: u16,
    repo: &str,
    interfaces: InterfaceSettings,
) -> NodeConfig {
    image(
        model,
        memory,
        interfaces,
        KindSettings::Container(ContainerSettings {
            repo: Some(repo.to_owned()),
            ztp: ZtpSettings {
                method: Ztp::Volume,
            },
        }),
    )
}

/// A unikernel that boots by `boot_mode`: one vCPU, 512 MiB, no data interfaces, a pc machine
/// and virtio interfaces.
#[must_use]
pub(super) fn unikernel(model: NodeModel, boot_mode: UnikernelBootMode) -> NodeConfig {
    image(
        model,
        512,
        InterfaceSettings {
            data_count: 0,
            management: MgmtInterfaces::Eth0,
            reserved_count: 0,
        },
        KindSettings::Unikernel(UnikernelSettings {
            boot_mode,
            cpu: UnikernelCpuSettings {
                architecture: CpuArchitecture::X86_64,
                model: CpuModels::HostModel,
            },
            machine: UnikernelMachineSettings {
                kind: MachineType::Pc,
            },
            interfaces: InterfaceDeviceSettings {
                kind: InterfaceType::Virtio,
                mtu: MTU_STD,
            },
        }),
    )
}
