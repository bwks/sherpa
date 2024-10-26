mod network;
mod qemu;
mod storage;
mod template;
mod vm;

pub use crate::libvirt::network::{create_isolated_network, create_network};
pub use crate::libvirt::qemu::Qemu;
pub use crate::libvirt::storage::create_sherpa_storage_pool;
pub use crate::libvirt::template::{
    AristaVeosZtpTemplate, ArubaAoscxTemplate, CiscoAsavZtpTemplate, CiscoIosXeZtpTemplate,
    CiscoIosvZtpTemplate, CiscoIosxrZtpTemplate, CiscoNxosZtpTemplate, CloudInitTemplate,
    CumulusLinuxZtpTemplate, DomainTemplate,
};
pub use crate::libvirt::vm::{clone_disk, create_vm, delete_disk, get_mgmt_ip};
