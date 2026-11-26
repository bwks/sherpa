#![deny(clippy::unwrap_used, clippy::expect_used)]
#![forbid(unsafe_code)]

mod console;
mod error;
mod host;
mod interface;
mod names;
mod network;
mod node_vm;
mod overlay;
mod qcow2;
mod qemu;
mod size;
mod storage;
mod vm;
mod vnc;

pub use console::{ConsoleSockets, SerialConsole, move_consoles_to_sockets, serial_console};
pub use error::LibvirtError;
pub use host::{
    DomainMemory, HostCpus, HostMemory, NumaCell, NumaCellValue, host_cpus, host_memory,
    list_domain_memory,
};
pub use interface::{interface_link_up, set_interface_link};
pub use names::{is_device_volume, is_lab_volume};
pub use network::{BridgeNetwork, IsolatedNetwork, NatNetwork, ReservedNetwork};
pub use node_vm::{set_node_vm_cmdline, wait_until_stopped};
pub use overlay::create_overlay;
pub use qcow2::{Qcow2Metadata, read_qcow2_metadata};
pub use qemu::{Qemu, QemuConnection};
pub use storage::{
    BackedVolume, SherpaStoragePool, create_blank_qcow2, list_lab_volumes, overlay_on_image,
    pool_backed_volumes, refresh_pool,
};
pub use vm::{clone_disk, create_vm, delete_disk, get_mgmt_ip, remove_domain, resize_disk};
pub use vnc::{VncConsole, vnc_console};
