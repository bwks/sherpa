#![deny(clippy::unwrap_used, clippy::expect_used)]
#![forbid(unsafe_code)]

mod network;
mod overlay;
mod qcow2;
mod qemu;
mod storage;
mod vm;

pub use network::{BridgeNetwork, IsolatedNetwork, NatNetwork, ReservedNetwork};
pub use overlay::create_overlay;
pub use qcow2::{Qcow2Metadata, ensure_no_overlay_dependencies, read_qcow2_metadata};
pub use qemu::{Qemu, QemuConnection};
pub use storage::{SherpaStoragePool, refresh_pool};
pub use vm::{clone_disk, create_vm, delete_disk, get_mgmt_ip, resize_disk};
