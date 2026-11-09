#![deny(clippy::unwrap_used, clippy::expect_used)]
#![forbid(unsafe_code)]

mod host;
mod names;
mod network;
mod overlay;
mod qcow2;
mod qemu;
mod size;
mod storage;
mod vm;

pub use host::{
    DomainMemory, HostCpus, HostMemory, NumaCell, host_cpus, host_memory, list_domain_memory,
};
pub use names::{is_device_volume, is_lab_volume};
pub use network::{BridgeNetwork, IsolatedNetwork, NatNetwork, ReservedNetwork};
pub use overlay::create_overlay;
pub use qcow2::{Qcow2Metadata, ensure_no_overlay_dependencies, read_qcow2_metadata};
pub use qemu::{Qemu, QemuConnection};
pub use storage::{SherpaStoragePool, list_lab_volumes, refresh_pool};
pub use vm::{clone_disk, create_vm, delete_disk, get_mgmt_ip, resize_disk};
