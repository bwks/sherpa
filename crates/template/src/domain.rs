use askama::Template;

use data::{
    BiosTypes, CloneDisk, ConnectionTypes, CpuArchitecture, CpuModels, DiskBuses, DiskDevices,
    Interface, InterfaceType, MachineType, NodeDisk, QemuCommand,
};

#[derive(Debug, Template)]
#[template(path = "libvirt/libvirt_domain.jinja", ext = "xml", escape = "xml")]
pub struct DomainTemplate {
    pub name: String,
    pub memory: u16,
    pub cpu_architecture: CpuArchitecture,
    pub cpu_model: CpuModels,
    pub machine_type: MachineType,
    pub cpu_count: u8,
    pub vmx_enabled: bool,
    pub qemu_bin: String,
    pub bios: BiosTypes,
    pub disks: Vec<NodeDisk>,
    pub interfaces: Vec<Interface>,
    pub interface_type: InterfaceType,
    pub loopback_ipv4: String,
    pub telnet_port: u16,
    pub qemu_commands: Vec<QemuCommand>,
    pub lab_id: String,
    pub isolated_network: String,
}

pub struct BootServer {
    pub template: DomainTemplate,
    pub copy_disks: Vec<CloneDisk>,
}
