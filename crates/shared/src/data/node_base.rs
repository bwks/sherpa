//! The shapes the built-in node models are built from.
//!
//! Each model's constructor in `node` starts from one of these bases and states only what differs
//! from it, with struct update syntax. The bases are one level deep: none is built from another,
//! and no model is built from another model, so changing a base changes only the models built
//! from it.

use super::cpu::CpuModels;
use super::disk::DiskBuses;
use super::interface::MgmtInterfaces;
use super::node::{
    BiosTypes, BootDiskSettings, CdromSettings, CpuArchitecture, CpuSettings, InterfaceSettings,
    InterfaceType, MachineSettings, MachineType, NodeConfig, NodeKind, NodeModel, OsVariant,
    UnikernelBootMode, UnikernelSettings, Ztp, ZtpSettings,
};
use crate::konst::{MTU_JUMBO_INT, MTU_STD};

/// A Linux VM provisioned by cloud-init: one vCPU, 1 GiB, a q35 machine with `SeaBIOS`, a virtio
/// boot disk, and one jumbo-MTU virtio data interface after `eth0`.
#[must_use]
pub(super) fn cloud_init_vm(model: NodeModel) -> NodeConfig {
    NodeConfig {
        id: None,
        model,
        version: "0.0.0".to_owned(),
        repo: None,
        os_variant: OsVariant::Linux,
        kind: NodeKind::VirtualMachine,
        default: true,
        memory: 1024,
        cpu: CpuSettings {
            count: 1,
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
        interfaces: InterfaceSettings {
            data_count: 1,
            kind: InterfaceType::Virtio,
            mtu: MTU_JUMBO_INT,
            prefix: "eth".to_owned(),
            first_index: 0,
            management: MgmtInterfaces::Eth0,
            reserved_count: 0,
        },
        ztp: ZtpSettings {
            method: Ztp::CloudInit,
            username: None,
            password: None,
            password_auth: false,
        },
        unikernel: None,
    }
}

/// A network operating system VM, provisioned from a config CD-ROM: one vCPU, a pc machine with
/// `SeaBIOS` and a virtio boot disk. Every such model has its own memory, OS and interface
/// layout, so they are arguments rather than defaults.
#[must_use]
pub(super) fn network_os_vm(
    model: NodeModel,
    memory: u16,
    os_variant: OsVariant,
    interfaces: InterfaceSettings,
) -> NodeConfig {
    NodeConfig {
        id: None,
        model,
        version: "0.0.0".to_owned(),
        repo: None,
        os_variant,
        kind: NodeKind::VirtualMachine,
        default: true,
        memory,
        cpu: CpuSettings {
            count: 1,
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
        ztp: ZtpSettings {
            method: Ztp::Cdrom,
            username: None,
            password: None,
            password_auth: false,
        },
        unikernel: None,
    }
}

/// An application container: one vCPU, 1 GiB, no provisioning, and one macvlan data interface
/// after `eth0`. The machine, boot disk and CD-ROM are the defaults; a container does not use
/// them.
#[must_use]
pub(super) fn app_container(model: NodeModel) -> NodeConfig {
    NodeConfig {
        id: None,
        model,
        version: "0.0.0".to_owned(),
        repo: None,
        os_variant: OsVariant::Unknown,
        kind: NodeKind::Container,
        default: true,
        memory: 1024,
        cpu: CpuSettings {
            count: 1,
            ..CpuSettings::default()
        },
        machine: MachineSettings::default(),
        boot_disk: BootDiskSettings::default(),
        cdrom: CdromSettings::default(),
        interfaces: InterfaceSettings {
            data_count: 1,
            kind: InterfaceType::MacVlan,
            mtu: MTU_STD,
            prefix: "eth".to_owned(),
            first_index: 0,
            management: MgmtInterfaces::Eth0,
            reserved_count: 0,
        },
        ztp: ZtpSettings {
            method: Ztp::Disabled,
            username: None,
            password: None,
            password_auth: false,
        },
        unikernel: None,
    }
}

/// A network operating system container, provisioned from a config volume: one vCPU. Every
/// such model has its own memory, OS and interface layout, so they are arguments rather than
/// defaults. Its machine, boot disk and CD-ROM values are the ones these models have always
/// carried; a container does not use them.
#[must_use]
pub(super) fn network_os_container(
    model: NodeModel,
    memory: u16,
    os_variant: OsVariant,
    interfaces: InterfaceSettings,
) -> NodeConfig {
    NodeConfig {
        id: None,
        model,
        version: "0.0.0".to_owned(),
        repo: None,
        os_variant,
        kind: NodeKind::Container,
        default: true,
        memory,
        cpu: CpuSettings {
            count: 1,
            ..CpuSettings::default()
        },
        machine: MachineSettings {
            kind: MachineType::Q35,
            bios: BiosTypes::SeaBios,
        },
        boot_disk: BootDiskSettings {
            bus: DiskBuses::Sata,
        },
        cdrom: CdromSettings {
            image: None,
            bus: DiskBuses::Sata,
        },
        interfaces,
        ztp: ZtpSettings {
            method: Ztp::Volume,
            username: None,
            password: None,
            password_auth: false,
        },
        unikernel: None,
    }
}

/// A unikernel booted straight from its kernel: one vCPU, 512 MiB, no provisioning and no data
/// interfaces.
#[must_use]
pub(super) fn unikernel(model: NodeModel) -> NodeConfig {
    NodeConfig {
        id: None,
        model,
        version: "0.0.0".to_owned(),
        repo: None,
        os_variant: OsVariant::Unknown,
        kind: NodeKind::Unikernel,
        default: true,
        memory: 512,
        cpu: CpuSettings {
            count: 1,
            ..CpuSettings::default()
        },
        machine: MachineSettings::default(),
        boot_disk: BootDiskSettings::default(),
        cdrom: CdromSettings::default(),
        interfaces: InterfaceSettings {
            data_count: 0,
            kind: InterfaceType::Virtio,
            mtu: MTU_STD,
            prefix: "eth".to_owned(),
            first_index: 0,
            management: MgmtInterfaces::Eth0,
            reserved_count: 0,
        },
        ztp: ZtpSettings {
            method: Ztp::Disabled,
            username: None,
            password: None,
            password_auth: false,
        },
        unikernel: Some(UnikernelSettings {
            boot_mode: UnikernelBootMode::DirectKernel,
        }),
    }
}
