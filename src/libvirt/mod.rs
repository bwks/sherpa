mod network;
mod qemu;
mod template;
mod vm;

pub use crate::libvirt::network::{create_isolated_network, create_network};
pub use crate::libvirt::qemu::Qemu;
pub use crate::libvirt::template::{
    AristaVeosZtpTemplate, CiscoIosXeZtpTemplate, CiscoIosvZtpTemplate, CloudInitTemplate,
    CumulusLinuxZtpTemplate, DomainTemplate, FlatcarIgnitionTemplate,
};
pub use crate::libvirt::vm::{clone_disk, create_vm, delete_disk, get_mgmt_ip};
