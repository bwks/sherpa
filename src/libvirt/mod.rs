mod network;
mod qemu;
mod storage;
mod template;
mod vm;

pub use crate::libvirt::network::{IsolatedNetwork, ManagementNetwork};
pub use crate::libvirt::qemu::Qemu;
pub use crate::libvirt::storage::SherpaStoragePool;
pub use crate::libvirt::template::{
    ArubaAoscxTemplate, CiscoAsavZtpTemplate, CiscoIosxrZtpTemplate, CloudInitTemplate,
    DomainTemplate, JunipervJunosZtpTemplate,
};
pub use crate::libvirt::vm::{clone_disk, create_vm, delete_disk, get_mgmt_ip};
