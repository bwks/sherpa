use rinja::Template;

use crate::data::{
    BiosTypes, ConnectionTypes, CpuArchitecture, DeviceDisk, DiskBuses, DiskDevices, Interface,
    InterfaceTypes, MachineTypes,
};

#[derive(Debug, Template)]
#[template(path = "libvirt/libvirt_domain.jinja", ext = "xml")]
pub struct DomainTemplate {
    pub name: String,
    pub memory: u16,
    pub cpu_architecture: CpuArchitecture,
    pub machine_type: MachineTypes,
    pub cpu_count: u8,
    pub vmx_enabled: bool,
    pub qemu_bin: String,
    pub bios: BiosTypes,
    pub disks: Vec<DeviceDisk>,
    pub ignition_config: Option<bool>,
    pub interfaces: Vec<Interface>,
    pub interface_type: InterfaceTypes,
    pub loopback_ipv4: String,
    pub telnet_port: u16,
}
